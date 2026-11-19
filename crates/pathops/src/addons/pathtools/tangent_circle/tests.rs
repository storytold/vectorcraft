use kurbo::{Affine, CubicBez, Point, Rect};
use vectorcraft_geom::{PathData, SubPath, shapes};

use super::*;

fn circle(x: f64, y: f64, r: f64) -> PathData {
    shapes::ellipse(Rect::new(x - r, y - r, x + r, y + r))
}

fn square(x: f64, y: f64, half: f64) -> PathData {
    shapes::rectangle(Rect::new(x - half, y - half, x + half, y + half))
}

/// Is `c` touching the ideal circle about `center` of radius `r`: `|c − center| = c.r ± r`?
fn touches_circle(c: &Circle, center: Point, r: f64, tol: f64) -> bool {
    let d = (c.center - center).hypot();
    (d - (c.radius + r)).abs() <= tol || (d - (c.radius - r).abs()).abs() <= tol
}

/// The nearest and farthest distance from `q` to points `pts`, a dense sampling of a curve.
fn reach(pts: &[Point], q: Point) -> (f64, f64) {
    pts.iter().fold((f64::INFINITY, 0.0), |(lo, hi), p| {
        let d = (*p - q).hypot();
        (lo.min(d), hi.max(d))
    })
}

/// Does circle `c` touch the densely sampled outline `pts`: its nearest or farthest point on the circle?
fn touches(c: &Circle, pts: &[Point], tol: f64) -> bool {
    let (lo, hi) = reach(pts, c.center);
    (lo - c.radius).abs() <= tol || (hi - c.radius).abs() <= tol
}

/// Points along every segment of `path`, `n` per segment.
fn samples(path: &PathData, n: usize) -> Vec<Point> {
    path.subpaths
        .iter()
        .flat_map(|sp| (0..sp.segment_count()).flat_map(move |i| (0..n).map(move |k| sp.segment(i).eval(k as f64 / n as f64))))
        .collect()
}

fn has(found: &[Circle], center: Point, r: f64, tol: f64) -> bool {
    found.iter().any(|c| (c.center - center).hypot() <= tol && (c.radius - r).abs() <= tol)
}

#[test]
fn three_circles_give_the_eight_apollonius_circles() {
    // Equal circles about the corners of a 3-4-5 triangle: the circumcenter (2, 1.5) at 2.5 from
    // each center gives the circle outside all three (r = 2.5 − 1) and the one holding all three
    // (r = 2.5 + 1).
    let shapes = [circle(0.0, 0.0, 1.0), circle(4.0, 0.0, 1.0), circle(0.0, 3.0, 1.0)];
    let found = tangent_circles(&[&shapes[0], &shapes[1], &shapes[2]], None).unwrap();
    // The answers are polished on the Bézier circles, which stray up to 0.0273% of their radius
    // from the ideal ones.
    let tol = 2e-3;
    assert_eq!(found.len(), 8, "{found:?}");
    assert!(has(&found, Point::new(2.0, 1.5), 1.5, tol), "{found:?}");
    assert!(has(&found, Point::new(2.0, 1.5), 3.5, tol), "{found:?}");
    for c in &found {
        for (center, r) in [(Point::new(0.0, 0.0), 1.0), (Point::new(4.0, 0.0), 1.0), (Point::new(0.0, 3.0), 1.0)] {
            assert!(touches_circle(c, center, r, tol), "{c:?} and {center:?}");
        }
    }
    // Listed by radius.
    assert!(found.windows(2).all(|w| w[0].radius <= w[1].radius + 1e-9));
}

#[test]
fn answers_for_circles_touch_the_drawn_bezier_outlines() {
    // Radius-100 Bézier circles stray up to 0.027 pt from the ideal circles; the answers touch the
    // drawn outlines far closer than that.
    let shapes = [circle(0.0, 0.0, 100.0), circle(500.0, 40.0, 100.0), circle(180.0, 420.0, 100.0)];
    let found = tangent_circles(&[&shapes[0], &shapes[1], &shapes[2]], None).unwrap();
    assert_eq!(found.len(), 8, "{found:?}");
    let dense: Vec<Vec<Point>> = shapes.iter().map(|s| samples(s, 20_000)).collect();
    for c in &found {
        for pts in &dense {
            assert!(touches(c, pts, 1e-3), "{c:?}");
        }
    }
    let pair = tangent_circles(&[&shapes[0], &shapes[1]], Some(250.0)).unwrap();
    assert!(!pair.is_empty());
    for c in &pair {
        assert!(touches(c, &dense[0], 1e-3) && touches(c, &dense[1], 1e-3), "{c:?}");
    }
}

#[test]
fn three_circles_of_different_sizes() {
    // A worked case: circles (0,0) r 1, (10,0) r 2, (5,8) r 3. Every answer satisfies the
    // tangency equations, and there are eight since no circle holds or crosses another.
    let defs = [(0.0, 0.0, 1.0), (10.0, 0.0, 2.0), (5.0, 8.0, 3.0)];
    let shapes = defs.map(|(x, y, r)| circle(x, y, r));
    let found = tangent_circles(&[&shapes[0], &shapes[1], &shapes[2]], None).unwrap();
    assert_eq!(found.len(), 8, "{found:?}");
    for c in &found {
        for (x, y, r) in defs {
            assert!(touches_circle(c, Point::new(x, y), r, 5e-3), "{c:?}");
        }
    }
}

