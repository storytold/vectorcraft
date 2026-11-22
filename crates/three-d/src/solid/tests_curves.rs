use vectorcraft_geom::{BezPath, CubicBez, FillRule, PathData, Point, Rect};

use super::build::Turn;
use super::*;

fn shape(b: &BezPath) -> (PathData, Rect) {
    let p = PathData::from_bezpath(b);
    let r = p.bounds().unwrap();
    (p, r)
}

/// Straight on, without perspective.
fn front() -> View {
    View { rotation_x: 0.0, rotation_y: 0.0, rotation_z: 0.0, perspective: 0.0, ..View::default() }
}

/// A circle of four cubics about `c`.
fn circle(c: Point, r: f64) -> BezPath {
    let k = 0.552_284_749_830_793_6 * r;
    let mut b = BezPath::new();
    b.move_to((c.x + r, c.y));
    b.curve_to((c.x + r, c.y + k), (c.x + k, c.y + r), (c.x, c.y + r));
    b.curve_to((c.x - k, c.y + r), (c.x - r, c.y + k), (c.x - r, c.y));
    b.curve_to((c.x - r, c.y - k), (c.x - k, c.y - r), (c.x, c.y - r));
    b.curve_to((c.x + k, c.y - r), (c.x + r, c.y - k), (c.x + r, c.y));
    b.close_path();
    b
}

/// A "D" with its counter: straight stems, round bowls, and corners where they meet.
fn letter_d() -> BezPath {
    let mut b = BezPath::new();
    b.move_to((0.0, 0.0));
    b.line_to((40.0, 0.0));
    b.curve_to((95.0, 0.0), (95.0, 100.0), (40.0, 100.0));
    b.line_to((0.0, 100.0));
    b.close_path();
    b.move_to((20.0, 20.0));
    b.line_to((20.0, 80.0));
    b.line_to((40.0, 80.0));
    b.curve_to((70.0, 80.0), (70.0, 20.0), (40.0, 20.0));
    b.close_path();
    b
}

fn square(x: f64, y: f64, s: f64) -> BezPath {
    let mut b = BezPath::new();
    b.move_to((x, y));
    b.line_to((x + s, y));
    b.line_to((x + s, y + s));
    b.line_to((x, y + s));
    b.close_path();
    b
}

/// A path's curved segments, segments and anchors.
fn anatomy(p: &PathData) -> [usize; 3] {
    let mut n = [0; 3];
    for sp in &p.subpaths {
        let count = if sp.closed { sp.anchors.len() } else { sp.anchors.len().saturating_sub(1) };
        n[0] += (0..count).filter(|i| !sp.segment_is_line(*i)).count();
        n[1] += count;
        n[2] += sp.anchors.len();
    }
    n
}

/// A path's curved segments.
fn curves(p: &PathData) -> Vec<CubicBez> {
    let mut out = vec![];
    for sp in &p.subpaths {
        let count = if sp.closed { sp.anchors.len() } else { sp.anchors.len().saturating_sub(1) };
        out.extend((0..count).filter(|i| !sp.segment_is_line(*i)).map(|i| sp.segment(i)));
    }
    out
}

/// `b` flattened to within 1e-6 pt, each point moved by `map`.
fn fine(b: &BezPath, map: impl Fn(Point) -> Point) -> Vec<Vec<Point>> {
    let mut out: Vec<Vec<Point>> = vec![];
    vectorcraft_geom::kurbo::flatten(b, 1e-6, |el| match el {
        vectorcraft_geom::kurbo::PathEl::MoveTo(p) => out.push(vec![map(p)]),
        vectorcraft_geom::kurbo::PathEl::LineTo(p) => out.last_mut().into_iter().for_each(|r| r.push(map(p))),
        _ => {}
    });
    out
}

/// How far `c` lies from the polygons `target` at most, at 33 points along it.
fn farthest(c: &CubicBez, target: &[Vec<Point>]) -> f64 {
    (0..=32).map(|k| edge_distance(target, c.eval(f64::from(k) / 32.0))).fold(0.0, f64::max)
}

