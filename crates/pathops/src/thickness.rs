//! The thickness of a filled shape: whether a circle of a given width fits anywhere inside its
//! fill (Clean Up's and Select › Object's Hairline Slivers).

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape as _};
use vectorcraft_geom::{FillRule, PathData};

use crate::boolean::{eps_for, fill_bezpath, normalize_bez};

/// Most line pieces the outline of one shape is measured with.
pub const MAX_THICKNESS_PIECES: usize = 100_000;
/// Most cells one search for a wide enough spot looks at.
const MAX_CELLS: usize = 500_000;
/// Most distances from a point to a piece of outline one search measures.
const MAX_WORK: usize = 50_000_000;
/// Most distances from a point to a piece of outline the shapes of one document are measured
/// with, all together ([`ThicknessBudget::document`]).
pub const MAX_DOCUMENT_THICKNESS_WORK: usize = 1_000_000_000;
/// The resolved outline is measured as line pieces within `width / FINE_PARTS` of its curves.
const FINE_PARTS: f64 = 32.0;
/// The first, rough look measures the path as drawn with pieces within `width / ROUGH_PARTS`.
const ROUGH_PARTS: f64 = 8.0;

/// Is the fill of `path` (every subpath closed, as a fill paints it) under `rule` thinner than
/// `width` everywhere: does no circle of diameter `width` fit inside it? A path with no fill is.
/// A shape within a small fraction of `width` of the limit is taken as thick enough.
///
/// `None` when it can't be told: `width` isn't a finite positive number, a coordinate isn't
/// finite, the fill can't be resolved into outlines (or only so coarsely, for a shape that large,
/// that edges closer than `width` would merge), or measuring it would take more than
/// [`MAX_THICKNESS_PIECES`] pieces of outline or a search too long.
pub fn thinner_than(path: &PathData, rule: FillRule, width: f64) -> Option<bool> {
    thinner_than_within(path, rule, width, &mut ThicknessBudget::new(MAX_WORK))
}

/// The work left for measuring shapes with [`thinner_than_within`]: a count of distances from a
/// point to a piece of outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThicknessBudget(usize);

impl ThicknessBudget {
    pub fn new(work: usize) -> Self {
        Self(work)
    }
    /// The work all the shapes of one document may take: [`MAX_DOCUMENT_THICKNESS_WORK`].
    pub fn document() -> Self {
        Self(MAX_DOCUMENT_THICKNESS_WORK)
    }
    /// The work left.
    pub fn left(&self) -> usize {
        self.0
    }
}