#[test]
fn only_true_circles_take_the_closed_form() {
    assert!(as_circle(&circle(0.0, 0.0, 100.0)).is_some());
    // Turned, a Bézier circle keeps its square box.
    assert!(as_circle(&circle(0.0, 0.0, 100.0).transformed(Affine::rotate(0.35))).is_some());
    // A 200 × 200.1 ellipse is searched along its own outline.
    assert!(as_circle(&shapes::ellipse(Rect::new(0.0, 0.0, 200.0, 200.1))).is_none());
    assert!(as_circle(&square(0.0, 0.0, 10.0)).is_none());
    // A 720-sided polygon lies within 0.03% of its circle, but is searched as drawn.
    let polygon: Vec<Point> = (0..720)
        .map(|k| Point::new(100.0 * (k as f64 / 360.0 * std::f64::consts::PI).cos(), 100.0 * (k as f64 / 360.0 * std::f64::consts::PI).sin()))
        .collect();
    assert!(as_circle(&PathData::single(SubPath::polyline(&polygon, true))).is_none());
}

#[test]
fn quadratics_without_a_linear_term_give_both_roots() {
    let mut roots = quadratic_roots(1.0, 0.0, -1.0);
    roots.sort_by(f64::total_cmp);
    assert_eq!(roots, vec![-1.0, 1.0]);
    let mut roots = quadratic_roots(2.0, -0.0, -8.0);
    roots.sort_by(f64::total_cmp);
    assert_eq!(roots, vec![-2.0, 2.0]);
}

#[test]
fn three_circles_keep_the_radius_limit() {
    // Nearly in a row: some answers have radii near 1250, past 64 × the bounds' diagonal (about 770).
    let shapes = [circle(0.0, 0.0, 1.0), circle(10.0, 0.0, 1.0), circle(5.0, 0.01, 1.0)];
    let found = tangent_circles(&[&shapes[0], &shapes[1], &shapes[2]], None).unwrap();
    let b = shapes.iter().map(|s| s.control_bounds().unwrap()).reduce(|a, b| a.union(b)).unwrap();
    let limit = MAX_RADIUS_FACTOR * b.width().hypot(b.height());
    assert!(!found.is_empty() && found.len() < 8, "{found:?}");
    assert!(found.iter().all(|c| c.radius <= limit), "{found:?}");
}

#[test]
fn two_circles_and_a_radius() {
    let a = circle(0.0, 0.0, 10.0);
    let b = circle(30.0, 0.0, 10.0);
    // Polished on the Bézier circles (see above).
    let tol = 0.02;
    // Radius 5: only the circle in the gap, touching both at once.
    let found = tangent_circles(&[&a, &b], Some(5.0)).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(has(&found, Point::new(15.0, 0.0), 5.0, tol));
    // Radius 20: two outside both, two holding the first and two holding the second.
    let found = tangent_circles(&[&a, &b], Some(20.0)).unwrap();
    assert_eq!(found.len(), 6, "{found:?}");
    let h = (30.0f64 * 30.0 - 15.0 * 15.0).sqrt();
    assert!(has(&found, Point::new(15.0, -h), 20.0, tol) && has(&found, Point::new(15.0, h), 20.0, tol));
    for c in &found {
        assert!(touches_circle(c, Point::new(0.0, 0.0), 10.0, tol) && touches_circle(c, Point::new(30.0, 0.0), 10.0, tol));
    }
    // Top to bottom for one radius.
    assert!(found.windows(2).all(|w| w[0].center.y <= w[1].center.y + 1e-9));
    // Radius 10, the circles' own, 20 apart: each circle itself touches the other at one point; with
    // the two outside both, four in all.
    let c = circle(20.0, 0.0, 10.0);
    let found = tangent_circles(&[&a, &c], Some(10.0)).unwrap();
    assert_eq!(found.len(), 4, "{found:?}");
    assert!(has(&found, Point::new(0.0, 0.0), 10.0, tol) && has(&found, Point::new(20.0, 0.0), 10.0, tol), "{found:?}");
    for f in &found {
        assert!(touches_circle(f, Point::new(0.0, 0.0), 10.0, tol) && touches_circle(f, Point::new(20.0, 0.0), 10.0, tol), "{f:?}");
    }
    // Too small to reach across: none.
    let far = circle(100.0, 0.0, 10.0);
    assert_eq!(tangent_circles(&[&a, &far], Some(5.0)).unwrap(), vec![]);
}

