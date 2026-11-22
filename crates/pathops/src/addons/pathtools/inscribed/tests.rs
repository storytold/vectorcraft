use kurbo::{Point, Rect};
use vectorcraft_geom::hit::fill_contains;
use vectorcraft_geom::{Anchor, FillRule, PathData, SubPath, shapes};

use super::*;

fn polygon(pts: &[(f64, f64)]) -> PathData {
    PathData::new(vec![SubPath::new(pts.iter().map(|&(x, y)| Anchor::corner(Point::new(x, y))).collect(), true)])
}

fn circle(x: f64, y: f64, r: f64) -> SubPath {
    shapes::ellipse(Rect::new(x - r, y - r, x + r, y + r)).subpaths.remove(0)
}

/// Points around the circle `c`, a little inside its edge.
fn rim(c: Inscribed) -> impl Iterator<Item = Point> {
    (0..360).map(move |i| {
        let a = (i as f64).to_radians();
        c.center + kurbo::Vec2::new(a.cos(), a.sin()) * (c.radius - 1e-6)
    })
}

/// The circle lies in the filled area and is within twice the precision of `largest`, which may
/// be `slack` too small: a circle of four curves bulges up to 0.03% past a true circle.
fn holds_within(path: &PathData, rule: FillRule, c: Inscribed, largest: f64, slack: f64) {
    let bp = path.to_bezpath();
    assert!(rim(c).all(|p| fill_contains(&bp, rule, p)), "{c:?} reaches outside the fill");
    assert!(c.radius <= largest + slack + 1e-6 && c.radius >= largest - 2.0 * INSCRIBED_PRECISION, "{c:?}, largest {largest}");
}

fn holds(path: &PathData, rule: FillRule, c: Inscribed, largest: f64) {
    holds_within(path, rule, c, largest, 0.0);
}

#[test]
fn a_square_holds_the_circle_at_its_center() {
    let sq = polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    let c = inscribed_circle(&sq, FillRule::NonZero).unwrap();
    holds(&sq, FillRule::NonZero, c, 50.0);
    assert!((c.center - Point::new(50.0, 50.0)).hypot() < 2.0 * INSCRIBED_PRECISION, "{c:?}");
}

#[test]
fn a_concave_shape_holds_the_circle_in_its_corner() {
    // An L whose arms are 30 wide: the box's center (50, 50) is outside it. The largest circle
    // touches the two outer edges and the inner corner (30, 30): c = √2 (30 − c).
    let l = polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 30.0), (30.0, 30.0), (30.0, 100.0), (0.0, 100.0)]);
    let c = inscribed_circle(&l, FillRule::NonZero).unwrap();
    let largest = 30.0 * std::f64::consts::SQRT_2 / (1.0 + std::f64::consts::SQRT_2);
    holds(&l, FillRule::NonZero, c, largest);
}

#[test]
fn a_ring_holds_the_circle_beside_its_hole_under_even_odd() {
    let ring = PathData::new(vec![circle(0.0, 0.0, 100.0), circle(0.0, 0.0, 40.0)]);
    let c = inscribed_circle(&ring, FillRule::EvenOdd).unwrap();
    holds_within(&ring, FillRule::EvenOdd, c, 30.0, 0.0003 * 140.0);
    assert!((c.center.to_vec2().hypot() - 70.0).abs() < 0.1, "{c:?}");
    // Both subpaths turn the same way, so under nonzero the inner one fills in and is no hole.
    let c = inscribed_circle(&ring, FillRule::NonZero).unwrap();
    holds_within(&ring, FillRule::NonZero, c, 100.0, 0.0003 * 100.0);
}

