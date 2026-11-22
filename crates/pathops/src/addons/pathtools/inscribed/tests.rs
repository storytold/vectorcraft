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
    // A strip 5,000 × 1 whose top edge has 5,000 pieces: cut into cells as wide as it is high,
    // the first cells alone would measure 5,000 times every edge.
    let n = 5000;
    let top: Vec<(f64, f64)> = (0..=n).map(|i| (i as f64, 0.0)).collect();
    let pts: Vec<(f64, f64)> = top.into_iter().chain([(n as f64, 1.0), (0.0, 1.0)]).collect();
    let e = edges(&polygon(&pts).to_bezpath(), 0.005).unwrap();
    let limit = 100 * e.len();
    assert_eq!(search(&e, Rect::new(0.0, 0.0, n as f64, 1.0), 0.01, limit), Err(InscribedError::TooComplex));
    // Past the limit by at most the four cells of one split.
    let measured = MEASURED.with(std::cell::Cell::get);
    assert!(measured <= limit + 4 * e.len(), "{measured} measured, limit {limit}");
}

#[test]
fn an_outline_with_too_many_edges_is_too_complex() {
    let zigzag: Vec<(f64, f64)> = (0..=MAX_INSCRIBED_EDGES).map(|i| (i as f64, (i % 2) as f64)).chain([(0.0, -10.0)]).collect();
    let bp = polygon(&zigzag).to_bezpath();
    assert_eq!(edges(&bp, 0.005), Err(InscribedError::TooComplex));
    assert_eq!(edges(&polygon(&zigzag[..1000]).to_bezpath(), 0.005).map(|e| e.len()), Ok(1000));
}

#[test]
fn a_search_past_its_work_limit_is_too_complex() {
    let l = polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 30.0), (30.0, 30.0), (30.0, 100.0), (0.0, 100.0)]);
    let e = edges(&l.to_bezpath(), 0.005).unwrap();
    let b = Rect::new(0.0, 0.0, 100.0, 100.0);
    assert!(search(&e, b, 0.01, MAX_WORK).is_ok());
    assert_eq!(search(&e, b, 0.01, 20 * e.len()), Err(InscribedError::TooComplex));
}

#[test]
fn a_shape_with_no_area_has_no_circle() {
    let flat = polygon(&[(0.0, 0.0), (100.0, 0.0), (50.0, 0.0)]);
    assert_eq!(inscribed_circle(&flat, FillRule::NonZero), Err(InscribedError::NoArea));
    assert_eq!(inscribed_circle(&PathData::default(), FillRule::NonZero), Err(InscribedError::NoArea));
}