#[test]
fn the_numeric_search_agrees_with_the_closed_form() {
    // The grid search and Newton on the Bézier outlines find what the closed form finds, within
    // the Bézier circle's own error (0.03% of the radius).
    let defs = [(0.0, 0.0, 10.0), (35.0, 5.0, 6.0), (12.0, 30.0, 4.0)];
    let shapes = defs.map(|(x, y, r)| circle(x, y, r));
    let refs = [&shapes[0], &shapes[1], &shapes[2]];
    let closed = tangent_circles(&refs, None).unwrap();
    let b = shapes.iter().map(|s| s.control_bounds().unwrap()).reduce(|a, b| a.union(b)).unwrap();
    let (o, s) = (b.center(), b.width().hypot(b.height()));
    let local = |p: Point| Point::new((p.x - o.x) / s, (p.y - o.y) / s);
    let outlines: Vec<Outline> = refs.iter().map(|p| Outline::new(p, &local)).collect();
    let numeric3 = dedup(numeric(&outlines, None).unwrap(), SAME);
    assert_eq!(numeric3.len(), closed.len(), "{numeric3:?} vs {closed:?}");
    for c in &closed {
        let tol = 1e-3 * c.radius.max(10.0) / s;
        assert!(has(&numeric3, local(c.center), c.radius / s, tol), "{c:?} missing from {numeric3:?}");
    }
    let closed2 = tangent_circles(&refs[..2], Some(12.0)).unwrap();
    let numeric2 = dedup(numeric(&outlines[..2], Some(12.0 / s)).unwrap(), SAME);
    assert_eq!(numeric2.len(), closed2.len(), "{numeric2:?} vs {closed2:?}");
    for c in &closed2 {
        assert!(has(&numeric2, local(c.center), c.radius / s, 1e-2 / s), "{c:?}");
    }
}

/// An ellipse with semi-axes `a` and `b` about the origin, mapped by `m`: its Bézier path and its
/// exact points by angle.
fn mapped_ellipse(a: f64, b: f64, m: Affine) -> (PathData, impl Fn(f64) -> Point + Copy) {
    let path = shapes::ellipse(Rect::new(-a, -b, a, b)).transformed(m);
    let exact = move |th: f64| m * Point::new(a * th.cos(), b * th.sin());
    (path, exact)
}

/// Independently of the code under test: the centers of the circles of radius `r` that touch the
/// exact convex curve `exact` (about `middle`) and the circle about `cc` of radius `rc`, in every
/// combination. A center lies on the curve's normal at `r` (outward or inward) where its distance to
/// `cc` is `rc + r` or `|rc − r|`; it counts when the curve's nearest or farthest point from it is
/// at `r`.
fn independent_centers(exact: impl Fn(f64) -> Point + Copy, middle: Point, cc: Point, rc: f64, r: f64) -> Vec<Point> {
    let tau = std::f64::consts::TAU;
    let dense: Vec<Point> = (0..100_000).map(|k| exact(k as f64 / 100_000.0 * tau)).collect();
    let n = 200_000;
    let mut out: Vec<Point> = vec![];
    for side in [1.0, -1.0] {
        let normal = move |th: f64| {
            let p = exact(th);
            let t = (exact(th + 1e-7) - exact(th - 1e-7)).normalize();
            let mut nrm = Vec2::new(t.y, -t.x);
            if nrm.dot(p - middle) < 0.0 {
                nrm = -nrm;
            }
            p + nrm * (side * r)
        };
        for target in [rc + r, (rc - r).abs()] {
            let g = |th: f64| (normal(th) - cc).hypot() - target;
            for k in 0..n {
                let (mut a, mut b) = (k as f64 / n as f64 * tau, (k + 1) as f64 / n as f64 * tau);
                if g(a) * g(b) < 0.0 {
                    for _ in 0..60 {
                        let m = (a + b) / 2.0;
                        if g(a) * g(m) <= 0.0 { b = m } else { a = m }
                    }
                    let q = normal(a);
                    let (lo, hi) = reach(&dense, q);
                    let touching = (lo - r).abs() <= 1e-3 || (hi - r).abs() <= 1e-3;
                    if touching && !out.iter().any(|o| (*o - q).hypot() <= 1e-6) {
                        out.push(q);
                    }
                }
            }
        }
    }
    out
}

/// `found` and `independent` are the same centers, within `tol`.
fn same_centers(found: &[Circle], independent: &[Point], tol: f64) {
    assert_eq!(found.len(), independent.len(), "{found:?} vs {independent:?}");
    for p in independent {
        assert!(found.iter().any(|c| (c.center - *p).hypot() <= tol), "{p:?} missing from {found:?}");
    }
}

#[test]
fn a_rotated_ellipse_and_a_circle_match_an_independent_solve() {
    let (ellipse, exact) = mapped_ellipse(40.0, 20.0, Affine::rotate(30f64.to_radians()));
    let (cc, rc, r) = (Point::new(70.0, 10.0), 12.0, 15.0);
    let disc = circle(cc.x, cc.y, rc);
    let found = tangent_circles(&[&ellipse, &disc], Some(r)).unwrap();
    // Every circle touches the analytic ellipse and the circle (Bézier error: 0.03% of 40 pt).
    let dense: Vec<Point> = (0..200_000).map(|k| exact(k as f64 / 200_000.0 * std::f64::consts::TAU)).collect();
    for c in &found {
        assert!(touches(c, &dense, 0.03), "{c:?}");
        assert!(touches_circle(c, cc, rc, 0.01), "{c:?}");
    }
    // Every combination (outside both, holding the circle, holding the ellipse, inside it) solved
    // independently: the same circles.
    same_centers(&found, &independent_centers(exact, Point::ZERO, cc, rc, r), 0.05);
    // A circle overlapping the ellipse, with a radius large enough to hold either.
    let (cc, rc, r) = (Point::new(30.0, 5.0), 15.0, 50.0);
    let found = tangent_circles(&[&ellipse, &circle(cc.x, cc.y, rc)], Some(r)).unwrap();
    same_centers(&found, &independent_centers(exact, Point::ZERO, cc, rc, r), 0.05);
}