#[test]
fn overlapping_subpaths_count_as_the_area_they_fill() {
    // Two squares overlapping by half: under nonzero they fill one 150 × 100 rectangle, and the
    // edges inside it do not bound the circle.
    let a = polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    let b = polygon(&[(50.0, 0.0), (150.0, 0.0), (150.0, 100.0), (50.0, 100.0)]);
    let both = PathData::new(a.subpaths.into_iter().chain(b.subpaths).collect());
    let c = inscribed_circle(&both, FillRule::NonZero).unwrap();
    holds(&both, FillRule::NonZero, c, 50.0);
}

#[test]
fn a_small_shape_is_measured_to_its_own_scale() {
    // The L of the concave test, a thousandth of its size.
    let l = polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 30.0), (30.0, 30.0), (30.0, 100.0), (0.0, 100.0)].map(|(x, y)| (x / 1000.0, y / 1000.0)));
    let c = inscribed_circle(&l, FillRule::NonZero).unwrap();
    let largest = 0.03 * std::f64::consts::SQRT_2 / (1.0 + std::f64::consts::SQRT_2);
    assert!(c.radius <= largest + 1e-9 && c.radius > largest * 0.99, "{c:?}, largest {largest}");
}

#[test]
fn a_shape_narrower_than_the_default_precision_gets_a_circle() {
    // 0.005 pt square: resolved at 0.01 pt it would be dropped as a sliver.
    let sq = polygon(&[(0.0, 0.0), (0.005, 0.0), (0.005, 0.005), (0.0, 0.005)]);
    let c = inscribed_circle(&sq, FillRule::NonZero).unwrap();
    assert!(c.radius <= 0.0025 + 1e-12 && c.radius > 0.0025 * 0.99, "{c:?}");
}

#[test]
fn a_tiny_shape_far_from_the_origin_gets_a_circle() {
    let sq = polygon(&[(10_000.0, 10_000.0), (10_000.000_1, 10_000.0), (10_000.000_1, 10_000.000_1), (10_000.0, 10_000.000_1)]);
    let c = inscribed_circle(&sq, FillRule::NonZero).unwrap();
    assert!(c.radius > 0.00005 * 0.99 && c.radius <= 0.00005 + 1e-9, "{c:?}");
    // With a second tiny square far from the first, each square's area is measured from its own
    // corner.
    let near = polygon(&[(0.0, 0.0), (0.0001, 0.0), (0.0001, 0.0001), (0.0, 0.0001)]);
    let far = polygon(&[(1e6, 1e6), (1e6 + 0.0001, 1e6), (1e6 + 0.0001, 1e6 + 0.0001), (1e6, 1e6 + 0.0001)]);
    let both = PathData::new(near.subpaths.into_iter().chain(far.subpaths).collect());
    let a = area(&edges(&both.to_bezpath(), 0.005).unwrap());
    assert!((a - 2e-8).abs() < 1e-10, "{a}");
}

#[test]
fn a_hairline_hole_bounds_the_circle() {
    // A 100 pt square with a hole 0.001 pt wide down its middle: the circle stays on one side.
    let sq = polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    let slit = polygon(&[(49.9995, 10.0), (50.0005, 10.0), (50.0005, 90.0), (49.9995, 90.0)]);
    let both = PathData::new(sq.subpaths.into_iter().chain(slit.subpaths).collect());
    let c = inscribed_circle(&both, FillRule::EvenOdd).unwrap();
    assert!((c.center.x - 50.0).abs() >= c.radius - 2.0 * INSCRIBED_PRECISION, "{c:?}");
    assert!(c.radius < 30.0, "{c:?}");
}

#[test]
fn a_long_thin_shape_is_refused_without_measuring_past_the_work_limit() {
    // A strip 5,000 long, 1 high at one end and 2 at the other, whose top edge has 5,000 pieces:
    // cut into cells as wide as it is high, the first cells alone would measure 2,500 times every
    // edge.
    let n = 5000;
    let top: Vec<(f64, f64)> = (0..=n).map(|i| (i as f64, 0.0)).collect();
    let pts: Vec<(f64, f64)> = top.into_iter().chain([(n as f64, 1.0), (0.0, 2.0)]).collect();
    let e = edges(&polygon(&pts).to_bezpath(), 0.005).unwrap();
    let limit = 100 * e.len();
    assert_eq!(search(&e, Rect::new(0.0, 0.0, n as f64, 2.0), 0.01, &mut limit.clone()), Err(InscribedError::TooComplex));
    let measured = MEASURED.with(std::cell::Cell::get);
    assert!(measured <= limit, "{measured} measured, limit {limit}");
}