/// [`thinner_than`], taking the search's work from `budget`: `None` as well when the search needs
/// more than is left. The quick checks take nothing from it, so they still settle shapes once it
/// runs out.
pub fn thinner_than_within(path: &PathData, rule: FillRule, width: f64, budget: &mut ThicknessBudget) -> Option<bool> {
    let finite = |p: Point| p.x.is_finite() && p.y.is_finite();
    if !(width.is_finite() && width > 0.0)
        || !path.subpaths.iter().flat_map(|sp| &sp.anchors).all(|a| finite(a.p) && finite(a.h_in) && finite(a.h_out))
    {
        return None;
    }
    // Every segment is at least one piece of outline, so a path of more anchors is too long to
    // measure before its outline is built.
    if path.subpaths.iter().map(|sp| sp.anchors.len()).sum::<usize>() > MAX_THICKNESS_PIECES {
        return None;
    }
    let mut bp = fill_bezpath(path);
    if bp.elements().is_empty() {
        return Some(true);
    }
    // Measured from the corner of its bounds, so that a shape far from the origin measures as it
    // does near it.
    let corner = bp.bounding_box().origin();
    bp.apply_affine(Affine::translate(-corner.to_vec2()));
    let b = bp.bounding_box();
    let (r, fine) = (width / 2.0, width / FINE_PARTS);
    // A circle inside the fill is inside its bounds. Bounds within the fine measure's error of the
    // width go on to that measure, which keeps them.
    if b.width().min(b.height()) < width - 2.0 * fine {
        return Some(true);
    }
    // Every point of a fill lies within the widest circle's radius r of its outline, and the points
    // within r of a closed outline of length L cover at most 2rL + πr². A fill larger than that holds
    // a circle of diameter `width`. Only one subpath filled nonzero that winds at most once around
    // any point is measured so: the areas of several subpaths add up to more than their fill where
    // they overlap, a path that runs around a spot twice counts its area there twice, and an
    // even-odd fill of a path that runs around twice is empty.
    if path.subpaths.len() == 1
        && rule == FillRule::NonZero
        && winds_at_most_once(&bp)
        && bp.area().abs() > 2.0 * r * bp.perimeter(1e-3) * 1.01 + std::f64::consts::PI * r * r
    {
        return Some(false);
    }
    // The circle lies between any two parallel lines that hold the path's points and handles too,
    // which settles a long sliver lying on a slant.
    if hull_width(&bp) < width - 2.0 * fine {
        return Some(true);
    }
    // A rough look at the path as drawn: an edge running inside the fill is nearer than the fill's
    // outline, so a spot this far from every edge (and from the rough pieces' error) is far enough.
    let rough = width / ROUGH_PARTS;
    let mut edges = vec![];
    if !flatten_into(&bp, rough, &mut edges) {
        return None;
    }
    match search(&edges, b, rule, r + rough, rough, budget) {
        Search::Found => return Some(false),
        // Resolving the outlines takes time of its own, so it isn't started when what is left of
        // the budget cut the rough look short. The rough look can run out where the resolved
        // outline is much shorter, so one that ran out of its own share goes on.
        Search::TooLong if budget.0 < MAX_WORK => return None,
        _ => {}
    }
    // The fill's own outlines: where subpaths overlap or a path crosses itself, the edges inside the
    // fill are no outline of it. A spot within the fine pieces' error of the radius counts as one.
    // The sweep snaps points within its tolerance together, which grows with the shape's size: one
    // as coarse as the fine pieces could close up a fill that holds the circle, so it isn't measured.
    if eps_for([&bp]).ok()? >= fine {
        return None;
    }
    let contours = normalize_bez(&bp, rule).ok()?;
    edges.clear();
    for c in contours.contours() {
        if !flatten_into(&c.path, fine, &mut edges) {
            return None;
        }
    }
    if edges.is_empty() {
        return Some(true);
    }
    match search(&edges, b, FillRule::EvenOdd, r - fine, fine, budget) {
        Search::Found | Search::Unsure => Some(false),
        Search::Absent => Some(true),
        Search::TooLong => None,
    }
}

/// The points and handles of `bp`, in order.
fn control_points(bp: &BezPath) -> Vec<Point> {
    bp.elements()
        .iter()
        .flat_map(|el| match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => vec![p],
            PathEl::QuadTo(a, p) => vec![a, p],
            PathEl::CurveTo(a, b, p) => vec![a, b, p],
            PathEl::ClosePath => vec![],
        })
        .collect()
}

/// Does `bp`, one closed subpath, wind at most once around every point? Every line through a point
/// that a curve winds around twice crosses it at least four times, so such a curve turns through
/// at least 4π in all. A Bézier curve turns no more than its control polygon, so a control polygon
/// that turns through less than 4π winds at most once.
fn winds_at_most_once(bp: &BezPath) -> bool {
    let mut pts = control_points(bp);
    pts.dedup();
    while pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    if pts.len() < 3 {
        return true;
    }
    let turned: f64 = pts
        .iter()
        .zip(pts.iter().cycle().skip(1))
        .zip(pts.iter().cycle().skip(2))
        .map(|((&a, &b), &c)| {
            let (u, v) = (b - a, c - b);
            u.cross(v).atan2(u.dot(v)).abs()
        })
        .sum();
    turned < 4.0 * std::f64::consts::PI * (1.0 - 1e-9)
}

/// Most corners of a convex hull [`hull_width`] measures; a larger hull measures as unbounded.
const MAX_HULL: usize = 512;