#[test]
fn a_rotated_and_skewed_ellipse_matches_an_independent_solve() {
    let m = Affine::translate((-20.0, 15.0)) * Affine::skew(0.4, 0.0) * Affine::rotate(-0.7);
    let (ellipse, exact) = mapped_ellipse(30.0, 12.0, m);
    let (cc, rc, r) = (Point::new(15.0, 30.0), 8.0, 20.0);
    let found = tangent_circles(&[&ellipse, &circle(cc.x, cc.y, rc)], Some(r)).unwrap();
    let independent = independent_centers(exact, m * Point::ZERO, cc, rc, r);
    assert!(!independent.is_empty());
    same_centers(&found, &independent, 0.05);
}

#[test]
fn two_centers_close_together_are_both_found() {
    // An ellipse 20 × 10 about (100, 0) and a circle of radius 10 about the origin: a circle of
    // radius 35 fits the 70 pt gap between them on the axis. At 35.000003 two circles touch both
    // from outside, about 0.011 pt above and below the axis.
    let (ellipse, exact) = mapped_ellipse(20.0, 10.0, Affine::translate((100.0, 0.0)));
    let (cc, rc, r) = (Point::ZERO, 10.0, 35.000003);
    let found = tangent_circles(&[&ellipse, &circle(cc.x, cc.y, rc)], Some(r)).unwrap();
    let independent = independent_centers(exact, Point::new(100.0, 0.0), cc, rc, r);
    assert_eq!(independent.iter().filter(|p| (p.x - 45.0).abs() < 1.0).count(), 2, "{independent:?}");
    same_centers(&found, &independent, 0.005);
    // Two tiny circles a million points apart, and a radius just past half the gap: the two
    // answers are about 22.36 pt above and below the axis, close together in units of the gap.
    let (a, b) = (circle(0.0, 0.0, 0.01), circle(1e6, 0.0, 0.01));
    let found = tangent_circles(&[&a, &b], Some(499999.9905)).unwrap();
    assert_eq!(found.len(), 2, "{found:?}");
    for c in &found {
        assert!((c.center.x - 5e5).abs() < 0.1 && (c.center.y.abs() - 500f64.sqrt()).abs() < 0.1, "{found:?}");
    }
}

#[test]
fn three_circles_give_eight_distinct_circles_in_a_steady_order() {
    // Mirror-image pairs about y = 0 have radii equal up to rounding: the upper one comes first.
    let defs = [(0.0, -5.0, 1.0), (0.0, 5.0, 1.0), (12.0, 0.0, 2.0)];
    let shapes = defs.map(|(x, y, r)| circle(x, y, r));
    let found = tangent_circles(&[&shapes[0], &shapes[1], &shapes[2]], None).unwrap();
    for (i, a) in found.iter().enumerate() {
        for b in &found[i + 1..] {
            assert!((a.center - b.center).hypot() + (a.radius - b.radius).abs() > 1e-3, "{a:?} twice in {found:?}");
        }
    }
    for w in found.windows(2) {
        if (w[0].radius - w[1].radius).abs() <= 1e-9 {
            assert!(w[0].center.y < w[1].center.y, "{found:?}");
        }
    }
    // The selection mirrored left to right: the same circles mirrored, in the same order.
    let flip = Affine::scale_non_uniform(-1.0, 1.0);
    let mirrored = shapes.clone().map(|s| s.transformed(flip));
    let again = tangent_circles(&[&mirrored[0], &mirrored[1], &mirrored[2]], None).unwrap();
    assert_eq!(again.len(), found.len());
    for (a, b) in found.iter().zip(&again) {
        assert!((flip * a.center - b.center).hypot() <= 1e-6 && (a.radius - b.radius).abs() <= 1e-6, "{found:?} vs {again:?}");
    }
}

#[test]
fn radii_equal_up_to_rounding_sort_top_to_bottom() {
    let lower = Circle { center: Point::new(0.0, 0.2), radius: 0.1 };
    let upper = Circle { center: Point::new(0.0, -0.2), radius: 0.1 + 1e-13 };
    let wider = Circle { center: Point::new(0.0, -0.5), radius: 0.2 };
    let mut v = [wider, lower, upper];
    sort(&mut v, SAME);
    assert_eq!(v, [upper, lower, wider]);
}

#[test]
fn the_nearest_point_is_looked_for_on_every_segment_it_may_be_on() {
    // An arc bulging to y = −0.3 whose one chord lies on y = 0, and a line on y = −0.25. Seen from
    // (0, −1), the line's chord is nearer (0.75 against 1.0), but the arc itself is (0.7).
    let arc = CubicBez::new((-1.0, 0.0), (-1.0, -0.4), (1.0, -0.4), (1.0, 0.0));
    let line = CubicBez::new((-0.5, -0.25), (-0.2, -0.25), (0.2, -0.25), (0.5, -0.25));
    let outline = Outline {
        segments: vec![arc, line],
        beside: vec![(None, None), (None, None)],
        chords: vec![(arc.p0, arc.p3, 0, 0.0, 1.0), (line.p0, line.p3, 1, 0.0, 1.0)],
        slack: 0.3,
    };
    let (p, d) = outline.extreme(Point::new(0.0, -1.0), false);
    assert!((d - 0.7).abs() <= 1e-9 && (p - Point::new(0.0, -0.3)).hypot() <= 1e-9, "{p:?} {d}");
}

