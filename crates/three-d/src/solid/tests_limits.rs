use vectorcraft_geom::{Anchor, SubPath};

use super::*;

const SHAPE: &str = "The shape is too complex for a 3D effect";
const SURFACE: &str = "The 3D surface is too complex; simplify the shape";

/// A closed ring of `count` curved petals around a circle of `radius`: each curve leaves its anchor
/// outward, `lift` points out, and comes back to the next anchor, so a larger lift asks for more
/// pieces per curve.
fn petals(count: usize, radius: f64, lift: f64) -> PathData {
    let chord = std::f64::consts::TAU * radius / count as f64;
    let anchors = (0..count)
        .map(|i| {
            let (s, c) = (std::f64::consts::TAU * i as f64 / count as f64).sin_cos();
            let p = Point::new(radius * 2.0 + radius * c, radius * 2.0 + radius * s);
            let (out, along) = (Vec2::new(c, s) * lift, Vec2::new(-s, c) * (chord / 3.0));
            Anchor::with_handles(p, p + out - along, p + out + along)
        })
        .collect();
    PathData::new(vec![SubPath::new(anchors, true)])
}

/// A closed polygon of `count` straight edges around a circle.
fn polygon(count: usize, radius: f64) -> PathData {
    let anchors = (0..count)
        .map(|i| {
            let (s, c) = (std::f64::consts::TAU * i as f64 / count as f64).sin_cos();
            Anchor::corner(Point::new(radius * 2.0 + radius * c, radius * 2.0 + radius * s))
        })
        .collect();
    PathData::new(vec![SubPath::new(anchors, true)])
}

/// Turned so that the walls and bevels show.
fn turned() -> View {
    View { rotation_x: -18.0, rotation_y: -26.0, rotation_z: 8.0, ..View::default() }
}

fn size(path: &PathData) -> f64 {
    let b = path.bounds().unwrap();
    b.width().max(b.height())
}

/// How many points the outline samples to, each curve in at most `most` pieces.
fn points(path: &PathData, most: u32) -> usize {
    rings::sample(path, size(path), usize::MAX, most).unwrap().iter().map(Vec::len).sum()
}

fn built(facets: &[Facet]) -> bool {
    !facets.is_empty()
        && facets.len() <= MAX_SOLID_FACES
        && facets.iter().flat_map(|f| f.contours.iter().flatten()).all(|p| p.x.is_finite() && p.y.is_finite())
}

#[test]
fn two_thousand_curves_extrude_with_a_bevel_and_inflate() {
    let path = petals(2000, 500.0, 4.0);
    let b = path.bounds().unwrap();
    // Every curve takes several pieces: far more points than 2,048, all of them kept.
    let sampled = points(&path, rings::CURVE_PIECES);
    assert!(sampled >= 3 * 2000, "{sampled}");
    assert_eq!(rings::curve_pieces(&path, size(&path), 2, 2), Ok(rings::CURVE_PIECES));
    let bevel = Extrude { bevel: Some(Bevel::default()), ..Extrude::default() };
    let extruded = extrude(&path, FillRule::NonZero, b, bevel, turned()).unwrap();
    assert!(built(&extruded) && extruded.len() > sampled, "{}", extruded.len());
    let inflated = inflate(&path, FillRule::NonZero, b, Inflate::default(), turned()).unwrap();
    assert!(built(&inflated), "{}", inflated.len());
}

#[test]
fn an_outline_at_the_point_limit_is_taken_and_one_past_it_is_refused_before_sampling() {
    let at = polygon(MAX_SOLID_POINTS, 20_000.0);
    assert_eq!(rings::curve_pieces(&at, size(&at), 1, 2), Ok(rings::CURVE_PIECES));
    assert!(built(&extrude(&at, FillRule::NonZero, at.bounds().unwrap(), Extrude::default(), turned()).unwrap()));
    let past = polygon(MAX_SOLID_POINTS + 1, 20_000.0);
    assert_eq!(rings::curve_pieces(&past, size(&past), 1, 2), Err(SHAPE.to_string()));
    assert_eq!(extrude(&past, FillRule::NonZero, past.bounds().unwrap(), Extrude::default(), turned()), Err(SHAPE.to_string()));
    assert_eq!(inflate(&past, FillRule::NonZero, past.bounds().unwrap(), Inflate::default(), turned()), Err(SHAPE.to_string()));
}