/// A rounded bevel on both sides steps through many rings between the caps. Every edge along the
/// art's curves, between two bevel steps or two wall slices as well as at a cap, is a cubic: each
/// side facet that follows a curve has both its edges curved, four anchors, and lies within the
/// sampling tolerance of its own polygon. Two visible facets on either side of one ring draw the
/// edge they share as the same curve.
#[test]
fn interior_bevel_edges_keep_the_curves_of_the_art() {
    for art in [circle(Point::new(50.0, 50.0), 40.0), letter_d()] {
        let (p, b) = shape(&art);
        let bevel = Bevel { shape: BevelShape::Rounded, inside: true, width: 4.0, height: 4.0, both_sides: true, ..Bevel::default() };
        let o = Extrude { depth: 50.0, bevel: Some(bevel), ..Extrude::default() };
        let f = extrude(&p, FillRule::EvenOdd, b, o, View { rotation_x: 20.0, rotation_y: 30.0, ..front() }).unwrap();
        let sides: Vec<&Facet> = f.iter().filter(|f| f.side && f.outline.is_some()).collect();
        assert!(sides.len() > 40, "{}", sides.len());
        for side in &sides {
            let path = side.path();
            assert_eq!(anatomy(&path), [2, 4, 4], "{path:?}");
            for c in curves(&path) {
                assert!(farthest(&c, &side.contours) <= 2.0 * 0.05, "{c:?}");
            }
        }
        let all: Vec<(usize, CubicBez)> = sides.iter().enumerate().flat_map(|(k, f)| curves(&f.path()).into_iter().map(move |c| (k, c))).collect();
        let mut shared = 0;
        for (k, c) in &all {
            for (_, d) in all.iter().filter(|(j, d)| j != k && d.p0.distance(c.p3) < 1e-9 && d.p3.distance(c.p0) < 1e-9) {
                shared += 1;
                let far = (0..=16).map(|u| c.eval(f64::from(u) / 16.0).distance(d.eval(1.0 - f64::from(u) / 16.0))).fold(0.0, f64::max);
                assert!(far <= 1e-6, "{far}");
            }
        }
        assert!(shared > 20, "{shared}");
    }
}

/// A twist cuts each edge of a wall into pieces at points of the art's curve: every piece keeps
/// both its edges curved, and the pieces at the front draw the cap's edge, within 0.001 pt of the
/// cap's curves without perspective and twice that with it.
#[test]
fn a_twisted_wall_keeps_curves_where_a_twist_cuts_it() {
    let art = circle(Point::new(50.0, 50.0), 40.0);
    let (p, b) = shape(&art);
    // Shallow, so that each slice of the wall is narrower than its edges are long.
    let o = Extrude { depth: 2.0, twist: 30.0, ..Extrude::default() };
    let turned = View { rotation_x: 20.0, rotation_y: 30.0, ..front() };
    for (view, tolerance) in [(turned, outline::CURVE_TOLERANCE), (View { perspective: 60.0, ..turned }, 2.0 * outline::CURVE_TOLERANCE)] {
        let f = extrude(&p, FillRule::EvenOdd, b, o, view).unwrap();
        let cap = f.iter().find(|f| f.depth == f64::MAX).unwrap();
        let corners: Vec<Point> = cap.contours.iter().flatten().copied().collect();
        let cap = fine(&cap.path().to_bezpath(), |q| q);
        let sides: Vec<&Facet> = f.iter().filter(|f| f.side).collect();
        assert!(sides.len() > 64, "{}", sides.len());
        let (mut at_cap, mut cut) = (0, 0);
        for side in &sides {
            let path = side.path();
            assert_eq!(anatomy(&path), [2, 4, 4], "{path:?}");
            // The cap's curves are fitted within the curve tolerance in perspective.
            for c in curves(&path).iter().filter(|c| edge_distance(&cap, c.p0) <= tolerance && edge_distance(&cap, c.p3) <= tolerance) {
                at_cap += 1;
                // A piece that starts or ends where the twist cuts the edge, between two of the
                // cap's sampled points.
                if [c.p0, c.p3].iter().any(|q| corners.iter().all(|v| v.distance(*q) > 1e-6)) {
                    cut += 1;
                }
                let far = farthest(c, &cap);
                assert!(far <= tolerance + 1e-6, "{far}");
            }
        }
        assert!(at_cap > 10 && cut > 5, "{at_cap} at the cap, {cut} of them cut");
    }
}

/// A half disc: a straight diameter and a round arc, meeting it at right angles.
fn half_disc() -> BezPath {
    let k = 0.552_284_749_830_793_6 * 50.0;
    let mut b = BezPath::new();
    b.move_to((0.0, 50.0));
    b.line_to((100.0, 50.0));
    b.curve_to((100.0, 50.0 - k), (50.0 + k, 0.0), (50.0, 0.0));
    b.curve_to((50.0 - k, 0.0), (0.0, 50.0 - k), (0.0, 50.0));
    b.close_path();
    b
}

/// With a bevel, the cap is inset, and its corners where the arc meets the diameter at a right
/// angle lie off the curve's inset by the miter. A twist cuts the band next to the cap into pieces:
/// the pieces that start or end at one of the cap's corners draw the cap's edge, within the cap's
/// sampling tolerance of its curves, with no gap where they meet it.
#[test]
fn twisted_bevel_pieces_meet_the_cap_at_sharp_joins() {
    let (p, b) = shape(&half_disc());
    let bevel = Bevel { inside: true, width: 4.0, height: 1.0, ..Bevel::default() };
    let o = Extrude { depth: 4.0, twist: 30.0, bevel: Some(bevel), ..Extrude::default() };
    let turned = View { rotation_x: 20.0, rotation_y: 30.0, ..front() };
    for view in [turned, View { perspective: 60.0, ..turned }] {
        let f = extrude(&p, FillRule::EvenOdd, b, o, view).unwrap();
        let cap = f.iter().find(|f| f.depth == f64::MAX).unwrap();
        let corners: Vec<Point> = cap.contours.iter().flatten().copied().collect();
        let cap = fine(&cap.path().to_bezpath(), |q| q);
        let mut pieces = 0;
        for side in f.iter().filter(|f| f.side) {
            for c in curves(&side.path()) {
                let at_corner = |q: Point| corners.iter().any(|v| v.distance(q) < 1e-6);
                // One end at a corner of the cap, the other where the twist cuts the edge.
                if at_corner(c.p0) == at_corner(c.p3) {
                    continue;
                }
                pieces += 1;
                let far = farthest(&c, &cap);
                assert!(far <= 2.0 * 0.05 + 1e-6, "{far}: {c:?}");
            }
        }
        assert!(pieces > 10, "{pieces}");
    }
}