#[test]
fn copies_among_three_shapes_are_a_whole_family() {
    use TangentCircleError::*;
    // A circle turned and turned back differs from itself by rounding only.
    let b = circle(3.0, 7.0, 20.0);
    let turned = b.transformed(Affine::rotate(0.3)).transformed(Affine::rotate(-0.3));
    assert_ne!(b, turned);
    assert_eq!(tangent_circles(&[&b, &turned, &circle(100.0, 0.0, 5.0)], None), Err(TooMany));
    // Two copies of a square.
    let sq = square(0.0, 0.0, 10.0);
    assert_eq!(tangent_circles(&[&sq, &sq.clone(), &square(60.0, 0.0, 10.0)], None), Err(TooMany));
    // Two circles half a point apart, a million points from the third, are not copies.
    let far = [circle(0.0, 0.0, 10.0), circle(0.0, 0.5, 10.0), circle(1e6, 0.0, 10.0)];
    assert_ne!(tangent_circles(&[&far[0], &far[1], &far[2]], None), Err(TooMany));
}

#[test]
fn three_squares_are_solved_numerically() {
    let shapes = [square(0.0, 0.0, 10.0), square(60.0, 0.0, 10.0), square(30.0, 50.0, 10.0)];
    let found = tangent_circles(&[&shapes[0], &shapes[1], &shapes[2]], None).unwrap();
    assert!(!found.is_empty());
    let dense: Vec<Vec<Point>> = shapes.iter().map(|s| samples(s, 4000)).collect();
    for c in &found {
        for pts in &dense {
            assert!(touches(c, pts, 0.01), "{c:?}");
        }
    }
    // By symmetry, the circle in the middle touching all three from outside is centered on x = 30.
    let inner = found.iter().find(|c| (c.center.x - 30.0).abs() < 1e-6 && c.center.y > 0.0 && c.center.y < 40.0);
    assert!(inner.is_some(), "{found:?}");
}

#[test]
fn compound_and_far_off_paths_give_the_same_circles_moved() {
    // A ring (two subpaths of one path) and a small square in its hole.
    let mut ring = circle(0.0, 0.0, 50.0);
    ring.subpaths.extend(circle(0.0, 0.0, 30.0).subpaths);
    let dot = square(0.0, 0.0, 5.0);
    // Radius 12: in the hole 18 from the center, and 12 from a corner of the square: two by each
    // corner.
    let found = tangent_circles(&[&ring, &dot], Some(12.0)).unwrap();
    assert_eq!(found.len(), 8, "{found:?}");
    let (ring_pts, dot_pts) = (samples(&ring, 4000), samples(&dot, 4000));
    for c in &found {
        assert!(touches(c, &ring_pts, 0.02) && touches(c, &dot_pts, 0.02), "{c:?}");
    }
    // Moved far off and scaled: the same circles, moved and scaled.
    let m = Affine::translate((1e6, -2e6)) * Affine::scale(3.0);
    let (ring2, dot2) = (ring.transformed(m), dot.transformed(m));
    let moved = tangent_circles(&[&ring2, &dot2], Some(36.0)).unwrap();
    assert_eq!(moved.len(), found.len(), "{moved:?} vs {found:?}");
    for c in &found {
        assert!(has(&moved, m * c.center, 3.0 * c.radius, 1e-4), "{c:?}");
    }
}

#[test]
fn open_paths_and_no_answer() {
    let line = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0)], false));
    let disc = circle(50.0, 40.0, 10.0);
    // Radius 10: a center 10 from the line is at least 30 from the disc's center, never 20.
    assert_eq!(tangent_circles(&[&line, &disc], Some(10.0)).unwrap(), vec![]);
    // Radius 20: centers on y = 20, 30 from (50, 40): x = 50 ± √500.
    let found = tangent_circles(&[&line, &disc], Some(20.0)).unwrap();
    assert_eq!(found.len(), 2, "{found:?}");
    let dx = 500f64.sqrt();
    assert!(has(&found, Point::new(50.0 - dx, 20.0), 20.0, 0.05) && has(&found, Point::new(50.0 + dx, 20.0), 20.0, 0.05), "{found:?}");
    // A given radius far past 64 × the bounds' diagonal: two short segments 10 apart and radius
    // 1000 (the limit applies only when three shapes set the radius).
    let left = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(1.0, 0.0)], false));
    let right = PathData::single(SubPath::polyline(&[Point::new(10.0, 0.0), Point::new(11.0, 0.0)], false));
    let found = tangent_circles(&[&left, &right], Some(1000.0)).unwrap();
    assert!(!found.is_empty());
    let (lp, rp) = (samples(&left, 4000), samples(&right, 4000));
    for c in &found {
        assert!(c.radius == 1000.0 && touches(c, &lp, 1e-3) && touches(c, &rp, 1e-3), "{c:?}");
    }
    // Two squares far apart with a small radius: none.
    let (a, b) = (square(0.0, 0.0, 10.0), square(500.0, 0.0, 10.0));
    assert_eq!(tangent_circles(&[&a, &b], Some(5.0)).unwrap(), vec![]);
}