#[test]
fn a_long_rectangle_stops_once_no_wider_circle_fits() {
    // Every point of the center line is 50 from the long sides: the box's center is already the
    // largest circle's, and the search stops at the first cells.
    let r = polygon(&[(0.0, 0.0), (100_000.0, 0.0), (100_000.0, 100.0), (0.0, 100.0)]);
    let c = inscribed_circle(&r, FillRule::NonZero).unwrap();
    holds(&r, FillRule::NonZero, c, 50.0);
    let measured = MEASURED.with(std::cell::Cell::get);
    assert!(measured <= (MAX_FIRST_CELLS as usize + 1) * 4, "{measured} measured");
}

#[test]
fn a_fill_far_smaller_than_the_precision_still_gets_a_circle() {
    // Two 0.001 pt squares at opposite corners of a 100 pt box, which sets the precision to
    // 0.01 pt.
    let a = polygon(&[(0.0, 0.0), (0.001, 0.0), (0.001, 0.001), (0.0, 0.001)]);
    let b = polygon(&[(99.999, 99.999), (100.0, 99.999), (100.0, 100.0), (99.999, 100.0)]);
    let both = PathData::new(a.subpaths.into_iter().chain(b.subpaths).collect());
    let c = inscribed_circle(&both, FillRule::NonZero).unwrap();
    assert!(c.radius > 0.0 && c.radius <= 0.0005 + 1e-9, "{c:?}");
    assert!(fill_contains(&both.to_bezpath(), FillRule::NonZero, c.center), "{c:?}");
}

#[test]
fn a_lens_thinner_than_the_flattening_tolerance_gets_a_circle() {
    // A straight side and a curve bowed 0.001 pt from it, 141 pt long: flattened to 0.005 pt the
    // curve would lie on the straight side.
    let lens = |side: f64| {
        let n = kurbo::Vec2::new(1.0, -1.0) / 2f64.sqrt() * (0.001 / 0.75);
        let (a, b) = (Point::ZERO, Point::new(side, side));
        PathData::new(vec![SubPath::new(
            vec![
                Anchor::with_handles(a, Point::new(side / 3.0, side / 3.0) + n, a),
                Anchor::with_handles(b, b, Point::new(side * 2.0 / 3.0, side * 2.0 / 3.0) + n),
            ],
            true,
        )])
    };
    let l = lens(100.0);
    let c = inscribed_circle(&l, FillRule::NonZero).unwrap();
    assert!(c.radius > 0.0 && c.radius <= 0.0005 + 1e-9, "{c:?}");
    assert!(fill_contains(&l.to_bezpath(), FillRule::NonZero, c.center), "{c:?}");
    // A curve that long is flattened too coarsely to show the bow at all: too complex, not empty.
    assert_eq!(inscribed_circle(&lens(1e6), FillRule::NonZero), Err(InscribedError::TooComplex));
}