/// Inflate keeps the art's curves where its faces meet the outline: the flat back is the art
/// turned, one cubic for each of its curves within 0.001 pt; every side facet has both its edges
/// curved; and the front's cells along the outline are curved there, within the sampling
/// tolerance of the art.
#[test]
fn inflate_keeps_the_curves_of_its_outline() {
    let art = circle(Point::new(50.0, 50.0), 40.0);
    let (p, b) = shape(&art);
    let o = Inflate { depth: 20.0, twist: 30.0, ..Inflate::default() };
    let behind = View { rotation_y: 180.0, ..front() };
    let f = inflate(&p, FillRule::EvenOdd, b, o, behind).unwrap();
    let back = f.iter().find(|f| f.depth == f64::MAX).unwrap().path();
    assert_eq!(anatomy(&back), [4, 4, 4], "{back:?}");
    let camera = Camera::new(behind, b, 1.0);
    let turn = Turn::new(b, o.twist, o.taper, 1.0);
    let target = fine(&art, |q| camera.project(camera.place(turn.apply(q), -o.depth / 2.0)));
    for c in curves(&back) {
        assert!(farthest(&c, &target) <= outline::CURVE_TOLERANCE + 1e-6, "{c:?}");
    }

    for (o, view) in [
        (o, View { rotation_x: 20.0, rotation_y: 30.0, ..front() }),
        (Inflate { both_sides: true, map_surface: true, ..o }, View { rotation_x: 20.0, rotation_y: 30.0, perspective: 60.0, ..front() }),
    ] {
        let f = inflate(&p, FillRule::EvenOdd, b, o, view).unwrap();
        let sides: Vec<&Facet> = f.iter().filter(|f| f.side).collect();
        assert!(!sides.is_empty());
        assert!(sides.iter().all(|f| anatomy(&f.path())[..2] == [2, 4]), "a side facet with a straight edge along the circle");
        let rim: Vec<&Facet> = f.iter().filter(|f| !f.side && f.outline.is_some()).collect();
        assert!(rim.len() > 20, "{}", rim.len());
    }

    // Seen straight on, the front's curved edges are the art's circle.
    let f = inflate(&p, FillRule::EvenOdd, b, Inflate::default(), front()).unwrap();
    let rim: Vec<CubicBez> = f.iter().filter(|f| f.outline.is_some()).flat_map(|f| curves(&f.path())).collect();
    assert!(rim.len() > 20, "{}", rim.len());
    let circle = fine(&art, |q| q);
    for c in &rim {
        assert!(farthest(c, &circle) <= 0.05 + 1e-6, "{c:?}");
    }
}

/// A shape of straight edges gets no curves; a tiny circle and a twisted, tapered Inflate keep
/// theirs without points that are not finite.
#[test]
fn straight_tiny_and_twisted_solids_keep_their_curves_bounded() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let bevel = Bevel { shape: BevelShape::Rounded, inside: true, width: 4.0, height: 4.0, both_sides: true, ..Bevel::default() };
    let view = View { rotation_x: 20.0, rotation_y: 30.0, ..front() };
    let f = extrude(&p, FillRule::EvenOdd, b, Extrude { depth: 50.0, twist: 45.0, bevel: Some(bevel), ..Extrude::default() }, view).unwrap();
    assert!(f.iter().all(|f| f.outline.is_none()));
    let f = inflate(&p, FillRule::EvenOdd, b, Inflate { depth: 20.0, ..Inflate::default() }, view).unwrap();
    assert!(f.iter().all(|f| f.outline.is_none()));

    let tiny = PathData::from_bezpath(&circle(Point::new(1.0, 1.0), 0.01));
    let b = tiny.bounds().unwrap();
    for f in [
        extrude(
            &tiny,
            FillRule::NonZero,
            b,
            Extrude { depth: 0.02, twist: 90.0, bevel: Some(Bevel { width: 0.002, height: 0.002, ..bevel }), ..Extrude::default() },
            view,
        ),
        inflate(&tiny, FillRule::NonZero, b, Inflate { depth: 0.02, twist: 360.0, taper: 0.01, both_sides: true, ..Inflate::default() }, view),
    ] {
        let f = f.unwrap();
        assert!(f.iter().any(|f| !curves(&f.path()).is_empty()), "no curve on a tiny circle");
        for f in f {
            for c in curves(&f.path()) {
                assert!([c.p0, c.p1, c.p2, c.p3].iter().all(|q| q.x.is_finite() && q.y.is_finite()), "{c:?}");
            }
        }
    }
}