#[test]
fn whole_curves_of_answers_and_bad_input_are_errors() {
    use TangentCircleError::*;
    let (a, b) = (circle(0.0, 0.0, 50.0), circle(0.0, 0.0, 20.0));
    assert_eq!(tangent_circles(&[&a, &b], Some(15.0)), Err(TooMany));
    let (outer, inner) = (square(0.0, 0.0, 50.0), square(0.0, 0.0, 20.0));
    assert_eq!(tangent_circles(&[&outer, &inner], Some(15.0)), Err(TooMany));
    assert_eq!(tangent_circles(&[&a], Some(1.0)), Err(BadInput));
    assert_eq!(tangent_circles(&[&a, &b], None), Err(BadInput));
    assert_eq!(tangent_circles(&[&a, &b, &a], Some(1.0)), Err(BadInput));
    assert_eq!(tangent_circles(&[&a, &b], Some(0.0)), Err(BadInput));
    assert_eq!(tangent_circles(&[&a, &b], Some(f64::NAN)), Err(BadInput));
    // Two of three circles the same: a whole family touches all three.
    let third = circle(100.0, 0.0, 5.0);
    assert_eq!(tangent_circles(&[&b, &b.clone(), &third], None), Err(TooMany));
    let mut reversed = b.clone();
    reversed.reverse();
    assert_eq!(tangent_circles(&[&b, &reversed, &third], None), Err(TooMany));
    // Shapes so tiny that the radius overflows in their units.
    let (tiny_a, tiny_b) = (circle(0.0, 0.0, 1e-305), circle(1e-304, 0.0, 1e-305));
    assert_eq!(tangent_circles(&[&tiny_a, &tiny_b], Some(1e6)), Err(BadInput));
    let empty = PathData::new(vec![]);
    assert_eq!(tangent_circles(&[&a, &empty], Some(1.0)), Err(BadInput));
    let big = PathData::single(SubPath::polyline(&(0..2001).map(|i| Point::new(i as f64, (i % 2) as f64)).collect::<Vec<_>>(), false));
    assert_eq!(tangent_circles(&[&a, &big], Some(1.0)), Err(TooComplex));
    // Lone anchors count as segments.
    let dots = PathData::new((0..2001).map(|i| SubPath::polyline(&[Point::new(i as f64, 0.0)], false)).collect());
    assert_eq!(tangent_circle_segments(&dots), 2001);
    assert_eq!(tangent_circles(&[&a, &dots], Some(1.0)), Err(TooComplex));
}

#[test]
fn answers_about_a_tiny_shape_far_away_stay_apart() {
    // Each tiny circle is touched from outside and from inside: answers 0.02 pt apart on each side,
    // eight in all.
    let (a, b) = (circle(0.0, 0.0, 0.01), circle(1e6, 0.0, 0.01));
    let found = tangent_circles(&[&a, &b], Some(1e6)).unwrap();
    assert_eq!(found.len(), 8, "{found:?}");
    // All of one radius: top to bottom, then left to right.
    for w in found.windows(2) {
        let (a, b) = (w[0].center, w[1].center);
        assert!(a.y < b.y - 1e-3 || ((a.y - b.y).abs() <= 1e-3 && a.x < b.x), "{found:?}");
    }
}

/// A straight segment from `a` to `b`, its handles at its thirds so that `t` runs evenly along it.
fn line(a: (f64, f64), b: (f64, f64)) -> CubicBez {
    let (a, b) = (Point::new(a.0, a.1), Point::new(b.0, b.1));
    CubicBez::new(a, a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b)
}

#[test]
fn a_line_with_its_handles_on_its_anchors_has_a_direction_at_its_ends() {
    let (a, b) = (Point::new(0.0, 0.0), Point::new(100.0, 0.0));
    for t in [0.0, 1.0] {
        let p = CubicBez::new(a, a, b, b).eval(t);
        let found = circles_through(CubicBez::new(a, a, b, b), t, &[line((-50.0, 40.0), (150.0, 40.0))]);
        assert_eq!(found.len(), 1, "{t}: {found:?}");
        assert!(near(found[0].circle.center, p.x, 20.0, 1e-6), "{t}: {found:?}");
    }
}

fn near(p: Point, x: f64, y: f64, tol: f64) -> bool {
    (p - Point::new(x, y)).hypot() <= tol
}

#[test]
fn a_circle_through_a_point_of_one_line_touches_a_parallel_line() {
    let found = circles_through(line((0.0, 0.0), (100.0, 0.0)), 0.5, &[line((0.0, 40.0), (100.0, 40.0))]);
    // Only toward the other line: away from it, no circle reaches it.
    assert_eq!(found.len(), 1, "{found:?}");
    let c = found[0];
    assert!(near(c.circle.center, 50.0, 20.0, 1e-6) && (c.circle.radius - 20.0).abs() < 1e-6, "{c:?}");
    assert!(near(c.first, 50.0, 0.0, 1e-9) && near(c.second, 50.0, 40.0, 1e-6), "{c:?}");
}

#[test]
fn a_crossing_line_gives_a_circle_on_each_side() {
    let found = circles_through(line((0.0, 0.0), (100.0, 0.0)), 0.3, &[line((80.0, -100.0), (80.0, 100.0))]);
    assert_eq!(found.len(), 2, "{found:?}");
    for y in [50.0, -50.0] {
        assert!(found.iter().any(|c| near(c.circle.center, 30.0, y, 1e-6) && (c.circle.radius - 50.0).abs() < 1e-6), "{y}: {found:?}");
    }
}