/// The least distance between two parallel lines that hold every point and handle of `bp` (the
/// width of their convex hull, which holds the curves too).
fn hull_width(bp: &BezPath) -> f64 {
    let mut pts = control_points(bp);
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup();
    if pts.len() < 3 {
        return 0.0;
    }
    // Andrew's monotone chain: the lower hull, then the upper.
    let turn = |o: Point, a: Point, b: Point| (a - o).cross(b - o);
    let mut hull: Vec<Point> = Vec::with_capacity(pts.len() + 1);
    for pass in [false, true] {
        let floor = hull.len();
        let ordered: Box<dyn Iterator<Item = &Point>> = if pass { Box::new(pts.iter().rev()) } else { Box::new(pts.iter()) };
        for &p in ordered {
            while hull.len() >= floor + 2 && hull.get(hull.len() - 2).zip(hull.last()).is_some_and(|(&o, &a)| turn(o, a, p) <= 0.0) {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    if hull.len() < 3 {
        return 0.0;
    }
    if hull.len() > MAX_HULL {
        return f64::INFINITY;
    }
    // For each side, the farthest corner from its line; the least of those is the width.
    let mut least = f64::INFINITY;
    for (&a, &b) in hull.iter().zip(hull.iter().cycle().skip(1)) {
        let side = b - a;
        let len = side.hypot();
        if len > 0.0 {
            let far = hull.iter().map(|&p| side.cross(p - a).abs() / len).fold(0.0, f64::max);
            least = least.min(far);
        }
    }
    least
}

/// Add `bp`'s outline to `edges` as line pieces within `tolerance` of it: false when that takes
/// more than [`MAX_THICKNESS_PIECES`], or when a curve is too long for [`vectorcraft_geom::flatten`]
/// to keep within `tolerance`.
fn flatten_into(bp: &BezPath, tolerance: f64, edges: &mut Vec<(Point, Point)>) -> bool {
    let push = |from: Point, to: Point, edges: &mut Vec<(Point, Point)>| {
        if from != to {
            edges.push((from, to));
        }
    };
    let (mut start, mut last) = (Point::ZERO, Point::ZERO);
    let most = vectorcraft_geom::MAX_FLATTEN_PER_CURVE * vectorcraft_geom::MAX_FLATTEN_PER_CURVE;
    for el in bp.iter() {
        if edges.len() > MAX_THICKNESS_PIECES {
            return false;
        }
        match el {
            // An outline that doesn't end where it starts is closed with a line, as a fill paints it.
            PathEl::MoveTo(p) => {
                push(last, start, edges);
                (start, last) = (p, p);
            }
            PathEl::ClosePath => {
                push(last, start, edges);
                last = start;
            }
            PathEl::LineTo(p) => {
                push(last, p, edges);
                last = p;
            }
            PathEl::QuadTo(a, p) | PathEl::CurveTo(a, _, p) => {
                let length = match el {
                    PathEl::CurveTo(_, b, _) => last.distance(a) + a.distance(b) + b.distance(p),
                    _ => last.distance(a) + a.distance(p),
                };
                if length / most > tolerance {
                    return false;
                }
                let mut from = last;
                vectorcraft_geom::flatten([PathEl::MoveTo(last), el], tolerance, |piece| {
                    if let PathEl::LineTo(to) = piece {
                        push(from, to, edges);
                        from = to;
                    }
                });
                last = p;
            }
        }
    }
    push(last, start, edges);
    edges.len() <= MAX_THICKNESS_PIECES
}

/// The distance from `p` to the nearest of `edges`, negative outside the region they bound under
/// `rule`.
fn signed_distance(edges: &[(Point, Point)], rule: FillRule, p: Point) -> f64 {
    let mut winding = 0i32;
    let mut nearest = f64::INFINITY;
    for &(a, b) in edges {
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            winding += if b.y > a.y { 1 } else { -1 };
        }
        let ab = b - a;
        let len2 = ab.hypot2();
        let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
        nearest = nearest.min((p - (a + ab * t)).hypot2());
    }
    let inside = match rule {
        FillRule::NonZero => winding != 0,
        FillRule::EvenOdd => winding % 2 != 0,
    };
    let d = nearest.sqrt();
    if inside { d } else { -d }
}

/// A square cell of the search: its center, half its side, the signed distance from its center to
/// the outline, and the most that distance can be anywhere in it.
struct Cell {
    c: Point,
    h: f64,
    d: f64,
    most: f64,
}

impl PartialEq for Cell {
    fn eq(&self, other: &Self) -> bool {
        self.most.total_cmp(&other.most) == Ordering::Equal
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
        self.most.total_cmp(&other.most)
    }
}

/// What [`search`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Search {
    /// A point at least the distance asked for from the outline.
    Found,
    /// No such point.
    Absent,
    /// None found, but cells smaller than the finest size could still hold one.
    Unsure,
    /// The search needed more cells or distances than it may take.
    TooLong,
}

/// Look for a point inside the region `edges` bound under `rule` (within `bounds`) at least `goal`
/// from its outline. The bounds are cut into square cells, and the cell that could hold the farthest
/// point is split first, until a cell's center is `goal` away or no cell could be. The distance
/// changes no faster than the point moves, so a cell holds no point farther than its center's
/// distance plus half its diagonal. Cells smaller than `finest` are not split further. The
/// distances it measures, no more than [`MAX_WORK`], are taken from `budget`.
fn search(edges: &[(Point, Point)], bounds: Rect, rule: FillRule, goal: f64, finest: f64, budget: &mut ThicknessBudget) -> Search {
    let mut work = 0usize;
    let found = search_up_to(edges, bounds, rule, (goal, finest), budget.0.min(MAX_WORK), &mut work);
    budget.0 = budget.0.saturating_sub(work);
    found
}

/// [`search`], measuring no more than `limit` distances and counting them in `work`.
fn search_up_to(edges: &[(Point, Point)], bounds: Rect, rule: FillRule, (goal, finest): (f64, f64), limit: usize, work: &mut usize) -> Search {
    let side = bounds.width().min(bounds.height());
    if side.is_nan() || side <= 0.0 {
        return Search::Absent;
    }
    let (cols, rows) = ((bounds.width() / side).ceil(), (bounds.height() / side).ceil());
    if cols * rows > MAX_CELLS as f64 || cols * rows * edges.len() as f64 > limit as f64 {
        return Search::TooLong;
    }
    let cell = |c: Point, h: f64, work: &mut usize| {
        *work += edges.len();
        let d = signed_distance(edges, rule, c);
        Cell { c, h, d, most: d + h * std::f64::consts::SQRT_2 }
    };
    let mut heap = BinaryHeap::new();
    let h = side / 2.0;
    for i in 0..cols as usize {
        for j in 0..rows as usize {
            heap.push(cell(Point::new(bounds.x0 + (i as f64 + 0.5) * side, bounds.y0 + (j as f64 + 0.5) * side), h, work));
        }
    }
    let (mut made, mut unsure) = (heap.len(), false);
    while let Some(next) = heap.pop() {
        if next.d >= goal {
            return Search::Found;
        }
        if next.most < goal {
            break;
        }
        if next.h < finest / 2.0 {
            unsure = true;
            continue;
        }
        if made + 4 > MAX_CELLS || work.saturating_add(4 * edges.len()) > limit {
            return Search::TooLong;
        }
        let h = next.h / 2.0;
        for (dx, dy) in [(-h, -h), (h, -h), (-h, h), (h, h)] {
            let part = cell(Point::new(next.c.x + dx, next.c.y + dy), h, work);
            // A cell that can't hold a point `goal` away is left out.
            if part.most >= goal {
                heap.push(part);
            }
        }
        made += 4;
    }
    if unsure { Search::Unsure } else { Search::Absent }
}

#[cfg(test)]
mod tests {
    use vectorcraft_geom::{Anchor, SubPath};

    use super::*;

    fn polygon(points: &[(f64, f64)]) -> SubPath {
        SubPath::new(points.iter().map(|&(x, y)| Anchor::corner(Point::new(x, y))).collect(), true)
    }

    fn one(points: &[(f64, f64)]) -> PathData {
        PathData::single(polygon(points))
    }

    fn thin(p: &PathData, rule: FillRule, width: f64) -> bool {
        thinner_than(p, rule, width) == Some(true)
    }

    fn thick(p: &PathData, rule: FillRule, width: f64) -> bool {
        thinner_than(p, rule, width) == Some(false)
    }

    #[test]
    fn a_triangle_whose_third_anchor_almost_meets_its_first_is_thinner_than_a_hairline() {
        let sliver = one(&[(0.0, 0.0), (40.0, 30.0), (0.001, 0.0015)]);
        assert!(thin(&sliver, FillRule::NonZero, 0.085));
        // Long and on a slant at the narrowest width, and far from the origin.
        assert!(thin(&one(&[(0.0, 0.0), (600.0, 600.0), (0.0, 0.0001)]), FillRule::NonZero, 0.001));
        let (x, y) = (1_000_000.0, 1_000_000.0);
        assert!(thin(&one(&[(x, y), (x + 0.01, y + 0.0075), (x + 0.0001, y + 0.0002)]), FillRule::NonZero, 0.001));
    }

    #[test]
    fn a_bent_sliver_is_found_by_measuring_inside_it() {
        // Out along two sides of a triangle and back 0.002 pt inside them: its hull and bounds are
        // wide, and only the measure inside finds it thin.
        let v = one(&[(0.0, 0.0), (40.0, 30.0), (80.0, 0.0), (79.997, 0.0), (40.0, 29.998), (0.003, 0.0)]);
        assert!(thin(&v, FillRule::NonZero, 0.085));
    }

    #[test]
    fn a_narrow_strip_wider_than_the_width_is_kept_and_one_narrower_is_not() {
        // A diagonal strip 100 pt long.
        let strip = |w: f64| {
            let (dx, dy) = (0.6 * w, -0.8 * w);
            one(&[(0.0, 0.0), (80.0, 60.0), (80.0 + dx, 60.0 + dy), (dx, dy)])
        };
        assert!(thick(&strip(0.2), FillRule::NonZero, 0.085));
        assert!(thick(&strip(0.2), FillRule::NonZero, 0.18));
        assert!(thin(&strip(0.05), FillRule::NonZero, 0.085));
        assert!(thin(&strip(0.2), FillRule::NonZero, 0.24));
    }

    #[test]
    fn a_triangle_holding_the_circle_off_every_cell_center_is_kept() {
        // Its inscribed circle is 2 across, centered at (1, 1).
        let t = one(&[(0.0, 0.0), (4.0, 0.0), (0.0, 3.0)]);
        assert!(thick(&t, FillRule::NonZero, 1.95));
        assert!(thin(&t, FillRule::NonZero, 2.3));
    }

    #[test]
    fn a_tiny_round_dot_is_kept() {
        let dot = vectorcraft_geom::shapes::ellipse(kurbo::Rect::new(10.0, 10.0, 10.3, 10.3));
        assert!(thick(&dot, FillRule::NonZero, 0.085));
        assert!(thick(&dot, FillRule::NonZero, 0.29));
        assert!(thin(&dot, FillRule::NonZero, 0.4));
    }

    #[test]
    fn a_thin_ring_is_thin_whichever_way_its_subpaths_run() {
        let square = |x0: f64, x1: f64| polygon(&[(x0, x0), (x1, x0), (x1, x1), (x0, x1)]);
        let mut hole = square(1.01, 98.99);
        hole.reverse();
        assert!(thin(&PathData::new(vec![square(1.0, 99.0), hole]), FillRule::NonZero, 0.085));
        // Even-odd: an inner square running the same way still cuts a hole.
        assert!(thin(&PathData::new(vec![square(1.0, 99.0), square(1.01, 98.99)]), FillRule::EvenOdd, 0.085));
        // Nonzero: it doesn't, and the edges inside the fill are no outline of it.
        assert!(thick(&PathData::new(vec![square(1.0, 99.0), square(1.01, 98.99)]), FillRule::NonZero, 0.085));
    }

    #[test]
    fn overlapping_strips_are_measured_by_their_union() {
        // Two strips 0.2 pt tall overlapping by half: their fill is 0.3 pt tall, while the edges of
        // each that run inside the other are no more than 0.1 pt apart.
        let strip = |y: f64| polygon(&[(0.0, y), (100.0, y), (100.0, y + 0.2), (0.0, y + 0.2)]);
        let strips = PathData::new(vec![strip(0.0), strip(0.1)]);
        assert!(thick(&strips, FillRule::NonZero, 0.15));
        assert!(thin(&strips, FillRule::NonZero, 0.35));
    }

    #[test]
    fn an_outline_out_and_back_along_itself_has_no_fill_and_is_thin() {
        assert!(thin(&one(&[(0.0, 0.0), (30.0, 40.0), (0.0, 0.0), (30.0, 40.0)]), FillRule::NonZero, 0.085));
        assert!(thin(&PathData::default(), FillRule::NonZero, 0.085));
    }

    /// Shapes thicker than the width by their area, and slivers lying flat along an axis, are settled
    /// without resolving their outlines, so a document of thousands of them is measured quickly.
    #[test]
    fn plain_shapes_and_flat_slivers_are_settled_without_resolving_their_outlines() {
        let before = crate::boolean::NORMALIZE_SWEEPS.with(|n| n.get());
        for k in 0..2000 {
            let x = f64::from(k) * 12.0;
            assert!(thick(&one(&[(x, 0.0), (x + 10.0, 0.0), (x + 10.0, 10.0), (x, 10.0)]), FillRule::NonZero, 0.085));
            assert!(thin(&one(&[(x, 20.0), (x + 10.0, 20.0), (x + 10.0, 20.01), (x, 20.01)]), FillRule::NonZero, 0.085));
            // Long slivers on a slant are settled by the width of their hull.
            assert!(thin(&one(&[(x, 40.0), (x + 9.0, 52.0), (x + 0.001, 40.0015)]), FillRule::NonZero, 0.085));
        }
        assert_eq!(crate::boolean::NORMALIZE_SWEEPS.with(|n| n.get()), before);
    }

    /// The area measure is used only for an outline that runs around once: one that runs around
    /// twice counts its area twice.
    #[test]
    fn an_outline_that_runs_around_twice_is_not_measured_by_its_area() {
        let square = |n: usize| {
            let mut bp = BezPath::new();
            bp.move_to((0.0, 0.0));
            for _ in 0..n {
                for p in [(10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 0.0)] {
                    bp.line_to(p);
                }
            }
            bp.close_path();
            bp
        };
        assert!(winds_at_most_once(&square(1)));
        assert!(!winds_at_most_once(&square(2)));
        // Measured inside instead, a square run around twice is still as thick as one.
        let twice = one(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        assert!(thick(&twice, FillRule::NonZero, 0.085));
    }

    /// Shapes that need the search share one budget: once it is spent they can't be measured, and
    /// the quick checks still settle the rest.
    #[test]
    fn a_spent_budget_leaves_the_shapes_that_need_a_search_unmeasured() {
        let bent = one(&[(0.0, 0.0), (40.0, 30.0), (80.0, 0.0), (79.997, 0.0), (40.0, 29.998), (0.003, 0.0)]);
        let square = one(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        let mut budget = ThicknessBudget::document();
        assert_eq!(thinner_than_within(&bent, FillRule::NonZero, 0.085, &mut budget), Some(true));
        assert!(budget.left() < MAX_DOCUMENT_THICKNESS_WORK);
        let mut spent = ThicknessBudget::new(0);
        assert_eq!(thinner_than_within(&bent, FillRule::NonZero, 0.085, &mut spent), None);
        assert_eq!(thinner_than_within(&square, FillRule::NonZero, 0.085, &mut spent), Some(false));
    }

    /// A shape so long that resolving its outline snaps edges closer than the width together is not
    /// measured, while the same height at an ordinary length is.
    #[test]
    fn a_shape_too_long_to_resolve_at_the_width_cannot_be_measured() {
        let rect = |w: f64| one(&[(0.0, 0.0), (w, 0.0), (w, 0.0011), (0.0, 0.0011)]);
        assert_eq!(thinner_than(&rect(1e12), FillRule::NonZero, 0.001), None);
        assert_eq!(thinner_than(&rect(1.0), FillRule::NonZero, 0.001), Some(false));
        assert_eq!(thinner_than(&rect(1.0), FillRule::NonZero, 0.0012), Some(true));
    }

    #[test]
    fn a_width_or_a_coordinate_that_is_no_finite_number_cannot_be_measured() {
        let sliver = one(&[(0.0, 0.0), (40.0, 30.0), (0.001, 0.0015)]);
        for w in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(thinner_than(&sliver, FillRule::NonZero, w), None, "{w}");
        }
        let flat = one(&[(0.0, 0.0), (f64::NAN, 0.0), (40.0, 0.0)]);
        assert_eq!(thinner_than(&flat, FillRule::NonZero, 0.085), None);
    }
}