#[test]
fn a_bowed_strip_thinner_than_the_flattening_tolerance_is_not_reported_empty() {
    // Two curves 0.001 pt apart, both bowed 0.003 pt the same way: flattened to 0.005 pt they
    // become a straight strip that lies outside the bowed one.
    let strip = |side: f64| {
        let unit = kurbo::Vec2::new(1.0, -1.0) / 2f64.sqrt();
        let (bow, gap) = (unit * (0.003 / 0.75), unit * 0.001);
        let (a, b) = (Point::ZERO, Point::new(side, side));
        let third = (b - a) / 3.0;
        let (a1, b1) = (a + gap, b + gap);
        PathData::new(vec![SubPath::new(
            vec![
                Anchor::with_handles(a, a, a + third + bow),
                Anchor::with_handles(b, b - third + bow, b),
                Anchor::with_handles(b1, b1, b1 - third + bow),
                Anchor::with_handles(a1, a1 + third + bow, a1),
            ],
            true,
        )])
    };
    let inside = |s: &PathData, c: Inscribed| {
        assert!(c.radius > 0.0 && c.radius <= 0.0005 + 1e-9, "{c:?}");
        // Measured against the curves flattened to 1e-8 pt by kurbo itself, which does not
        // coarsen long curves: kurbo's winding can miss a curve a point lies this close to.
        let mut polyline = kurbo::BezPath::new();
        kurbo::flatten(s.to_bezpath(), 1e-8, |el| polyline.push(el));
        let fine = edges(&polyline, 1e-8).unwrap();
        assert!(rim(c).all(|p| super::inside(&fine, p) > 1e-8), "{c:?} reaches outside");
    };
    // 28 and 141 pt long: a circle inside it, or too complex to measure; never "no area".
    for side in [20.0, 100.0] {
        let s = strip(side);
        match inscribed_circle(&s, FillRule::NonZero) {
            Ok(c) => inside(&s, c),
            e => assert_eq!(e, Err(InscribedError::TooComplex), "{side}"),
        }
    }
}

#[test]
fn a_thin_turned_rectangle_gets_the_circle_at_its_center() {
    // 100 pt long and 0.009998 pt wide, turned 45°: no wider than twice the flattening tolerance.
    let (u, v) = (kurbo::Vec2::new(1.0, 1.0) / 2f64.sqrt(), kurbo::Vec2::new(1.0, -1.0) / 2f64.sqrt());
    let (along, across) = (u * 100.0, v * 0.009998);
    let corners = [Point::ZERO, Point::ZERO + along, Point::ZERO + along + across, Point::ZERO + across];
    let r = polygon(&corners.map(|p| (p.x, p.y)));
    let c = inscribed_circle(&r, FillRule::NonZero).unwrap();
    holds(&r, FillRule::NonZero, c, 0.004999);
    assert!(c.radius > 0.004999 * 0.99, "{c:?}");
    // 1,000 pt long and 0.0001 pt wide: also measured at the center.
    let (along, across) = (u * 1000.0, v * 0.0001);
    let corners = [Point::ZERO, Point::ZERO + along, Point::ZERO + along + across, Point::ZERO + across];
    let r = polygon(&corners.map(|p| (p.x, p.y)));
    let c = inscribed_circle(&r, FillRule::NonZero).unwrap();
    assert!(c.radius > 0.00005 * 0.99 && c.radius <= 0.00005 + 1e-9, "{c:?}");
}

#[test]
fn a_search_past_its_cell_limit_is_too_complex() {
    let l = polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 30.0), (30.0, 30.0), (30.0, 100.0), (0.0, 100.0)]);
    let e = edges(&l.to_bezpath(), 0.005).unwrap();
    let b = Rect::new(0.0, 0.0, 100.0, 100.0);
    assert!(search_measuring(&e, b, (0.01, MAX_INSCRIBED_WORK, MAX_INSCRIBED_CELLS), &mut 0).is_ok());
    assert_eq!(search_measuring(&e, b, (0.01, MAX_INSCRIBED_WORK, 10), &mut 0), Err(InscribedError::TooComplex));
}