#[test]
fn a_circle_that_crosses_the_first_segment_is_left_out() {
    // Through (50, 75) on a flat first segment, the circle touching y = -200 has radius 137.5.
    let below = [line((-500.0, -200.0), (500.0, -200.0))];
    let flat = circles_through(line((0.0, 75.0), (100.0, 75.0)), 0.5, &below);
    assert_eq!(flat.len(), 1, "{flat:?}");
    assert!((flat[0].circle.radius - 137.5).abs() < 1e-6, "{flat:?}");
    // On a hump from (0, 0) up to (50, 75) and back down to (100, 0), the same circle crosses the
    // hump's sides (its center (50, -62.5) is 80 from (0, 0)), and none on the other side reaches.
    let hump = CubicBez::new(Point::new(0.0, 0.0), Point::new(0.0, 100.0), Point::new(100.0, 100.0), Point::new(100.0, 0.0));
    assert!(near(hump.eval(0.5), 50.0, 75.0, 1e-9));
    let found = circles_through(hump, 0.5, &below);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn where_the_first_segment_turns_back_only_circles_that_touch_are_kept() {
    // Out to (75, 0) and back: the normal flips at the turn, and the distance to the line jumps there.
    let first = CubicBez::new(Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 0.0), Point::new(0.0, 0.0));
    let second = [line((-100.0, 40.0), (200.0, 40.0))];
    let found = circles_of_radius(first, &second, 30.0);
    for c in &found {
        assert!((c.circle.center.y - 10.0).abs() < 1e-6 || (c.circle.center.y - 70.0).abs() < 1e-6, "{c:?}");
    }
    assert!(!found.iter().any(|c| (c.circle.center.x - 75.0).abs() < 1.0), "{found:?}");
}

#[test]
fn a_circle_through_a_point_touches_a_curved_segment() {
    // The second segments are a Bézier circle of radius 20 about (50, 100): touched from outside,
    // |center − (50, 100)| = r + 20 with the center at (50, r), so r = 40.
    let ring = circle(50.0, 100.0, 20.0);
    let segs: Vec<CubicBez> = (0..4).map(|i| ring.subpaths[0].segment(i)).collect();
    let found = circles_through(line((0.0, 0.0), (100.0, 0.0)), 0.5, &segs);
    assert_eq!(found.len(), 1, "{found:?}");
    let c = found[0];
    // Within the Bézier circle's own error (0.0273% of 20).
    assert!((c.circle.radius - 40.0).abs() < 0.01 && near(c.circle.center, 50.0, 40.0, 0.01), "{c:?}");
    assert!(near(c.second, 50.0, 80.0, 0.01), "{c:?}");
}

#[test]
fn on_a_curved_first_segment_the_center_is_on_the_normal() {
    // A quarter of a ring of radius 50 about (0, 0); the second segment a line far off to the right.
    let ring = circle(0.0, 0.0, 50.0);
    let mut checked = 0;
    for i in 0..4 {
        let first = ring.subpaths[0].segment(i);
        for t in [0.0, 0.3, 0.7, 1.0] {
            let found = circles_through(first, t, &[line((300.0, -500.0), (300.0, 500.0))]);
            let tangent = kurbo::ParamCurveDeriv::deriv(&first).eval(t).to_vec2();
            for c in &found {
                let to_center = c.circle.center - c.first;
                assert!(to_center.dot(tangent).abs() <= 1e-9 * to_center.hypot() * tangent.hypot(), "{i} {t}: {c:?}");
                checked += 1;
            }
        }
    }
    assert!(checked >= 8, "{checked}");
}

#[test]
fn a_radius_fills_the_corner_between_two_lines() {
    let first = line((0.0, 0.0), (100.0, 0.0));
    let found = circles_of_radius(first, &[line((100.0, 0.0), (100.0, 100.0))], 10.0);
    // Outside the corner a circle tangent to the first line reaches the second only at the shared
    // corner, where it touches both at one point: left out.
    assert_eq!(found.len(), 1, "{found:?}");
    let c = found[0];
    assert!(near(c.circle.center, 90.0, 10.0, 1e-6) && c.circle.radius == 10.0, "{c:?}");
    assert!(near(c.first, 90.0, 0.0, 1e-6) && near(c.second, 100.0, 10.0, 1e-6), "{c:?}");
}

#[test]
fn lines_twice_the_radius_apart_give_circles_all_along() {
    let found = circles_of_radius(line((0.0, 0.0), (100.0, 0.0)), &[line((0.0, 40.0), (100.0, 40.0))], 20.0);
    assert!(found.len() > 100, "{}", found.len());
    assert!(found.iter().all(|c| (c.circle.center.y - 20.0).abs() < 1e-9 && c.circle.radius == 20.0), "{found:?}");
    let xs: Vec<f64> = found.iter().map(|c| c.circle.center.x).collect();
    assert!(xs.iter().any(|x| *x < 1.0) && xs.iter().any(|x| *x > 99.0), "from end to end: {xs:?}");
    // At a radius of 1,000,000 pt the places tried along the line, 0.2 pt apart, stay apart.
    let found = circles_of_radius(line((0.0, 0.0), (100.0, 0.0)), &[line((0.0, 2e6), (100.0, 2e6))], 1e6);
    let xs: Vec<f64> = found.iter().map(|c| c.circle.center.x).collect();
    let widest = xs.windows(2).map(|w| w[1] - w[0]).fold(0.0, f64::max);
    assert!(xs.iter().any(|x| *x < 1.0) && xs.iter().any(|x| *x > 99.0) && widest < 1.0, "from end to end, none merged: {xs:?}");
}

