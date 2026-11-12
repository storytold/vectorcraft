use vectorcraft_geom::{BezPath, FillRule, PathData, Point, Rect};

use super::super::{Extrude, Facet, PlanarFace, View, extrude, plane};
use super::{FaceMap, extrude_face_map, plane_face_map};

fn square() -> (PathData, Rect) {
    let mut b = BezPath::new();
    b.move_to((10.0, 20.0));
    b.line_to((70.0, 20.0));
    b.line_to((70.0, 60.0));
    b.line_to((10.0, 60.0));
    b.close_path();
    let p = PathData::from_bezpath(&b);
    let r = p.bounds().unwrap();
    (p, r)
}

fn turned(perspective: f64) -> View {
    View { rotation_x: -30.0, rotation_y: 40.0, rotation_z: 10.0, perspective, ..View::default() }
}

fn corners(r: Rect) -> [Point; 4] {
    [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)]
}

/// Every corner of `face` is where the map takes one of the art's corners.
fn lands_on(face: &Facet, map: &FaceMap, art: [Point; 4]) {
    for c in face.contours.iter().flatten() {
        let near = art.iter().filter_map(|p| map.to_page(*p)).map(|q| q.distance(*c)).fold(f64::INFINITY, f64::min);
        assert!(near < 1e-6, "facet corner {c:?} is {near} from every mapped corner");
    }
}

fn end(facets: &[Facet], end: PlanarFace) -> Option<&Facet> {
    facets.iter().find(|f| f.end == Some(end))
}

#[test]
fn the_front_cap_is_where_the_map_takes_the_art() {
    let (path, bounds) = square();
    for perspective in [0.0, 60.0, 140.0] {
        let view = turned(perspective);
        let facets = extrude(&path, FillRule::NonZero, bounds, Extrude::default(), view).unwrap();
        let map = extrude_face_map(bounds, Extrude::default(), view, PlanarFace::Front).unwrap();
        let front = end(&facets, PlanarFace::Front).unwrap();
        assert!(map.visible && !front.away);
        lands_on(front, &map, corners(bounds));
        assert_eq!(map.is_affine(), perspective == 0.0);
    }
}

#[test]
fn the_back_cap_follows_the_twist_and_taper() {
    let (path, bounds) = square();
    let options = Extrude { twist: 35.0, taper: 0.5, ..Extrude::default() };
    // Seen from behind, the back cap faces the viewer and the front does not.
    let view = View { rotation_y: 160.0, perspective: 50.0, ..turned(0.0) };
    let facets = extrude(&path, FillRule::NonZero, bounds, options, view).unwrap();
    let back = end(&facets, PlanarFace::Back).unwrap();
    let map = extrude_face_map(bounds, options, view, PlanarFace::Back).unwrap();
    assert!(map.visible);
    assert!(!extrude_face_map(bounds, options, view, PlanarFace::Front).unwrap().visible);
    assert!(end(&facets, PlanarFace::Front).is_none_or(|f| f.away));
    lands_on(back, &map, corners(bounds));
}

#[test]
fn a_plane_has_only_a_front_and_a_hollow_solid_none() {
    let (path, bounds) = square();
    let view = turned(80.0);
    let facets = plane(&path, FillRule::NonZero, bounds, view).unwrap();
    let map = plane_face_map(bounds, view).unwrap();
    lands_on(end(&facets, PlanarFace::Front).unwrap(), &map, corners(bounds));
    let flat = Extrude { depth: 0.0, ..Extrude::default() };
    assert_eq!(extrude_face_map(bounds, flat, view, PlanarFace::Front), Some(map));
    assert_eq!(extrude_face_map(bounds, flat, view, PlanarFace::Back), None);
    let hollow = Extrude { cap: false, ..Extrude::default() };
    assert_eq!(extrude_face_map(bounds, hollow, view, PlanarFace::Front), None);
    let facets = extrude(&path, FillRule::NonZero, bounds, hollow, view).unwrap();
    assert!(facets.iter().all(|f| f.end.is_none()));
}

#[test]
fn page_points_map_back_to_the_art() {
    let (_, bounds) = square();
    for perspective in [0.0, 120.0] {
        let map = extrude_face_map(bounds, Extrude::default(), turned(perspective), PlanarFace::Front).unwrap();
        for p in [Point::new(10.0, 20.0), Point::new(45.0, 33.0), Point::new(-200.0, 400.0)] {
            let q = map.to_page(p).unwrap();
            let back = map.to_art(q).unwrap();
            assert!(back.distance(p) < 1e-6, "{p:?} came back as {back:?}");
        }
    }
}

#[test]
fn the_local_affine_map_matches_the_face_near_its_point() {
    let (_, bounds) = square();
    let flat = extrude_face_map(bounds, Extrude::default(), turned(0.0), PlanarFace::Front).unwrap();
    let m = flat.local_affine(Point::new(40.0, 40.0)).unwrap();
    for p in corners(bounds) {
        assert!((m * p).distance(flat.to_page(p).unwrap()) < 1e-9);
    }
    let deep = extrude_face_map(bounds, Extrude::default(), turned(120.0), PlanarFace::Front).unwrap();
    let at = Point::new(40.0, 40.0);
    let [a, b, c, d, ..] = deep.local_affine(at).unwrap().as_coeffs();
    let h = 1e-4;
    let slope = |dx: f64, dy: f64| {
        let (p, q) = (deep.to_page(Point::new(at.x + dx, at.y + dy)).unwrap(), deep.to_page(Point::new(at.x - dx, at.y - dy)).unwrap());
        ((p.x - q.x) / (2.0 * h), (p.y - q.y) / (2.0 * h))
    };
    let ((ax, bx), (cy, dy)) = (slope(h, 0.0), slope(0.0, h));
    for (got, want) in [(a, ax), (b, bx), (c, cy), (d, dy)] {
        assert!((got - want).abs() < 1e-6, "{got} against {want}");
    }
    assert!((deep.local_affine(at).unwrap() * at).distance(deep.to_page(at).unwrap()) < 1e-9);
}

#[test]
fn a_point_behind_the_eye_has_no_page_point() {
    let (_, bounds) = square();
    // Seen almost edge-on at the widest perspective, far points of the plane pass behind the eye.
    let view = View { rotation_x: -89.0, perspective: 160.0, ..turned(0.0) };
    let map = plane_face_map(bounds, view).unwrap();
    assert!(map.to_page(Point::new(40.0, -1e6)).is_none() || map.to_page(Point::new(40.0, 1e6)).is_none());
}

#[test]
fn the_mapped_part_of_a_rectangle_stops_before_the_eye() {
    let (_, bounds) = square();
    let view = View { rotation_x: -89.0, perspective: 160.0, ..turned(0.0) };
    let map = plane_face_map(bounds, view).unwrap();
    let r = Rect::new(-1e6, -1e6, 1e6, 1e6);
    let part = map.mapped_part(r).unwrap();
    // Some of the rectangle passes behind the eye, and every corner of what is left maps.
    assert!((3..=5).contains(&part.len()));
    assert!(part.iter().all(|p| map.to_page(*p).is_some()), "{part:?}");
    assert!(corners(r).iter().any(|p| !part.contains(p)));
    // Without perspective, all of it maps.
    let flat = plane_face_map(bounds, View { perspective: 0.0, ..view }).unwrap();
    assert_eq!(flat.mapped_part(r).map(|p| p.len()), Some(4));
}