#[test]
fn faces_at_the_limit_are_taken_and_one_past_it_is_refused_before_sampling() {
    // 40,960 straight edges in four bands are exactly the face limit.
    let path = polygon(MAX_SOLID_FACES / 4, 20_000.0);
    assert_eq!(rings::curve_pieces(&path, size(&path), 4, 0), Ok(rings::CURVE_PIECES));
    assert_eq!(rings::curve_pieces(&path, size(&path), 4, 1), Err(SURFACE.to_string()));
    // A rounded bevel gives 20,000 edges nine bands each, past the limit.
    let path = polygon(20_000, 20_000.0);
    let rounded = Extrude { bevel: Some(Bevel { shape: BevelShape::Rounded, ..Bevel::default() }), ..Extrude::default() };
    assert_eq!(extrude(&path, FillRule::NonZero, path.bounds().unwrap(), rounded, turned()), Err(SURFACE.to_string()));
}

#[test]
fn curves_over_the_budget_take_fewer_pieces_down_to_four() {
    let path = petals(2000, 500.0, 20.0);
    let s = size(&path);
    assert_eq!(points(&path, rings::CURVE_PIECES), 6 * 2000, "six pieces a curve within the tolerance");
    // 16 faces an edge: 12,000 points are too many, 10,000 at five pieces a curve fit.
    assert_eq!(rings::curve_pieces(&path, s, 16, 0), Ok(5));
    assert_eq!(points(&path, 5), 5 * 2000);
    // At four pieces a curve, 21 faces an edge are still too many.
    assert_eq!(rings::curve_pieces(&path, s, 21, 0), Err(SURFACE.to_string()));
    // Twisted 96°, the walls are 16 slices: built with five pieces a curve.
    let b = path.bounds().unwrap();
    let twisted = extrude(&path, FillRule::NonZero, b, Extrude { twist: 96.0, hidden: true, ..Extrude::default() }, turned()).unwrap();
    assert!(built(&twisted) && twisted.len() > MAX_FACES, "{}", twisted.len());
    // Twisted 360°, 60 slices are too many even at four pieces a curve.
    assert_eq!(extrude(&path, FillRule::NonZero, b, Extrude { twist: 360.0, ..Extrude::default() }, turned()), Err(SURFACE.to_string()));
}

#[test]
fn five_and_ten_thousand_curves_build_at_the_defaults() {
    let five = petals(5000, 1250.0, 2.0);
    assert!(built(&inflate(&five, FillRule::NonZero, five.bounds().unwrap(), Inflate::default(), turned()).unwrap()));
    assert!(built(&extrude(&five, FillRule::NonZero, five.bounds().unwrap(), Extrude::default(), turned()).unwrap()));
    let ten = petals(10_000, 2500.0, 4.0);
    let b = ten.bounds().unwrap();
    assert!(points(&ten, rings::CURVE_PIECES) > 10_000);
    assert!(built(&extrude(&ten, FillRule::NonZero, b, Extrude::default(), turned()).unwrap()));
    let deep = Extrude { twist: 360.0, bevel: Some(Bevel { shape: BevelShape::Rounded, repeat: 4, ..Bevel::default() }), ..Extrude::default() };
    assert_eq!(extrude(&ten, FillRule::NonZero, b, deep, turned()), Err(SURFACE.to_string()));
}

#[test]
fn many_empty_contours_and_invalid_handles_are_refused_before_sampling() {
    // Contours of one anchor and of none draw nothing, yet each is counted.
    let lone = |i: usize| SubPath::new(if i.is_multiple_of(2) { vec![] } else { vec![Anchor::corner(Point::new(1.0, i as f64))] }, false);
    let square = SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)], true);
    let at = PathData::new((0..MAX_SOLID_POINTS - 1).map(lone).chain([square.clone()]).collect());
    assert_eq!(rings::curve_pieces(&at, 10.0, 1, 2), Ok(rings::CURVE_PIECES));
    let past = PathData::new((0..MAX_SOLID_POINTS).map(lone).chain([square.clone()]).collect());
    assert_eq!(rings::curve_pieces(&past, 10.0, 1, 2), Err(SHAPE.to_string()));
    let b = Rect::new(0.0, 0.0, 10.0, 10.0);
    assert_eq!(extrude(&past, FillRule::NonZero, b, Extrude::default(), turned()), Err(SHAPE.to_string()));
    let mut bad = square;
    bad.anchors[1].h_out = Point::new(f64::NAN, 0.0);
    let bad = PathData::new(vec![bad]);
    assert_eq!(rings::curve_pieces(&bad, 10.0, 1, 2), Err("The shape contains invalid coordinates".to_string()));
    assert_eq!(inflate(&bad, FillRule::NonZero, b, Inflate::default(), turned()), Err("The shape contains invalid coordinates".to_string()));
}