#[test]
fn a_circle_that_touches_a_curve_between_the_places_tried_is_found() {
    // A ring of radius 20 about (50.3, 60): a circle of radius 20 on the line y = 0 reaches it only
    // at x = 50.3, between the places tried along the line, without the distance changing sign.
    let ring = circle(50.3, 60.0, 20.0);
    let segs: Vec<CubicBez> = (0..4).map(|i| ring.subpaths[0].segment(i)).collect();
    let found = circles_of_radius(line((0.0, 0.0), (100.0, 0.0)), &segs, 20.0);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(near(found[0].circle.center, 50.3, 20.0, 1e-3) && near(found[0].second, 50.3, 40.0, 1e-3), "{found:?}");
}

#[test]
fn a_touch_half_way_between_two_places_tried_is_found() {
    // A point half way between the places tried at x = 50 and x = 50.1953125: the two samples
    // either side are equally far from a touch.
    let dot = line((50.097_656_25, 40.0), (50.097_656_25, 40.0));
    let found = circles_of_radius(line((0.0, 0.0), (100.0, 0.0)), &[dot], 20.0);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(near(found[0].circle.center, 50.097_656_25, 20.0, 1e-6), "{found:?}");
}

#[test]
fn a_touch_between_the_end_and_the_next_place_tried_is_found() {
    for x in [0.05, 99.95] {
        let found = circles_of_radius(line((0.0, 0.0), (100.0, 0.0)), &[line((x, 40.0), (x, 40.0))], 20.0);
        assert_eq!(found.len(), 1, "{x}: {found:?}");
        assert!(near(found[0].circle.center, x, 20.0, 1e-6), "{x}: {found:?}");
    }
}

#[test]
fn two_circles_between_the_same_two_places_tried_are_both_found() {
    // A point a hair less than twice the radius from the line: two circles, about 0.02 pt either
    // side of x = 50.05, both between the places tried at 50 and 50.1953125.
    let (x, y) = (50.05, 39.99999);
    let found = circles_of_radius(line((0.0, 0.0), (100.0, 0.0)), &[line((x, y), (x, y))], 20.0);
    assert_eq!(found.len(), 2, "{found:?}");
    let dx = (400.0 - (y - 20.0) * (y - 20.0)).sqrt();
    assert!(near(found[0].circle.center, x - dx, 20.0, 1e-6) && near(found[1].circle.center, x + dx, 20.0, 1e-6), "{found:?}");
}

#[test]
fn a_tiny_circle_between_long_lines_close_together_is_found() {
    let found = circles_through(line((0.0, 0.0), (100.0, 0.0)), 0.5, &[line((0.0, 1e-4), (100.0, 1e-4))]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!((found[0].circle.radius - 5e-5).abs() < 1e-12 && near(found[0].circle.center, 50.0, 5e-5, 1e-12), "{found:?}");
    // A short line near a very long one: the long one's size does not hide the gap.
    let found = circles_through(line((0.0, 0.0), (100.0, 0.0)), 0.5, &[line((-5e9, 1e-3), (5e9, 1e-3))]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!((found[0].circle.radius - 5e-4).abs() < 1e-9, "{found:?}");
    // Lines 1e-5 pt apart moved 1e9 pt to the right.
    let found = circles_through(line((1e9, 0.0), (1e9 + 100.0, 0.0)), 0.5, &[line((1e9, 1e-5), (1e9 + 100.0, 1e-5))]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!((found[0].circle.radius - 5e-6).abs() < 1e-9, "{found:?}");
}

#[test]
fn long_segments_far_apart_give_no_circle_of_a_small_radius() {
    // Lines 1e10 pt long and 1450 pt apart: a circle of radius 500 on one stays 950 pt from the other.
    let found = circles_of_radius(line((0.0, 0.0), (1e10, 0.0)), &[line((0.0, 1450.0), (1e10, 1450.0))], 500.0);
    assert!(found.is_empty(), "{:?}", found.first());
    // A radius of 1,000,000 pt and lines 0.05 pt more than twice that apart: no touch.
    let found = circles_of_radius(line((0.0, 0.0), (100.0, 0.0)), &[line((0.0, 2_000_000.05), (100.0, 2_000_000.05))], 1e6);
    assert!(found.is_empty(), "{:?}", found.first());
}

#[test]
fn no_circle_when_the_radius_is_too_small_or_the_point_is_on_the_second() {
    let (first, other) = (line((0.0, 0.0), (100.0, 0.0)), line((0.0, 40.0), (100.0, 40.0)));
    assert!(circles_of_radius(first, &[other], 5.0).is_empty());
    for r in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(circles_of_radius(first, &[other], r).is_empty(), "{r}");
    }
    assert!(circles_through(first, 1.0, &[line((100.0, 0.0), (100.0, 100.0))]).is_empty(), "the point is on the second");
    assert!(circles_through(first, 0.5, &[]).is_empty());
    assert!(circles_through(first, f64::NAN, &[other]).is_empty());
    let dot = line((5.0, 5.0), (5.0, 5.0));
    assert!(circles_through(dot, 0.5, &[other]).is_empty(), "no direction");
}