#[test]
fn an_outline_with_too_many_edges_is_too_complex() {
    let zigzag: Vec<(f64, f64)> = (0..=MAX_INSCRIBED_EDGES).map(|i| (i as f64, (i % 2) as f64)).chain([(0.0, -10.0)]).collect();
    let bp = polygon(&zigzag).to_bezpath();
    assert_eq!(edges(&bp, 0.005), Err(InscribedError::TooComplex));
    // Too many anchors are refused before the outline is resolved, even on a line that would
    // otherwise fill no area.
    let line: Vec<(f64, f64)> = (0..=MAX_INSCRIBED_EDGES).map(|i| (i as f64, 0.0)).collect();
    assert_eq!(inscribed_circle(&polygon(&line), FillRule::NonZero), Err(InscribedError::TooComplex));
    assert_eq!(inscribed_circle(&polygon(&line[..1000]), FillRule::NonZero), Err(InscribedError::NoArea));
    assert_eq!(edges(&polygon(&zigzag[..1000]).to_bezpath(), 0.005).map(|e| e.len()), Ok(1000));
}

#[test]
fn a_search_past_its_work_limit_is_too_complex() {
    let l = polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 30.0), (30.0, 30.0), (30.0, 100.0), (0.0, 100.0)]);
    let e = edges(&l.to_bezpath(), 0.005).unwrap();
    let b = Rect::new(0.0, 0.0, 100.0, 100.0);
    assert!(search(&e, b, 0.01, &mut MAX_INSCRIBED_WORK.clone()).is_ok());
    assert_eq!(search(&e, b, 0.01, &mut (20 * e.len())), Err(InscribedError::TooComplex));
}

#[test]
fn searches_that_share_a_budget_stop_when_it_runs_out() {
    let l = polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 30.0), (30.0, 30.0), (30.0, 100.0), (0.0, 100.0)]);
    let mut work = MAX_INSCRIBED_WORK;
    assert!(inscribed_circle_within(&l, FillRule::NonZero, &mut work).is_ok());
    let one = MAX_INSCRIBED_WORK - work;
    assert!(one > 0);
    // Enough for one search and half of another: the first fits, the second runs out.
    let mut work = one + one / 2;
    assert!(inscribed_circle_within(&l, FillRule::NonZero, &mut work).is_ok());
    assert!(work < one, "{work} left of {one}");
    assert_eq!(inscribed_circle_within(&l, FillRule::NonZero, &mut work), Err(InscribedError::TooComplex));
    // One distance short of a whole search: refused without measuring past the budget.
    let mut work = one - 1;
    assert_eq!(inscribed_circle_within(&l, FillRule::NonZero, &mut work), Err(InscribedError::TooComplex));
    let measured = MEASURED.with(std::cell::Cell::get);
    assert!(measured < one, "{measured} measured, budget {}", one - 1);
}

#[test]
fn an_area_the_fill_rule_leaves_small_is_measured_to_its_own_scale() {
    // Two identical 100 pt squares cancel under even-odd, leaving the small L of the test above:
    // the outline's box is 100 pt across, the filled area's 0.1 pt.
    let sq = polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    let l = polygon(
        &[(0.0, 0.0), (100.0, 0.0), (100.0, 30.0), (30.0, 30.0), (30.0, 100.0), (0.0, 100.0)].map(|(x, y)| (x / 1000.0 + 50.0, y / 1000.0 + 50.0)),
    );
    let all = PathData::new(sq.subpaths.iter().cloned().chain(sq.subpaths.iter().cloned()).chain(l.subpaths).collect());
    let c = inscribed_circle(&all, FillRule::EvenOdd).unwrap();
    let largest = 0.03 * std::f64::consts::SQRT_2 / (1.0 + std::f64::consts::SQRT_2);
    assert!(c.radius <= largest + 1e-9 && c.radius > largest * 0.99, "{c:?}, largest {largest}");
}

#[test]
fn a_shape_with_no_area_has_no_circle() {
    let flat = polygon(&[(0.0, 0.0), (100.0, 0.0), (50.0, 0.0)]);
    assert_eq!(inscribed_circle(&flat, FillRule::NonZero), Err(InscribedError::NoArea));
    assert_eq!(inscribed_circle(&PathData::default(), FillRule::NonZero), Err(InscribedError::NoArea));
}
