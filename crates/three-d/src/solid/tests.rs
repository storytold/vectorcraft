use vectorcraft_geom::{BezPath, FillRule, PathData, Point, Rect};

use super::*;

fn square(x: f64, y: f64, s: f64) -> BezPath {
    let mut b = BezPath::new();
    b.move_to((x, y));
    b.line_to((x + s, y));
    b.line_to((x + s, y + s));
    b.line_to((x, y + s));
    b.close_path();
    b
}

fn shape(b: &BezPath) -> (PathData, Rect) {
    let p = PathData::from_bezpath(b);
    let r = p.bounds().unwrap();
    (p, r)
}

/// Straight on, without perspective.
fn front() -> View {
    View { rotation_x: 0.0, rotation_y: 0.0, rotation_z: 0.0, perspective: 0.0, ..View::default() }
}

fn extent(f: &[Facet]) -> Rect {
    let pts: Vec<Point> = f.iter().flat_map(|f| f.contours.iter().flatten().copied()).collect();
    pts.iter().fold(Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p))
}

/// The area the facets cover: each facet's contours counted with their winding (holes
/// subtract), whichever way round the facet runs.
fn area(f: &[Facet]) -> f64 {
    f.iter().map(|f| f.contours.iter().map(|c| signed_area(c)).sum::<f64>().abs()).sum()
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn seen_straight_on_an_extrusion_is_its_front_face_exactly_where_the_art_is() {
    let (p, b) = shape(&square(10.0, 20.0, 100.0));
    let f = extrude(&p, FillRule::EvenOdd, b, Extrude::default(), front()).unwrap();
    assert_eq!(f.len(), 1, "the walls and back face away: only the front cap shows");
    assert_eq!(f[0].contours.len(), 1);
    assert_eq!(extent(&f), Rect::new(10.0, 20.0, 110.0, 120.0));
}

#[test]
fn turned_about_y_the_front_foreshortens_and_one_side_wall_shows() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let o = Extrude { depth: 40.0, ..Extrude::default() };
    let f = extrude(&p, FillRule::EvenOdd, b, o, View { rotation_y: 30.0, ..front() }).unwrap();
    // Orthographic: the silhouette is the front's width times cos 30° plus the depth times sin 30°.
    let w = extent(&f).width();
    assert!(close(w, 100.0 * 30f64.to_radians().cos() + 40.0 * 30f64.to_radians().sin(), 1e-6), "{w}");
    assert_eq!(f.len(), 2, "the front cap and one side wall");
    // Lit from the upper left by default: the cap and the wall are shaded differently.
    assert!(!close(f[0].brightness, f[1].brightness, 1e-6));
    // The nearest face is drawn last.
    assert!(f.windows(2).all(|w| w[0].depth <= w[1].depth));
}

#[test]
fn no_depth_is_the_flat_shape_and_no_cap_shows_the_inside() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let v = View { rotation_x: 20.0, rotation_y: 30.0, ..front() };
    let flat = extrude(&p, FillRule::EvenOdd, b, Extrude { depth: 0.0, ..Extrude::default() }, v).unwrap();
    assert_eq!(flat, plane(&p, FillRule::EvenOdd, b, v).unwrap());
    let solid = extrude(&p, FillRule::EvenOdd, b, Extrude::default(), v).unwrap();
    let hollow = extrude(&p, FillRule::EvenOdd, b, Extrude { cap: false, ..Extrude::default() }, v).unwrap();
    // Solid: front, right and top faces. Hollow: all four walls (two seen from inside), no caps.
    assert_eq!(solid.len(), 3);
    assert_eq!(hollow.len(), 4);
    assert!(hollow.iter().all(|f| f.contours.len() == 1 && f.contours[0].len() == 4));
}

#[test]
fn hidden_faces_draw_every_face_of_the_solid_behind_the_visible_ones() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let v = View { rotation_x: 20.0, rotation_y: 30.0, ..front() };
    let shown = extrude(&p, FillRule::EvenOdd, b, Extrude::default(), v).unwrap();
    let all = extrude(&p, FillRule::EvenOdd, b, Extrude { hidden: true, ..Extrude::default() }, v).unwrap();
    // A box: the three faces turned to the viewer, then the three turned away, drawn first.
    assert_eq!(shown.len(), 3);
    assert_eq!(all.len(), 6);
    assert!(all.windows(2).all(|w| w[0].depth <= w[1].depth));
    // The faces turned away keep their outward normals, so a light behind the solid lights them.
    assert!(all[..3].iter().all(|f| f.normal[2] < 0.0), "{:?}", all.iter().map(|f| f.normal).collect::<Vec<_>>());
    for f in shown.iter().chain(&all) {
        let [x, y, z] = f.normal;
        assert!(close(x.hypot(y).hypot(z), 1.0, 1e-9), "{:?}", f.normal);
    }
    // The visible faces keep their normals and their place at the front.
    for (front, kept) in shown.iter().rev().zip(all.iter().rev()) {
        assert_eq!(front.normal, kept.normal);
    }
}

#[test]
fn taper_shrinks_the_back_and_twist_turns_it() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let side = View { rotation_y: 90.0, ..front() };
    let o = Extrude { depth: 60.0, cap: false, ..Extrude::default() };
    let straight = extent(&extrude(&p, FillRule::EvenOdd, b, o, side).unwrap());
    assert!(close(straight.height(), 100.0, 1e-6) && close(straight.width(), 60.0, 1e-6));
    // Seen from the side, a 50 % taper keeps the front's full height and halves the back's.
    let tapered = extrude(&p, FillRule::EvenOdd, b, Extrude { taper: 0.5, ..o }, side).unwrap();
    let back: Vec<Point> = tapered.iter().flat_map(|f| f.contours.iter().flatten().copied()).filter(|q| close(q.x, straight.x0, 1e-6)).collect();
    let (lo, hi) = back.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), q| (lo.min(q.y), hi.max(q.y)));
    let at_front: Vec<Point> = tapered.iter().flat_map(|f| f.contours.iter().flatten().copied()).filter(|q| close(q.x, straight.x1, 1e-6)).collect();
    let (flo, fhi) = at_front.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), q| (lo.min(q.y), hi.max(q.y)));
    let (back_h, front_h) = (hi - lo, fhi - flo);
    assert!(close(front_h, 100.0, 1e-6) && close(back_h, 50.0, 1e-6), "front {front_h}, back {back_h}");
    // Seen from behind, a 45° twist shows the back cap turned into a diamond, drawn last, and the
    // corners of the walls nearer the front reaching past it.
    let back = View { rotation_y: 180.0, ..front() };
    let twisted = extrude(&p, FillRule::EvenOdd, b, Extrude { twist: 45.0, cap: true, ..o }, back).unwrap();
    let cap = twisted.last().unwrap();
    assert_eq!(cap.contours[0].len(), 4);
    assert!(close(extent(std::slice::from_ref(cap)).width(), 100.0 * 2f64.sqrt(), 1e-6), "{:?}", extent(&twisted));
    assert!(close(extent(&twisted).width(), 100.0 * 2f64.sqrt(), 1e-6), "{:?}", extent(&twisted));
    assert!(depth_at(cap, Point::new(94.0, 94.0)).is_none() && twisted.iter().any(|f| depth_at(f, Point::new(94.0, 94.0)).is_some()));
    // Its walls are cut into slices that follow the turn.
    let open = extrude(&p, FillRule::EvenOdd, b, Extrude { twist: 45.0, ..o }, View { rotation_x: -20.0, rotation_y: 30.0, ..front() }).unwrap();
    assert!(open.len() > 2 * 8, "{}", open.len());
}

#[test]
fn a_bevel_inside_shrinks_the_cap_and_outside_grows_the_walls() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let bevel = Bevel { width: 10.0, height: 10.0, ..Bevel::default() };
    let inside = extrude(&p, FillRule::EvenOdd, b, Extrude { bevel: Some(Bevel { inside: true, ..bevel }), ..Extrude::default() }, front()).unwrap();
    // Straight on: the cap inset by the width, and the chamfer around it filling the outline.
    let cap = inside.iter().find(|f| close(signed_area(&f.contours[0]).abs(), 80.0 * 80.0, 1e-6));
    assert!(cap.is_some(), "an 80 × 80 cap");
    assert_eq!(extent(&inside), Rect::new(0.0, 0.0, 100.0, 100.0));
    assert!(close(area(&inside), 100.0 * 100.0, 1e-6));
    let outside = extrude(&p, FillRule::EvenOdd, b, Extrude { bevel: Some(bevel), ..Extrude::default() }, front()).unwrap();
    assert_eq!(extent(&outside), Rect::new(-10.0, -10.0, 110.0, 110.0), "the walls stand 10 pt outside the art");
    assert!(outside.iter().any(|f| close(signed_area(&f.contours[0]).abs(), 100.0 * 100.0, 1e-6)), "the cap is the art");
    // The chamfer's faces lean towards the light differently from the cap.
    assert!(inside.iter().any(|f| !close(f.brightness, inside[inside.len() - 1].brightness, 1e-3)));
}

#[test]
fn bevel_shapes_repeats_and_both_sides_change_the_surface() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let v = View { rotation_x: -20.0, rotation_y: 30.0, ..front() };
    let base = Bevel { width: 12.0, height: 12.0, ..Bevel::default() };
    let classic = extrude(&p, FillRule::EvenOdd, b, Extrude { bevel: Some(base), ..Extrude::default() }, v).unwrap();
    for shape in BevelShape::ALL.into_iter().skip(1) {
        assert_ne!(
            classic,
            extrude(&p, FillRule::EvenOdd, b, Extrude { bevel: Some(Bevel { shape, ..base }), ..Extrude::default() }, v).unwrap(),
            "{shape:?}"
        );
    }
    for bevel in [Bevel { repeat: 3, ..base }, Bevel { repeat: 3, space: 0.3, ..base }, Bevel { both_sides: true, ..base }] {
        assert_ne!(classic, extrude(&p, FillRule::EvenOdd, b, Extrude { bevel: Some(bevel), ..Extrude::default() }, v).unwrap(), "{bevel:?}");
    }
    // The step shape keeps its corners: half the width at half the height.
    assert_eq!(BevelShape::Step.at(0.5), (0.5, 0.5));
    assert_eq!(BevelShape::Step.at(0.25), (0.0, 0.5));
    for shape in BevelShape::ALL {
        assert_eq!(shape.at(0.0), (0.0, 0.0), "{shape:?}");
        let (x, y) = shape.at(1.0);
        assert!(close(x, 1.0, 1e-12) && close(y, 1.0, 1e-12), "{shape:?}");
        assert_eq!(BevelShape::parse(shape.id()), Some(shape));
    }
}

#[test]
fn holes_stay_holes_in_the_caps_and_get_walls_of_their_own() {
    let mut b = square(0.0, 0.0, 100.0);
    // The hole drawn the same way round as the outline: even-odd nesting still makes it a hole.
    b.extend(square(30.0, 30.0, 40.0));
    let (p, r) = shape(&b);
    let f = extrude(&p, FillRule::EvenOdd, r, Extrude::default(), front()).unwrap();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].contours.len(), 2);
    assert!(close(area(&f), 100.0 * 100.0 - 40.0 * 40.0, 1e-6));
    let turned = extrude(&p, FillRule::EvenOdd, r, Extrude::default(), View { rotation_y: 40.0, ..front() }).unwrap();
    // Front cap, the outline's right wall and the hole's left wall (seen through the hole).
    assert_eq!(turned.len(), 3);
}

#[test]
fn a_plane_turns_about_the_given_centre() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let f = plane(&p, FillRule::EvenOdd, b, View { rotation_y: 60.0, ..front() }).unwrap();
    assert_eq!(f.len(), 1);
    let e = extent(&f);
    assert!(close(e.width(), 50.0, 1e-6) && close(e.center().x, 50.0, 1e-9));
    // Turned about the centre of a larger box, the square moves as well as narrowing.
    let wide = plane(&p, FillRule::EvenOdd, Rect::new(0.0, 0.0, 300.0, 100.0), View { rotation_y: 60.0, ..front() }).unwrap();
    assert!(close(extent(&wide).x0, 150.0 - 150.0 * 0.5, 1e-6));
    // Its back shows when turned past 90°: a plane is two-sided.
    assert_eq!(plane(&p, FillRule::EvenOdd, b, View { rotation_y: 150.0, ..front() }).unwrap().len(), 1);
    // From either side it is drawn last, so the planes of several parts sorted together keep the
    // order they are given in, and it is lit as the side seen.
    for (y, seen) in [(30.0, 1.0), (150.0, -1.0)] {
        let v = View { rotation_x: -20.0, rotation_y: y, ..front() };
        let f = plane(&p, FillRule::EvenOdd, b, v).unwrap();
        let camera = Camera::new(v, b, 0.0);
        let lit = camera.brightness(camera.rotate(V3(0.0, 0.0, seen)));
        assert!(f.len() == 1 && f[0].depth == f64::MAX && close(f[0].brightness, lit, 1e-12), "{y}: {:?}", f[0]);
    }
}

#[test]
fn inflate_covers_the_shape_and_puffs_by_the_volume() {
    let mut c = BezPath::new();
    c.extend(vectorcraft_geom::kurbo::Shape::path_elements(&vectorcraft_geom::kurbo::Circle::new((100.0, 100.0), 50.0), 0.01));
    let (p, b) = shape(&c);
    let f = inflate(&p, FillRule::EvenOdd, b, Inflate::default(), front()).unwrap();
    // Straight on, the cushion covers the circle (sampled as a polygon) and nothing else.
    assert!(close(area(&f), std::f64::consts::PI * 50.0 * 50.0, 60.0), "{}", area(&f));
    assert!(extent(&f).x0 >= 50.0 - 1e-6 && extent(&f).x1 <= 150.0 + 1e-6);
    // Its slopes catch the light differently.
    let (lo, hi) = f.iter().fold((1.0f64, 0.0f64), |(lo, hi), f| (lo.min(f.brightness), hi.max(f.brightness)));
    assert!(hi - lo > 0.3, "{lo}..{hi}");
    // From the side: 50 % volume rounds a disc into a half sphere on a flat back.
    let side = View { rotation_y: 90.0, ..front() };
    let thick = |o: Inflate| extent(&inflate(&p, FillRule::EvenOdd, b, o, side).unwrap()).width();
    assert!(close(thick(Inflate::default()), 50.0, 2.0), "{}", thick(Inflate::default()));
    assert!(close(thick(Inflate { both_sides: true, ..Inflate::default() }), 100.0, 4.0));
    assert!(close(thick(Inflate { depth: 30.0, ..Inflate::default() }), 80.0, 2.0));
    assert!(close(thick(Inflate { volume: 25.0, ..Inflate::default() }), 25.0, 2.0));
    assert!(thick(Inflate { volume: 0.0, depth: 10.0, ..Inflate::default() }) <= 10.0 + 1e-6);
}

#[test]
fn inflate_leaves_holes_open() {
    let mut b = square(0.0, 0.0, 100.0);
    b.extend(square(30.0, 30.0, 40.0));
    let (p, r) = shape(&b);
    let f = inflate(&p, FillRule::EvenOdd, r, Inflate::default(), front()).unwrap();
    assert!(close(area(&f), 100.0 * 100.0 - 40.0 * 40.0, 1e-3), "{}", area(&f));
}

#[test]
fn malformed_and_degenerate_input_is_bounded() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    for r in [Rect::new(f64::NAN, 0.0, 1.0, 1.0), Rect::new(0.0, 0.0, 1e9, 1.0)] {
        assert!(extrude(&p, FillRule::EvenOdd, r, Extrude::default(), front()).is_err());
        assert!(inflate(&p, FillRule::EvenOdd, r, Inflate::default(), front()).is_err());
        assert!(plane(&p, FillRule::EvenOdd, r, front()).is_err());
    }
    let wild = Extrude {
        depth: f64::INFINITY,
        twist: f64::NAN,
        taper: -3.0,
        cap: true,
        bevel: Some(Bevel { width: f64::NAN, height: 1e12, repeat: usize::MAX, space: 7.0, ..Bevel::default() }),
        hidden: false,
    };
    let f = extrude(&p, FillRule::EvenOdd, b, wild, View { rotation_x: f64::NAN, perspective: 900.0, ..front() }).unwrap();
    assert!(f.iter().flat_map(|f| f.contours.iter().flatten()).all(|q| q.x.is_finite() && q.y.is_finite()));
    assert!(
        inflate(
            &p,
            FillRule::EvenOdd,
            b,
            Inflate { depth: -5.0, volume: f64::NAN, both_sides: true, twist: f64::INFINITY, taper: f64::NAN, cap: true },
            front()
        )
        .is_ok()
    );
    assert!(extrude(&PathData::default(), FillRule::EvenOdd, b, Extrude::default(), front()).unwrap().is_empty());
    // A line has no area to extrude.
    let mut line = BezPath::new();
    line.move_to((0.0, 0.0));
    line.line_to((100.0, 0.0));
    assert!(extrude(&PathData::from_bezpath(&line), FillRule::EvenOdd, b, Extrude::default(), front()).unwrap().is_empty());
    // Too many anchors: refused, not truncated.
    let mut many = BezPath::new();
    many.move_to((0.0, 0.0));
    for i in 0..70_000 {
        many.line_to((f64::from(i), f64::from(i % 7)));
    }
    many.close_path();
    assert_eq!(
        extrude(&PathData::from_bezpath(&many), FillRule::EvenOdd, b, Extrude::default(), front()).unwrap_err(),
        "The shape is too complex for a 3D effect"
    );
}

#[test]
fn perspective_makes_the_near_face_larger() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let o = Extrude { depth: 100.0, ..Extrude::default() };
    let flat = extent(&extrude(&p, FillRule::EvenOdd, b, o, front()).unwrap());
    let deep = extent(&extrude(&p, FillRule::EvenOdd, b, o, View { perspective: 30.0, ..front() }).unwrap());
    assert!(deep.width() > flat.width() + 1.0);
    assert!(close(deep.center().x, flat.center().x, 1e-6));
}

/// The wider the Perspective, the nearer the eye: seen straight on, the front of a cube is drawn
/// larger and larger, up to 160°, where the eye is still outside the cube.
#[test]
fn a_wider_perspective_brings_the_eye_nearer() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let o = Extrude { depth: 100.0, ..Extrude::default() };
    let mut last = 100.0;
    for angle in [20.0, 60.0, 110.0, 160.0] {
        // The front face alone shows: its width is the cube's on the page.
        let width = extent(&extrude(&p, FillRule::EvenOdd, b, o, View { perspective: angle, ..front() }).unwrap()).width();
        assert!(width > last, "{angle}°: {width} after {last}");
        last = width;
    }
    // At 160° the eye is about 87.9 pt in front of the cube's centre (√3 × 50 / sin 80°), so the
    // front face, 50 pt nearer than the centre, is drawn about 2.3 times its size.
    let eye = 3f64.sqrt() * 50.0 / 80f64.to_radians().sin();
    assert!(close(last, 100.0 * eye / (eye - 50.0), 1e-6), "{last}");
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

/// A "D" with its counter, as outlined type draws one: straight stems, round bowls, and corners
/// where they meet.
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

/// A path's curved segments, segments, smooth anchors and anchors.
fn anatomy(p: &PathData) -> [usize; 4] {
    let mut n = [0; 4];
    for sp in &p.subpaths {
        let count = if sp.closed { sp.anchors.len() } else { sp.anchors.len().saturating_sub(1) };
        n[0] += (0..count).filter(|i| !sp.segment_is_line(*i)).count();
        n[1] += count;
        n[2] += sp.anchors.iter().filter(|a| a.kind == vectorcraft_geom::AnchorKind::Smooth).count();
        n[3] += sp.anchors.len();
    }
    n
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

/// How far `p` lies from the closed polygons `target` at most, sampled along each segment.
fn farthest(p: &PathData, target: &[Vec<Point>]) -> f64 {
    let mut far = 0.0f64;
    for sp in &p.subpaths {
        let count = if sp.closed { sp.anchors.len() } else { sp.anchors.len().saturating_sub(1) };
        for i in 0..count {
            let c = sp.segment(i);
            for k in 0..=32 {
                let q = c.eval(f64::from(k) / 32.0);
                assert!(q.x.is_finite() && q.y.is_finite(), "{q:?}");
                far = far.max(edge_distance(target, q));
            }
        }
    }
    far
}

/// Seen straight on, the front cap of an extruded circle or letter is the art: each curve stays
/// one cubic, smooth anchors stay smooth, and no point lies farther than the curve tolerance from
/// the art. The facet's polygon, which visibility uses, is still the sampled outline.
#[test]
fn a_cap_keeps_the_curves_of_the_art() {
    for art in [circle(Point::new(50.0, 50.0), 40.0), letter_d()] {
        let (p, b) = shape(&art);
        let f = extrude(&p, FillRule::EvenOdd, b, Extrude::default(), front()).unwrap();
        assert_eq!(f.len(), 1);
        let cap = f[0].path();
        assert_eq!(anatomy(&cap), anatomy(&p), "{cap:?}");
        assert!(f[0].contours.iter().map(Vec::len).sum::<usize>() > anatomy(&p)[3]);
        let far = farthest(&cap, &fine(&art, |q| q));
        assert!(far <= outline::CURVE_TOLERANCE, "{far}");
    }
}

/// Turned and in perspective, the front cap's curves are fitted to the projected circle within
/// the curve tolerance, every anchor stays smooth, and the walls' edges along it are curved too.
#[test]
fn a_cap_in_perspective_fits_the_projected_curves() {
    let art = circle(Point::new(50.0, 50.0), 40.0);
    let (p, b) = shape(&art);
    let o = Extrude { depth: 50.0, ..Extrude::default() };
    for view in [View { rotation_x: 20.0, rotation_y: 30.0, perspective: 60.0, ..front() }, View { perspective: 120.0, ..front() }] {
        let f = extrude(&p, FillRule::EvenOdd, b, o, view).unwrap();
        let cap = f.iter().find(|f| f.depth == f64::MAX).unwrap().path();
        let [curves, segments, smooth, anchors] = anatomy(&cap);
        assert!(curves >= 4 && curves == segments && smooth == anchors, "{:?}", anatomy(&cap));
        let camera = Camera::new(view, b, o.depth);
        let projected = fine(&art, |q| camera.project(camera.place(q, o.depth / 2.0)));
        let far = farthest(&cap, &projected);
        assert!(far <= outline::CURVE_TOLERANCE + 1e-6, "{far}");
        if view.rotation_y != 0.0 {
            assert!(f.iter().filter(|f| f.depth != f64::MAX).any(|f| anatomy(&f.path())[0] == 2), "the walls' edges along the circle are curved");
        }
    }
}

/// A twisted, tapered back cap follows the turned, scaled art; a cap inset by a bevel stays
/// within the sampling tolerance of the solid; a tiny circle and a curve with a handle of zero
/// length keep their cubics.
#[test]
fn twisted_beveled_tiny_and_degenerate_caps_keep_their_curves() {
    let art = circle(Point::new(50.0, 50.0), 40.0);
    let (p, b) = shape(&art);
    let o = Extrude { depth: 50.0, twist: 45.0, taper: 0.5, ..Extrude::default() };
    let behind = View { rotation_y: 180.0, ..front() };
    let f = extrude(&p, FillRule::EvenOdd, b, o, behind).unwrap();
    let cap = f.iter().find(|f| f.depth == f64::MAX).unwrap().path();
    assert_eq!(anatomy(&cap)[0], 4);
    let camera = Camera::new(behind, b, o.depth);
    let back = fine(&art, |q| section(&[q], b, &o, 1.0).first().map_or(q, |q| camera.project(camera.place(*q, -o.depth / 2.0))));
    let far = farthest(&cap, &back);
    assert!(far <= outline::CURVE_TOLERANCE + 1e-6, "{far}");

    // Where a curve meets a straight edge at a corner, the bevel's miter moves the cap's corner off
    // the curve's offset; the cap's curves still meet it.
    let bevel = Extrude { bevel: Some(Bevel { inside: true, width: 4.0, height: 4.0, ..Bevel::default() }), ..Extrude::default() };
    for art in [art.clone(), letter_d()] {
        let (p, b) = shape(&art);
        let f = extrude(&p, FillRule::EvenOdd, b, bevel, front()).unwrap();
        let cap = f.iter().find(|f| f.depth == f64::MAX).unwrap();
        // An offset curve is no cubic: it may take several, but no edge of a curve stays straight.
        let ([curves, segments, ..], [art_curves, art_segments, ..]) = (anatomy(&cap.path()), anatomy(&p));
        assert!(curves >= art_curves && segments - curves == art_segments - art_curves, "{:?}", cap.path());
        let far = farthest(&cap.path(), &cap.contours);
        assert!(far <= 2.0 * 0.05, "{far}");
    }

    let mut flat = PathData::from_bezpath(&art);
    if let Some(a) = flat.subpaths.first_mut().and_then(|sp| sp.anchors.first_mut()) {
        a.h_out = a.p;
    }
    for p in [PathData::from_bezpath(&circle(Point::new(1.0, 1.0), 0.01)), flat] {
        let b = p.bounds().unwrap();
        let f = extrude(&p, FillRule::NonZero, b, Extrude::default(), front()).unwrap();
        let cap = f[0].path();
        assert_eq!(anatomy(&cap)[0], anatomy(&p)[0], "{cap:?}");
        let far = farthest(&cap, &fine(&p.to_bezpath(), |q| q));
        assert!(far <= outline::CURVE_TOLERANCE, "{far}");
    }
}

#[test]
fn the_inflate_membrane_matches_known_solutions() {
    // ∇²φ = −1, zero on the outline: a disc of radius r peaks at r²/4, a square of side l at
    // about 0.07367 l².
    let mut c = BezPath::new();
    c.extend(vectorcraft_geom::kurbo::Shape::path_elements(&vectorcraft_geom::kurbo::Circle::new((100.0, 100.0), 50.0), 0.01));
    let (p, b) = shape(&c);
    let r = rings(&p, FillRule::EvenOdd, 100.0, MAX_SOLID_POINTS).unwrap();
    let disc = Membrane::solve(&r, &Edges::new(&r).unwrap(), b.origin(), b.width() / 48.0, 49, 49);
    assert!(close(disc.peak, 50.0 * 50.0 / 4.0, 625.0 * 0.02), "{}", disc.peak);
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let r = rings(&p, FillRule::EvenOdd, 100.0, MAX_SOLID_POINTS).unwrap();
    let sq = Membrane::solve(&r, &Edges::new(&r).unwrap(), b.origin(), 100.0 / 48.0, 49, 49);
    assert!(close(sq.peak, 0.07367 * 100.0 * 100.0, 736.7 * 0.02), "{}", sq.peak);
    // Smooth across the middle: no crease where a distance-to-the-edge height would have one
    // (the diagonals). The second difference along a diagonal stays small and even.
    let along: Vec<f64> = (10..=90).step_by(5).map(|t| sq.at(Point::new(f64::from(t), f64::from(t)))).collect();
    // (Near the corners φ grows like r², so the bend changes sign there: that is the solution,
    // not a crease. A crease would be one bend far larger than its neighbours.)
    let bends: Vec<f64> = along.windows(3).map(|w| w[0] - 2.0 * w[1] + w[2]).collect();
    let largest = bends.iter().fold(0.0f64, |m, b| m.max(b.abs()));
    assert!(bends.windows(2).all(|w| (w[1] - w[0]).abs() < 0.4 * largest), "no kink: {bends:?}");
    // Zero (or just below, extrapolated) on the outline.
    assert!(sq.at(Point::new(0.0, 50.0)).abs() < 1.0 && sq.at(Point::new(50.0, 0.0)).abs() < 1.0);
}

/// Grid cells across a hole's edge are cut to the shape itself: a hole's piece cancelling part of
/// the outline's only works flat, and lifted onto the surface it left notches along the edge.
#[test]
fn inflate_cells_across_a_hole_edge_are_cut_to_the_shape() {
    let mut b = square(80.0, 330.0, 220.0);
    b.move_to((140.0, 390.0));
    b.line_to((140.0, 480.0));
    b.line_to((240.0, 480.0));
    b.line_to((240.0, 390.0));
    b.close_path();
    let (p, r) = shape(&b);
    let v = View { rotation_x: -21.88, rotation_y: -23.4, rotation_z: 9.06, ..front() };
    let f = inflate(&p, FillRule::EvenOdd, r, Inflate::default(), v).unwrap();
    assert!(f.iter().all(|f| f.contours.len() == 1), "every facet is one piece of the surface");
    // Straight on, the facets still cover exactly the shape.
    let flat = inflate(&p, FillRule::EvenOdd, r, Inflate::default(), front()).unwrap();
    assert!(close(area(&flat), 220.0 * 220.0 - 100.0 * 90.0, 1e-3), "{}", area(&flat));
}

/// A row of `count` small bumps, each a curve, closed along the bottom: like an outlined word,
/// many short curves.
fn bumps(count: usize) -> (PathData, Rect) {
    let mut b = BezPath::new();
    b.move_to((0.0, 20.0));
    for i in 0..count {
        let x = i as f64 * 4.0;
        b.curve_to((x + 1.0, 14.0), (x + 3.0, 14.0), (x + 4.0, 20.0));
    }
    b.line_to((count as f64 * 4.0, 30.0));
    b.line_to((0.0, 30.0));
    b.close_path();
    shape(&b)
}

/// Curves are followed within a tolerance rather than by sixteen points each: small curves take a
/// few points, so shapes of many curves (outlined type) fit, in a solid and in Rotate.
#[test]
fn small_curves_take_few_points_and_rotate_takes_large_outlines() {
    // 300 curves were 4,800 points at sixteen each, over the old limit of 1,024.
    let (p, r) = bumps(300);
    assert!(extrude(&p, FillRule::EvenOdd, r, Extrude::default(), front()).is_ok());
    assert!(inflate(&p, FillRule::EvenOdd, r, Inflate { volume: 20.0, ..Inflate::default() }, front()).is_ok());
    assert!(plane(&p, FillRule::EvenOdd, r, front()).is_ok());
    // 4,000 curves fit a solid and Rotate alike; 70,000 fit neither.
    let (p, r) = bumps(4000);
    assert!(extrude(&p, FillRule::EvenOdd, r, Extrude::default(), front()).is_ok());
    assert_eq!(plane(&p, FillRule::EvenOdd, r, front()).unwrap().len(), 1);
    let (p, r) = bumps(70_000);
    assert_eq!(extrude(&p, FillRule::EvenOdd, r, Extrude::default(), front()).unwrap_err(), "The shape is too complex for a 3D effect");
    assert_eq!(plane(&p, FillRule::EvenOdd, r, front()).unwrap_err(), "The shape is too complex for a 3D effect");
    // Each piece stays within the tolerance of the curve: a circle's chords sag at most 0.05 pt
    // here (1/2000 of its 100 pt size is less).
    let mut c = BezPath::new();
    c.extend(vectorcraft_geom::kurbo::Shape::path_elements(&vectorcraft_geom::kurbo::Circle::new((50.0, 50.0), 50.0), 0.01));
    let (p, r) = shape(&c);
    let ring = &rings(&p, FillRule::EvenOdd, 100.0, MAX_SOLID_POINTS).unwrap()[0];
    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        let mid = a.midpoint(*b);
        assert!(50.0 - mid.distance(Point::new(50.0, 50.0)) <= 0.05 + 1e-3, "{a:?} {b:?}");
    }
    assert!(r.width() > 0.0);
    // A small bump takes a few points; a large curve more than sixteen where it needs them.
    let (p, r) = bumps(300);
    let small = rings(&p, FillRule::EvenOdd, r.width().max(r.height()), MAX_SOLID_POINTS).unwrap()[0].len();
    assert!(small <= 300 * 4 + 3, "{small} points for 300 bumps");
    assert!(ring.len() > 4 * 16, "a 100 pt circle within 0.05 pt: {}", ring.len());
}

/// The pieces twisted walls and bevels are cut into stay within the facet limit: 200 bumps with
/// a rounded bevel, turned 360°, give no more facets than the limit.
#[test]
fn twisted_pieces_stay_within_the_facet_limit() {
    let (p, r) = bumps(200);
    let o = Extrude { twist: 360.0, bevel: Some(Bevel { shape: BevelShape::Rounded, ..Bevel::default() }), ..Extrude::default() };
    let f = extrude(&p, FillRule::EvenOdd, r, o, View { rotation_x: -18.0, rotation_y: -26.0, rotation_z: 8.0, ..front() });
    assert!(f.is_ok_and(|f| f.len() <= MAX_FACES));
}

/// The bucketed nearest-edge search agrees with checking every edge, inside, outside and far
/// beyond the shape.
#[test]
fn the_edge_index_finds_the_nearest_edge() {
    let (p, r) = bumps(200);
    let rings = rings(&p, FillRule::EvenOdd, r.width(), MAX_SOLID_POINTS).unwrap();
    let edges = Edges::new(&rings).unwrap();
    for k in 0..400 {
        let t = f64::from(k);
        let q = Point::new(-300.0 + (t * 37.0) % 1400.0, -50.0 + (t * 13.0) % 130.0);
        let (fast, slow) = (edges.distance(q), edge_distance(&rings, q));
        assert!((fast - slow).abs() < 1e-9, "{q:?}: {fast} against {slow}");
    }
}

/// From a point outside the grid, an edge further along the grid's side can be nearer than one
/// straight ahead: the search doesn't stop before it has looked there.
#[test]
fn the_edge_index_looks_along_the_side_from_outside() {
    let ring = |b: BezPath| rings(&shape(&b).0, FillRule::EvenOdd, 64.0, MAX_SOLID_POINTS).unwrap().remove(0);
    // Straight ahead of the point, a little further up its side, and the far corner (a 64 pt grid).
    let rings = vec![ring(square(2.5, 0.0, 1.0)), ring(square(0.0, 4.2, 0.8)), ring(square(63.0, 63.0, 1.0))];
    let edges = Edges::new(&rings).unwrap();
    let q = Point::new(-10.0, 0.5);
    let (fast, slow) = (edges.distance(q), edge_distance(&rings, q));
    assert!((fast - slow).abs() < 1e-9, "{fast} against {slow}");
}

/// Twist runs 0° to 360°, as documented: beyond either end it holds at that end.
#[test]
fn twist_holds_between_0_and_360_degrees() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let v = View { rotation_x: -20.0, rotation_y: 30.0, ..front() };
    let at = |twist: f64| extrude(&p, FillRule::EvenOdd, b, Extrude { twist, ..Extrude::default() }, v).unwrap();
    assert_eq!(at(-45.0), at(0.0));
    assert_eq!(at(500.0), at(360.0));
    assert_ne!(at(45.0), at(0.0));
}

/// A twisted extrusion shows every wall facing the viewer: a twist bends each strip between two
/// slices into a saddle, and its normal from one corner's triangle leaned so far back that from
/// 60° on every wall was dropped. Checked against the average normal of each piece the strip is
/// cut into along its edge ([`cuts`]), oriented away from the square's axis.
#[test]
fn twisted_walls_face_the_way_they_turn() {
    let lerp = |p: V3, q: V3, s: f64| {
        if s <= 0.0 {
            p
        } else if s >= 1.0 {
            q
        } else {
            V3(p.0 + (q.0 - p.0) * s, p.1 + (q.1 - p.1) * s, p.2 + (q.2 - p.2) * s)
        }
    };
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let c = b.center();
    for twist in [60.0, 120.0, 180.0, 300.0] {
        for (x, y) in [(-20.0, 30.0), (0.0, 90.0), (30.0, -60.0), (-45.0, 150.0)] {
            let v = View { rotation_x: x, rotation_y: y, ..front() };
            let o = Extrude { twist, ..Extrude::default() };
            let got = extrude(&p, FillRule::EvenOdd, b, o, v).unwrap();
            // Expected: walls (and caps) whose outward normal faces the viewer.
            let o = o.sanitized();
            let camera = Camera::new(v, b, o.depth);
            let ring = &rings(&p, FillRule::EvenOdd, b.width().max(b.height()), MAX_SOLID_POINTS).unwrap()[0];
            let slices = ((twist / 6.0).ceil() as usize).clamp(8, 64);
            let z = |t: f64| o.depth / 2.0 - o.depth * t;
            let placed = |t: f64| -> Vec<V3> { section(ring, b, &o, t).iter().map(|p| camera.place(*p, z(t))).collect() };
            let pieces = cuts(&placed(0.0), &placed(1.0 / slices as f64));
            let mut expected = 0;
            for k in 0..slices {
                let (t0, t1) = (k as f64 / slices as f64, (k + 1) as f64 / slices as f64);
                let (r0, r1) = (section(ring, b, &o, t0), section(ring, b, &o, t1));
                for i in 0..ring.len() {
                    let j = (i + 1) % ring.len();
                    let q = [camera.place(r0[i], z(t0)), camera.place(r0[j], z(t0)), camera.place(r1[j], z(t1)), camera.place(r1[i], z(t1))];
                    let pieces = pieces[i];
                    for k in 0..pieces {
                        let (s0, s1) = (k as f64 / pieces as f64, (k + 1) as f64 / pieces as f64);
                        let q = [lerp(q[0], q[1], s0), lerp(q[0], q[1], s1), lerp(q[3], q[2], s1), lerp(q[3], q[2], s0)];
                        let mut n = newell(&[q.to_vec()]).unit();
                        let s = (s0 + s1) / 2.0;
                        let mid = r0[i].lerp(r0[j], s).midpoint(r1[i].lerp(r1[j], s));
                        let away = camera.rotate(V3(mid.x - c.x, mid.y - c.y, 0.0));
                        if n.dot(away) < 0.0 {
                            n = V3(-n.0, -n.1, -n.2);
                        }
                        let sum = q.iter().fold(V3::default(), |s, v| V3(s.0 + v.0, s.1 + v.1, s.2 + v.2));
                        let centre = V3(sum.0 / 4.0, sum.1 / 4.0, sum.2 / 4.0);
                        // Kept as the builder keeps them: facing, or seen almost edge-on, with an area.
                        let projected: Vec<Point> = q.iter().map(|v| camera.project(*v)).collect();
                        if camera.facing(centre, n) > -0.02 && signed_area(&projected).abs() >= 1e-10 {
                            expected += 1;
                        }
                    }
                }
            }
            for (n, t) in [(1.0, 0.0), (-1.0, 1.0)] {
                let cap: Vec<Point> = section(ring, b, &o, t).iter().map(|p| camera.project(camera.place(*p, z(t)))).collect();
                if camera.facing(V3::default(), camera.rotate(V3(0.0, 0.0, n))) > -0.02 && signed_area(&cap).abs() >= 1e-10 {
                    expected += 1;
                }
            }
            assert_eq!(got.len(), expected, "twist {twist}, view ({x}, {y})");
        }
    }
}

/// The depth (larger is nearer the eye) of `f`'s own surface over the page point `p`, seen without
/// perspective, or `None` where `f` doesn't cover `p`. A flat facet is its plane, a four-cornered
/// one that isn't flat is the bilinear patch between its corners, and any other is a fan of
/// triangles from its center.
fn depth_at(f: &Facet, p: Point) -> Option<f64> {
    let winding: i32 = f
        .contours
        .iter()
        .flat_map(|c| c.iter().zip(c.iter().cycle().skip(1)).take(c.len()))
        .map(|(a, b)| {
            let side = (b.x - a.x) * (p.y - a.y) - (p.x - a.x) * (b.y - a.y);
            if a.y <= p.y && b.y > p.y && side > 0.0 {
                1
            } else if a.y > p.y && b.y <= p.y && side < 0.0 {
                -1
            } else {
                0
            }
        })
        .sum();
    if winding == 0 {
        return None;
    }
    // Without perspective a projected point is its view-space point moved by one offset.
    let pairs: Vec<Vec<(Point, f64)>> = f.contours.iter().zip(&f.view).map(|(c, v)| c.iter().zip(v).map(|(q, v)| (*q, v.2)).collect()).collect();
    let n = newell(&f.view);
    let (anchor, z0) = pairs[0][0];
    let flat = n.2.abs() > 1e-12
        && pairs.iter().flatten().all(|(q, z)| (n.0 * (q.x - anchor.x) + n.1 * (q.y - anchor.y) + n.2 * (z - z0)).abs() < 1e-9 * n.dot(n).sqrt());
    if flat {
        return Some(z0 - (n.0 * (p.x - anchor.x) + n.1 * (p.y - anchor.y)) / n.2);
    }
    let c = &pairs[0];
    if pairs.len() == 1 && c.len() == 4 {
        let at = |u: f64, v: f64| {
            let w = [(1.0 - u) * (1.0 - v), u * (1.0 - v), u * v, (1.0 - u) * v];
            c.iter().zip(w).fold((0.0, 0.0, 0.0), |s, ((q, z), w)| (s.0 + w * q.x, s.1 + w * q.y, s.2 + w * z))
        };
        // Newton's method for the patch point over `p`.
        let (mut u, mut v) = (0.5, 0.5);
        for _ in 0..50 {
            let (x, y, _) = at(u, v);
            let h = 1e-7;
            let (xu, yu, _) = at(u + h, v);
            let (xv, yv, _) = at(u, v + h);
            let (a, b, cc, d) = ((xu - x) / h, (xv - x) / h, (yu - y) / h, (yv - y) / h);
            let det = a * d - b * cc;
            if det.abs() < 1e-18 {
                return None;
            }
            let (ex, ey) = (p.x - x, p.y - y);
            u += (d * ex - b * ey) / det;
            v += (a * ey - cc * ex) / det;
        }
        return (u > -1e-6 && u < 1.0 + 1e-6 && v > -1e-6 && v < 1.0 + 1e-6).then(|| at(u, v).2);
    }
    let m = c.iter().fold((0.0, 0.0, 0.0), |s, (q, z)| (s.0 + q.x, s.1 + q.y, s.2 + z));
    let k = c.len() as f64;
    let (mx, my, mz) = (m.0 / k, m.1 / k, m.2 / k);
    for ((a, za), (b, zb)) in c.iter().zip(c.iter().cycle().skip(1)).take(c.len()) {
        let det = (a.x - mx) * (b.y - my) - (b.x - mx) * (a.y - my);
        if det.abs() < 1e-18 {
            continue;
        }
        let s = ((p.x - mx) * (b.y - my) - (b.x - mx) * (p.y - my)) / det;
        let t = ((a.x - mx) * (p.y - my) - (p.x - mx) * (a.y - my)) / det;
        if s >= -1e-9 && t >= -1e-9 && s + t <= 1.0 + 1e-9 {
            return Some(mz + s * (za - mz) + t * (zb - mz));
        }
    }
    None
}

/// Over a grid of `samples` × `samples` points across the facets, seen without perspective: how
/// far, at most, the facet drawn last over a point (the one seen) is behind a facet drawn before
/// it there, counting only facets it lies wholly behind the plane of (which must be drawn after
/// it), and at which point.
fn worst_misorder(facets: &[Facet], samples: usize) -> (f64, Point) {
    let e = extent(facets);
    let boxes: Vec<Rect> = facets.iter().map(|f| extent(std::slice::from_ref(f))).collect();
    let behind = |f: &Facet, other: &Facet| {
        // The other facet's plane through its corners' average, its normal toward the eye (+z).
        let n = newell(&other.view);
        let n = if n.2 < 0.0 { V3(-n.0, -n.1, -n.2) } else { n }.unit();
        let count = other.view.iter().map(Vec::len).sum::<usize>() as f64;
        let c = other.view.iter().flatten().fold(V3::default(), |s, v| V3(s.0 + v.0 / count, s.1 + v.1 / count, s.2 + v.2 / count));
        n.2 > 1e-9 && f.view.iter().flatten().all(|v| v.sub(c).dot(n) < -1e-3)
    };
    let mut worst = (0.0, e.origin());
    for j in 0..samples {
        for i in 0..samples {
            let p = Point::new(e.x0 + (i as f64 + 0.5) / samples as f64 * e.width(), e.y0 + (j as f64 + 0.5) / samples as f64 * e.height());
            let covering: Vec<(&Facet, f64)> =
                facets.iter().zip(&boxes).filter(|(_, r)| r.contains(p)).filter_map(|(f, _)| depth_at(f, p).map(|d| (f, d))).collect();
            let Some(((seen, depth), under)) = covering.split_last() else { continue };
            for (f, d) in under {
                if d - depth > worst.0 && behind(seen, f) {
                    worst = (d - depth, p);
                }
            }
        }
    }
    worst
}

/// A twist of 180° or 360° turns a square's walls into a twisted bar, whose ridges pass in front
/// of the walls between them, in front of its bevels and in front of its caps. Over every point
/// the facet drawn last is less than 6 pt behind any facet drawn before it whose plane it lies
/// wholly behind. Inflate's walls twist the same way (at 0 % volume its front is flat).
#[test]
fn twisted_walls_are_drawn_back_to_front() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let rounded = Bevel { shape: BevelShape::Rounded, width: 8.0, height: 8.0, both_sides: true, ..Bevel::default() };
    for (x, y, z) in [(-18.0, -26.0, 8.0), (-60.0, 20.0, 0.0), (75.0, 10.0, 30.0), (-20.0, 150.0, 10.0), (30.0, 200.0, 0.0)] {
        let v = View { rotation_x: x, rotation_y: y, rotation_z: z, ..front() };
        for twist in [180.0, 360.0] {
            for depth in [50.0, 200.0] {
                let solids = [
                    ("Extrude", extrude(&p, FillRule::EvenOdd, b, Extrude { depth, twist, ..Extrude::default() }, v)),
                    ("beveled", extrude(&p, FillRule::EvenOdd, b, Extrude { depth, twist, bevel: Some(rounded), ..Extrude::default() }, v)),
                    ("Inflate", inflate(&p, FillRule::EvenOdd, b, Inflate { depth, twist, volume: 0.0, ..Inflate::default() }, v)),
                    // 10 pt deep, the bevels take all of it; 40 pt deep, they leave a wall.
                    (
                        "bevels alone",
                        extrude(&p, FillRule::EvenOdd, b, Extrude { depth: depth / 5.0, twist, bevel: Some(rounded), ..Extrude::default() }, v),
                    ),
                ];
                for (kind, f) in solids {
                    let (gap, at) = worst_misorder(&f.unwrap(), 120);
                    assert!(gap < 6.0, "{kind}, view ({x}, {y}, {z}), twist {twist}, depth {depth}: {gap} pt behind at {at:?}");
                }
            }
        }
    }
}

/// Inflate takes Twist, Taper and Cap like the other types: the back turned and scaled (its puff
/// too), the sides following, and without a cap no back at all.
#[test]
fn inflate_twists_tapers_and_opens_its_back() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let side = View { rotation_y: 90.0, ..front() };
    let o = Inflate { depth: 60.0, volume: 0.0, ..Inflate::default() };
    // From the side, a flat (0 % volume) cushion is the extrusion: front 100 high, the back 50
    // at a 50 % taper.
    let tapered = inflate(&p, FillRule::EvenOdd, b, Inflate { taper: 0.5, ..o }, side).unwrap();
    let e = extent(&tapered);
    assert!(close(e.width(), 60.0, 1e-6) && close(e.height(), 100.0, 1e-6), "{e:?}");
    let back: Vec<Point> = tapered.iter().flat_map(|f| f.contours.iter().flatten().copied()).filter(|q| close(q.x, e.x0, 1e-6)).collect();
    let (lo, hi) = back.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), q| (lo.min(q.y), hi.max(q.y)));
    assert!(close(hi - lo, 50.0, 1e-6), "back {}", hi - lo);
    // From behind, a 45° twist shows the flat back as a diamond.
    let behind = View { rotation_y: 180.0, ..front() };
    let twisted = inflate(&p, FillRule::EvenOdd, b, Inflate { twist: 45.0, ..o }, behind).unwrap();
    assert!(close(extent(&twisted).width(), 100.0 * 2f64.sqrt(), 1e-6), "{:?}", extent(&twisted));
    // Hollow: no back; from behind the inside of the inflated front shows instead of a flat face.
    let solid = inflate(&p, FillRule::EvenOdd, b, Inflate { volume: 50.0, ..o }, behind).unwrap();
    assert_eq!(solid.len(), 1, "the flat back alone");
    let hollow = inflate(&p, FillRule::EvenOdd, b, Inflate { volume: 50.0, cap: false, ..o }, behind).unwrap();
    assert!(hollow.len() > 100 && hollow.iter().all(|f| f.contours.len() == 1 && f.contours[0].len() < 12));
    // With both sides puffed, the back puff is scaled with the taper: lower and smaller.
    let both =
        |taper: f64| extent(&inflate(&p, FillRule::EvenOdd, b, Inflate { both_sides: true, volume: 50.0, taper, depth: 0.0, ..o }, side).unwrap());
    assert!(both(0.5).width() < both(1.0).width() && both(0.5).height() <= both(1.0).height() + 1e-6);
    // Out of range values hold at the documented ends.
    let at = |i: Inflate| inflate(&p, FillRule::EvenOdd, b, i, View { rotation_x: -20.0, rotation_y: 30.0, ..front() }).unwrap();
    assert_eq!(at(Inflate { twist: -30.0, ..o }), at(o));
    assert_eq!(at(Inflate { taper: 0.0, ..o }), at(Inflate { taper: 0.01, ..o }));
}

/// A flat front is drawn after every other face only when it is turned toward the viewer: an open
/// Inflate at 0 % volume seen from behind shows the inside of its front, the far side, and the
/// walls nearer the eye are drawn over it.
#[test]
fn an_open_flat_front_seen_from_behind_is_drawn_under_the_walls() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let o = Inflate { depth: 60.0, volume: 0.0, cap: false, ..Inflate::default() };
    let ahead = inflate(&p, FillRule::EvenOdd, b, o, View { rotation_x: -20.0, rotation_y: 30.0, ..front() }).unwrap();
    assert!(ahead.last().is_some_and(|f| f.depth == f64::MAX), "from in front, the front is drawn last");
    let behind = inflate(&p, FillRule::EvenOdd, b, o, View { rotation_x: -20.0, rotation_y: 150.0, ..front() }).unwrap();
    assert!(behind.iter().all(|f| f.depth < f64::MAX));
    let (gap, at) = worst_misorder(&behind, 120);
    assert!(gap < 1.0, "{gap} pt behind at {at:?}");
}

/// At 0 % volume an inflated shape is the extruded one: twisted and tapered, their outlines
/// agree from every side (no wall goes missing in either).
#[test]
fn a_flat_twisted_inflate_matches_the_extrusion() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    for (x, y) in [(-20.0, 30.0), (0.0, 90.0), (30.0, -60.0), (-45.0, 150.0), (60.0, 200.0)] {
        let v = View { rotation_x: x, rotation_y: y, ..front() };
        let e = extent(&extrude(&p, FillRule::EvenOdd, b, Extrude { depth: 60.0, twist: 120.0, taper: 0.6, ..Extrude::default() }, v).unwrap());
        let i = extent(
            &inflate(&p, FillRule::EvenOdd, b, Inflate { depth: 60.0, twist: 120.0, taper: 0.6, volume: 0.0, ..Inflate::default() }, v).unwrap(),
        );
        for (a, c) in [(e.x0, i.x0), (e.y0, i.y0), (e.x1, i.x1), (e.y1, i.y1)] {
            assert!(close(a, c, 1e-6), "view ({x}, {y}): extrusion {e:?}, inflate {i:?}");
        }
    }
}

/// #1724: under the nonzero rule a contour inside another that runs the same way cuts no hole
/// (the middle is filled, as the art is drawn); one running the other way does, and so does
/// either under the even-odd rule.
#[test]
fn nonzero_shapes_keep_the_middle_a_same_way_contour_encloses() {
    let rect = |r: Rect, clockwise: bool| {
        let mut pts = vec![Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
        if clockwise {
            pts.reverse();
        }
        vectorcraft_geom::SubPath::polyline(&pts, true)
    };
    let (outer, inner) = (Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(25.0, 25.0, 75.0, 75.0));
    let nested = |inner_clockwise: bool| PathData::new(vec![rect(outer, false), rect(inner, inner_clockwise)]);
    let flat = |p: &PathData, rule: FillRule| area(&plane(p, rule, outer, front()).unwrap());
    let (solid, holed) = (100.0 * 100.0, 100.0 * 100.0 - 50.0 * 50.0);
    assert!(close(flat(&nested(false), FillRule::NonZero), solid, 1e-6), "the same way round: filled");
    assert!(close(flat(&nested(true), FillRule::NonZero), holed, 1e-6), "the other way round: a hole");
    assert!(close(flat(&nested(false), FillRule::EvenOdd), holed, 1e-6));
    assert!(close(flat(&nested(true), FillRule::EvenOdd), holed, 1e-6));
    // A third contour inside the hole is an island again, whichever way it runs.
    let island = |clockwise: bool| PathData::new(vec![rect(outer, false), rect(inner, true), rect(Rect::new(40.0, 40.0, 60.0, 60.0), clockwise)]);
    for clockwise in [false, true] {
        assert!(close(flat(&island(clockwise), FillRule::NonZero), holed + 20.0 * 20.0, 1e-6), "{clockwise}");
    }
    // Extrude's caps and Inflate follow the same outline.
    let caps = |p: &PathData| area(&extrude(p, FillRule::NonZero, outer, Extrude::default(), front()).unwrap());
    assert!(caps(&nested(false)) > caps(&nested(true)) + 1000.0);
    assert!(inflate(&nested(false), FillRule::NonZero, outer, Inflate::default(), front()).is_ok());
}

#[test]
fn inflate_over_bounds_of_subnormal_size_makes_no_grid() {
    let (p, _) = shape(&square(0.0, 0.0, 10.0));
    // 5e-324 divides to a zero cell; 1e-307 is a normal size whose cell (size / 48) is subnormal.
    for size in [5e-324, 1e-307] {
        let tiny = Rect::new(0.0, 0.0, size, size);
        assert_eq!(inflate(&p, FillRule::EvenOdd, tiny, Inflate::default(), front()).unwrap().len(), 0, "{size}");
    }
}

#[test]
fn many_small_rings_far_apart_are_too_complex() {
    // 8,000 squares in a row: no square's bounds hold another's corner, yet each pair is a step.
    let mut b = BezPath::new();
    for i in 0..8000 {
        b.extend(square(i as f64 * 3.0, 0.0, 1.0));
    }
    let (p, r) = shape(&b);
    let e = rings(&p, FillRule::NonZero, r.width().max(r.height()), usize::MAX).unwrap_err();
    assert!(e.contains("too complex"), "{e}");
}

/// Type often draws a glyph as contours that cross (a "k"'s arm across its stem, its leg across
/// the arm). Each contour's side was told from its first point, so one starting inside another
/// was dropped or cut as a hole, and the parts left were walled where they overlapped. The
/// outline is the union the art fills, under either rule.
#[test]
fn crossing_contours_make_one_outline() {
    let rect = |r: Rect| {
        vectorcraft_geom::SubPath::polyline(&[Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)], true)
    };
    // The stem, an arm starting inside it, and a leg starting inside the arm.
    let k = PathData::new(vec![
        rect(Rect::new(0.0, 0.0, 20.0, 100.0)),
        rect(Rect::new(10.0, 40.0, 60.0, 60.0)),
        rect(Rect::new(50.0, 50.0, 70.0, 100.0)),
    ]);
    let b = k.bounds().unwrap();
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let filled = vectorcraft_pathops::area(&k, rule);
        assert!(close(area(&plane(&k, rule, b, front()).unwrap()), filled, 1e-6), "{rule:?}: Rotate");
        let f = extrude(&k, rule, b, Extrude::default(), front()).unwrap();
        assert_eq!(f.len(), 1, "{rule:?}: straight on, only the front cap shows");
        assert!(close(area(&f), filled, 1e-6), "{rule:?}: Extrude's cap");
        let puffed = inflate(&k, rule, b, Inflate::default(), front()).unwrap();
        assert_eq!(extent(&puffed), b, "{rule:?}: Inflate covers the whole k");
        assert!(close(area(&puffed), filled, filled * 1e-3), "{rule:?}: Inflate's front, {} against {filled}", area(&puffed));
    }
    // The filled area is the union's: the stem, the arm past it and the leg past the arm.
    assert!(close(vectorcraft_pathops::area(&k, FillRule::NonZero), 20.0 * 100.0 + 40.0 * 20.0 + 20.0 * 50.0 - 10.0 * 10.0, 1e-6));
}

/// Resolving overlaps keeps every contour that fills area, however narrow: a hairline beside a
/// large shape is art. What a boolean leaves where two curves coincide is still dropped.
#[test]
fn resolving_overlaps_keeps_narrow_contours() {
    let rect = |r: Rect| {
        vectorcraft_geom::SubPath::polyline(&[Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)], true)
    };
    // A hairline beside a square, and two bars crossing elsewhere so the profile is resolved.
    let hairline = Rect::new(120.0, 0.0, 120.005, 50.0);
    let p = PathData::new(vec![
        rect(Rect::new(0.0, 0.0, 100.0, 100.0)),
        rect(hairline),
        rect(Rect::new(140.0, 40.0, 200.0, 60.0)),
        rect(Rect::new(160.0, 20.0, 180.0, 80.0)),
    ]);
    let b = p.bounds().unwrap();
    let filled = |rule: FillRule| {
        let bars = if rule == FillRule::NonZero { 60.0 * 20.0 + 20.0 * 40.0 } else { 60.0 * 20.0 + 20.0 * 40.0 - 20.0 * 20.0 };
        100.0 * 100.0 + hairline.width() * hairline.height() + bars
    };
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        assert!(close(area(&plane(&p, rule, b, front()).unwrap()), filled(rule), 1e-6), "{rule:?}: Rotate");
        assert!(close(area(&extrude(&p, rule, b, Extrude::default(), front()).unwrap()), filled(rule), 1e-6), "{rule:?}: Extrude's cap");
    }
    // A circle drawn twice, exactly and a hair off, is one outline: no sliver between the copies.
    let circle = |cx: f64, cy: f64, r: f64| {
        let (mut b, k) = (BezPath::new(), 0.5523 * r);
        b.move_to((cx + r, cy));
        b.curve_to((cx + r, cy + k), (cx + k, cy + r), (cx, cy + r));
        b.curve_to((cx - k, cy + r), (cx - r, cy + k), (cx - r, cy));
        b.curve_to((cx - r, cy - k), (cx - k, cy - r), (cx, cy - r));
        b.curve_to((cx + k, cy - r), (cx + r, cy - k), (cx + r, cy));
        b.close_path();
        b
    };
    for dx in [0.0, 1e-7] {
        let mut twice = circle(50.0, 50.0, 40.0);
        twice.extend(circle(50.0 + dx, 50.0, 40.0).iter());
        let p = PathData::from_bezpath(&twice);
        let f = plane(&p, FillRule::NonZero, p.bounds().unwrap(), front()).unwrap();
        assert_eq!(f.len(), 1, "{dx}");
        assert_eq!(f[0].contours.len(), 1, "{dx}");
        let once = PathData::from_bezpath(&circle(50.0, 50.0, 40.0));
        let one = area(&plane(&once, FillRule::NonZero, once.bounds().unwrap(), front()).unwrap());
        assert!(close(area(&f), one, one * 1e-6), "{dx}: the area of one circle, {} against {one}", area(&f));
    }
}

/// Only a profile whose contours meet goes through the boolean: crossing, touching, running along
/// another or folding back on itself. Separate and nested contours are taken as they are.
#[test]
fn contours_meet_when_they_cross_touch_or_fold_back() {
    let ring = |pts: &[(f64, f64)]| pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>();
    let square = |x: f64, y: f64, s: f64| ring(&[(x, y), (x + s, y), (x + s, y + s), (x, y + s)]);
    assert!(!rings::edges_meet(&[square(0.0, 0.0, 10.0), square(20.0, 0.0, 10.0)], 1e-6), "apart");
    assert!(!rings::edges_meet(&[square(0.0, 0.0, 10.0), square(3.0, 3.0, 4.0)], 1e-6), "nested, as an o's hole");
    assert!(rings::edges_meet(&[square(0.0, 0.0, 10.0), square(5.0, 5.0, 10.0)], 1e-6), "crossing");
    assert!(rings::edges_meet(&[square(0.0, 0.0, 10.0), square(10.0, 0.0, 10.0)], 1e-6), "sharing an edge");
    assert!(rings::edges_meet(&[square(0.0, 0.0, 10.0), square(10.0, 3.0, 4.0)], 1e-6), "along part of an edge");
    assert!(rings::edges_meet(&[square(0.0, 0.0, 10.0), square(10.0, 10.0, 4.0)], 1e-6), "at a corner");
    assert!(rings::edges_meet(&[square(0.0, 0.0, 10.0), square(10.0 + 1e-7, 0.0, 10.0)], 1e-6), "within the boolean's snapping");
    assert!(rings::edges_meet(&[ring(&[(0.0, 0.0), (10.0, 10.0), (10.0, 0.0), (0.0, 10.0)])], 1e-6), "a bowtie crossing itself");
    assert!(rings::edges_meet(&[ring(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (10.0, 5.0), (0.0, 10.0)])], 1e-6), "folding back on itself");
    // Squares sharing an edge inflate as one puff, not two with a seam between them.
    let shared = PathData::new(
        [Rect::new(0.0, 0.0, 50.0, 50.0), Rect::new(50.0, 0.0, 100.0, 50.0)]
            .iter()
            .map(|r| {
                vectorcraft_geom::SubPath::polyline(
                    &[Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)],
                    true,
                )
            })
            .collect(),
    );
    let whole = PathData::from_bezpath(&{
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        b.line_to((100.0, 0.0));
        b.line_to((100.0, 50.0));
        b.line_to((0.0, 50.0));
        b.close_path();
        b
    });
    let b = whole.bounds().unwrap();
    let puff = |p: &PathData| inflate(p, FillRule::NonZero, b, Inflate::default(), front()).unwrap().len();
    assert_eq!(puff(&shared), puff(&whole));
}

/// Flat contours fill nothing and build nothing, crossing or not; an invalid coordinate is refused,
/// as it was; and a profile the boolean refuses is built from its contours as drawn.
#[test]
fn flat_invalid_and_refused_profiles() {
    let flat =
        PathData::new(vec![vectorcraft_geom::SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 10.0), Point::new(20.0, 20.0)], true)]);
    let crossing_flat = PathData::new(vec![
        vectorcraft_geom::SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 10.0), Point::new(20.0, 20.0)], true),
        vectorcraft_geom::SubPath::polyline(&[Point::new(0.0, 20.0), Point::new(10.0, 10.0), Point::new(20.0, 0.0)], true),
    ]);
    let b = Rect::new(0.0, 0.0, 20.0, 20.0);
    for p in [&flat, &crossing_flat] {
        assert_eq!(area(&plane(p, FillRule::NonZero, b, front()).unwrap()), 0.0);
        assert_eq!(area(&extrude(p, FillRule::NonZero, b, Extrude::default(), front()).unwrap()), 0.0);
    }
    let mut bad = crossing_flat.clone();
    bad.subpaths[0].anchors[1].p = Point::new(f64::NAN, 10.0);
    assert_eq!(plane(&bad, FillRule::NonZero, b, front()).unwrap_err(), "The shape contains invalid coordinates");
    // A star of 3,001 points, each joined to the one 1,500 on: every edge spans the middle, so
    // past pathops' budget of segment pairs the boolean refuses it (`TooComplex`) without sweeping,
    // and the profile is turned as drawn.
    let n = 3001;
    let star: Vec<Point> = (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * ((i * 1500) % n) as f64 / n as f64;
            Point::new(100.0 + 100.0 * a.cos(), 100.0 + 100.0 * a.sin())
        })
        .collect();
    let star = PathData::new(vec![vectorcraft_geom::SubPath::polyline(&star, true)]);
    assert_eq!(
        vectorcraft_pathops::try_unite_all_with(&[(&star, FillRule::NonZero)], rings::DEBRIS),
        Err(vectorcraft_pathops::PathOpsError::TooComplex)
    );
    let b = star.bounds().unwrap();
    let drawn: f64 =
        rings::rings(&star, FillRule::NonZero, b.width().max(b.height()), usize::MAX).unwrap().iter().map(|r| rings::signed_area(r)).sum();
    assert!(drawn > 0.0);
    assert!(close(area(&plane(&star, FillRule::NonZero, b, front()).unwrap()), drawn, 1e-6), "built from its contours as drawn");
}

/// A contour that crosses itself into lobes that cancel has no area of its own, yet fills both
/// lobes; contours that cancel each other fill nothing. Both go through the boolean.
#[test]
fn self_crossing_and_cancelling_profiles_are_resolved() {
    let poly = |pts: &[(f64, f64)]| vectorcraft_geom::SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), true);
    let bowtie = PathData::new(vec![poly(&[(0.0, 0.0), (10.0, 10.0), (10.0, 0.0), (0.0, 10.0)])]);
    let b = bowtie.bounds().unwrap();
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        assert!(close(area(&plane(&bowtie, rule, b, front()).unwrap()), 50.0, 1e-6), "{rule:?}: Rotate");
        assert!(close(area(&extrude(&bowtie, rule, b, Extrude::default(), front()).unwrap()), 50.0, 1e-6), "{rule:?}: Extrude's cap");
        assert!(inflate(&bowtie, rule, b, Inflate::default(), front()).is_ok_and(|f| close(area(&f), 50.0, 1e-6)), "{rule:?}: Inflate");
    }
    let square = poly(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    let twice = PathData::new(vec![square.clone(), square]);
    let b = twice.bounds().unwrap();
    assert_eq!(area(&plane(&twice, FillRule::EvenOdd, b, front()).unwrap()), 0.0);
    assert_eq!(area(&extrude(&twice, FillRule::EvenOdd, b, Extrude::default(), front()).unwrap()), 0.0);
    assert!(inflate(&twice, FillRule::EvenOdd, b, Inflate::default(), front()).is_ok_and(|f| area(&f) == 0.0));
}

/// The boolean's answer often starts a contour where it touches another. Its rings are turned as
/// a whole, never judged from one point each, so touching parts are kept and holes stay open.
#[test]
fn touching_contours_keep_their_parts_and_holes() {
    let poly = |pts: &[(f64, f64)]| vectorcraft_geom::SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), true);
    // A triangle whose tip rests on a square's edge.
    let tip = PathData::new(vec![poly(&[(0.0, 10.0), (10.0, 10.0), (10.0, 20.0), (0.0, 20.0)]), poly(&[(0.0, 0.0), (10.0, 0.0), (5.0, 10.0)])]);
    let b = tip.bounds().unwrap();
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        assert!(close(area(&plane(&tip, rule, b, front()).unwrap()), 150.0, 1e-6), "{rule:?}: Rotate");
        assert!(close(area(&extrude(&tip, rule, b, Extrude::default(), front()).unwrap()), 150.0, 1e-6), "{rule:?}: Extrude's cap");
    }
    // An even-odd triangular hole touching its square's edge.
    let hole =
        PathData::new(vec![poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]), poly(&[(0.0, 50.0), (60.0, 0.0 + 20.0), (60.0, 80.0)])]);
    let b = hole.bounds().unwrap();
    assert!(close(area(&plane(&hole, FillRule::EvenOdd, b, front()).unwrap()), 10000.0 - 1800.0, 1e-6));
}

/// An open subpath is closed by a straight line, whichever way the profile goes: the same as the
/// renderer's fill, whether or not other contours meet.
#[test]
fn an_open_subpath_closes_straight_whether_or_not_contours_meet() {
    let open = vectorcraft_geom::SubPath {
        anchors: vec![
            vectorcraft_geom::Anchor {
                p: Point::new(0.0, 0.0),
                h_in: Point::new(0.0, -50.0),
                h_out: Point::new(0.0, 0.0),
                kind: vectorcraft_geom::AnchorKind::Corner,
            },
            vectorcraft_geom::Anchor::corner(Point::new(50.0, 100.0)),
            vectorcraft_geom::Anchor {
                p: Point::new(100.0, 0.0),
                h_in: Point::new(100.0, 0.0),
                h_out: Point::new(100.0, -50.0),
                kind: vectorcraft_geom::AnchorKind::Corner,
            },
        ],
        closed: false,
    };
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| {
        vectorcraft_geom::SubPath::polyline(&[Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)], true)
    };
    let alone = PathData::new(vec![open.clone()]);
    let with_bars = PathData::new(vec![open, rect(200.0, 40.0, 260.0, 60.0), rect(220.0, 20.0, 240.0, 80.0)]);
    let bars = 60.0 * 20.0 + 20.0 * 40.0;
    assert!(close(area(&plane(&alone, FillRule::NonZero, alone.bounds().unwrap(), front()).unwrap()), 5000.0, 1e-6));
    assert!(close(area(&plane(&with_bars, FillRule::NonZero, with_bars.bounds().unwrap(), front()).unwrap()), 5000.0 + bars, 1e-6));
}

/// A flat contour beside separate ones fills nothing and doesn't send the profile through the
/// boolean; contours far from the origin meet within the boolean's wider snapping; and a merged
/// outline past the point limit is too complex.
#[test]
fn flat_far_and_too_large_merges() {
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| {
        vectorcraft_geom::SubPath::polyline(&[Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)], true)
    };
    let squares: Vec<_> = (0..12).map(|i| rect(i as f64 * 20.0, 0.0, i as f64 * 20.0 + 10.0, 10.0)).collect();
    let line = vectorcraft_geom::SubPath::polyline(&[Point::new(0.0, 300.0), Point::new(50.0, 300.0), Point::new(100.0, 300.0)], false);
    let with_line = PathData::new(squares.iter().cloned().chain([line]).collect());
    assert!(!rings::meets(&rings::sample(&with_line, 300.0, usize::MAX, rings::CURVE_PIECES).unwrap(), &rings::closed_straight(&with_line)));
    assert!(close(area(&plane(&with_line, FillRule::NonZero, with_line.bounds().unwrap(), front()).unwrap()), 1200.0, 1e-6));
    // Far from the origin the boolean snaps wider than 1e-6 (1.4e-6 at 9.9e7), and so does the
    // test for meeting: squares 1.35e-6 apart there are one outline, one cap of both areas.
    let far = PathData::new(vec![rect(9.9e7, 0.0, 9.9e7 + 10.0, 10.0), rect(9.9e7 + 10.0 + 1.35e-6, 0.0, 9.9e7 + 20.0, 10.0)]);
    let cap = plane(&far, FillRule::NonZero, far.bounds().unwrap(), front()).unwrap();
    assert_eq!(cap.iter().map(|f| f.contours.len()).sum::<usize>(), 1, "joined into one outline");
    assert!(close(area(&cap), 200.0, 1e-3), "joined, not lost");
    // Handles count too: a lobe whose handles reach 1.3e8 widens the boolean's snapping to 1.85e-6.
    let lobe = vectorcraft_geom::SubPath {
        anchors: vec![
            vectorcraft_geom::Anchor {
                p: Point::new(0.0, 1000.0),
                h_in: Point::new(0.0, 1000.0),
                h_out: Point::new(1.3e8, 1000.0),
                kind: vectorcraft_geom::AnchorKind::Corner,
            },
            vectorcraft_geom::Anchor {
                p: Point::new(0.0, 1100.0),
                h_in: Point::new(1.3e8, 1100.0),
                h_out: Point::new(0.0, 1100.0),
                kind: vectorcraft_geom::AnchorKind::Corner,
            },
        ],
        closed: true,
    };
    let wide = PathData::new(vec![rect(9.9e7, 0.0, 9.9e7 + 10.0, 10.0), rect(9.9e7 + 10.0 + 1.6e-6, 0.0, 9.9e7 + 20.0, 10.0), lobe]);
    let b = wide.bounds().unwrap();
    let joined = plane(&wide, FillRule::NonZero, b, front()).unwrap();
    assert_eq!(joined.iter().map(|f| f.contours.len()).sum::<usize>(), 2, "the squares joined, the lobe apart");
    // 24 bars each way: 192 points as drawn, some 2,400 once every hole between them is traced,
    // within the 65,536 points Extrude & Bevel, Inflate and Rotate take.
    let mut bars = vec![];
    for i in 0..24 {
        let x = i as f64 * 10.0;
        bars.push(rect(x, 0.0, x + 4.0, 240.0));
        bars.push(rect(0.0, x, 240.0, x + 4.0));
    }
    let grid = PathData::new(bars);
    let b = grid.bounds().unwrap();
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        // With a limit of 2,048 points the grid fits as drawn and its merged outline does not: its
        // contours as drawn cross, so they can't be built as they are.
        assert!(rings::sample(&grid, 240.0, 2048, rings::CURVE_PIECES).is_ok());
        assert_eq!(
            rings::solid_rings(&grid, rule, 240.0, 2048, rings::CURVE_PIECES),
            Err("The shape is too complex for a 3D effect".into()),
            "{rule:?}"
        );
        let expected = vectorcraft_pathops::area(&grid, rule);
        assert!(close(area(&extrude(&grid, rule, b, Extrude::default(), front()).unwrap()), expected, 1e-6), "{rule:?}: Extrude & Bevel");
        assert!(!inflate(&grid, rule, b, Inflate::default(), front()).unwrap().is_empty(), "{rule:?}: Inflate");
        assert!(close(area(&plane(&grid, rule, b, front()).unwrap()), expected, 1e-6), "{rule:?}: Rotate");
    }
}

/// A profile goes through the boolean when its contours come within twice the boolean's tolerance
/// of the path it is given: pathops snaps anchors onto nearby heights before sweeping, then the
/// sweep joins edges within its tolerance, so contours a little more than it apart can come out
/// as one.
#[test]
fn meets_within_twice_the_boolean_tolerance() {
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| {
        vectorcraft_geom::SubPath::polyline(&[Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)], true)
    };
    let meets = |p: &PathData| rings::meets(&rings::sample(p, 300.0, usize::MAX, rings::CURVE_PIECES).unwrap(), &rings::closed_straight(p));
    // Two triangles whose facing 45° edges are 1.4e-6 apart, the second's corners 0.99e-6 lower on
    // the page (greater y) than the first's: snapped onto the first's heights, they are joined.
    let d = 0.99e-6;
    let snapped = PathData::new(vec![
        vectorcraft_geom::SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(0.0, 10.0)], true),
        vectorcraft_geom::SubPath::polyline(&[Point::new(10.0 + d, d), Point::new(10.0 + d, 10.0 + d), Point::new(d, 10.0 + d)], true),
    ]);
    assert!(meets(&snapped));
    let cap = plane(&snapped, FillRule::NonZero, snapped.bounds().unwrap(), front()).unwrap();
    assert_eq!(cap.iter().map(|f| f.contours.len()).sum::<usize>(), 1, "joined into one outline");
    assert!(close(area(&cap), 100.0, 1e-3), "joined, not lost");
    // At the same heights the boolean leaves the triangles apart, and the shape it builds is the
    // one drawn: both triangles, two outlines.
    let tri = |g: f64| {
        PathData::new(vec![
            vectorcraft_geom::SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(0.0, 10.0)], true),
            vectorcraft_geom::SubPath::polyline(&[Point::new(10.0 + g, 0.0), Point::new(10.0 + g, 10.0), Point::new(g, 10.0)], true),
        ])
    };
    let apart = tri(1.4e-6 * std::f64::consts::SQRT_2);
    assert!(meets(&apart));
    assert_eq!(vectorcraft_pathops::try_unite_all_with(&[(&apart, FillRule::NonZero)], rings::DEBRIS).map(|p| p.subpaths.len()), Ok(2));
    let cap = plane(&apart, FillRule::NonZero, apart.bounds().unwrap(), front()).unwrap();
    assert_eq!(cap.iter().map(|f| f.contours.len()).sum::<usize>(), 2);
    assert!(close(area(&cap), 100.0, 1e-3), "both kept");
    // Twice the tolerance, not less: 45° edges 1.9e-6 apart still meet.
    assert!(meets(&tri(1.9e-6 * std::f64::consts::SQRT_2)));
    // And pathops joins nothing past it: the worst case, 2.05e-6 apart as `meets` measures them,
    // with the second triangle's corners just under the tolerance lower so that the snap brings it
    // closest, stays two outlines.
    let lift = 0.999e-6;
    let g = 2.05e-6 * std::f64::consts::SQRT_2 - lift;
    let past = PathData::new(vec![
        vectorcraft_geom::SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(0.0, 10.0)], true),
        vectorcraft_geom::SubPath::polyline(&[Point::new(10.0 + g, lift), Point::new(10.0 + g, 10.0 + lift), Point::new(g, 10.0 + lift)], true),
    ]);
    assert!(!meets(&past));
    assert_eq!(vectorcraft_pathops::try_unite_all_with(&[(&past, FillRule::NonZero)], rings::DEBRIS).map(|p| p.subpaths.len()), Ok(2));
    // Near the origin, contours a few millionths apart stay apart and skip the boolean.
    assert!(!meets(&PathData::new(vec![rect(0.0, 0.0, 10.0, 10.0), rect(10.0 + 2.2e-6, 0.0, 20.0, 10.0)])));
    // What the boolean is given decides the tolerance: an open subpath's end handles (closed
    // straight) and a one-anchor subpath (skipped) don't widen it.
    let ends = vectorcraft_geom::SubPath {
        anchors: vec![
            vectorcraft_geom::Anchor {
                p: Point::new(0.0, 0.0),
                h_in: Point::new(1e9, 0.0),
                h_out: Point::new(0.0, 0.0),
                kind: vectorcraft_geom::AnchorKind::Corner,
            },
            vectorcraft_geom::Anchor {
                p: Point::new(10.0, 0.0),
                h_in: Point::new(10.0, 0.0),
                h_out: Point::new(1e9, 0.0),
                kind: vectorcraft_geom::AnchorKind::Corner,
            },
        ],
        closed: false,
    };
    let speck = vectorcraft_geom::SubPath { anchors: vec![vectorcraft_geom::Anchor::corner(Point::new(1e9, 1e9))], closed: true };
    let quiet = PathData::new(vec![ends, speck]);
    assert_eq!(rings::touch(&rings::closed_straight(&quiet)), 1e-6);
    // A closing-side handle at either end alone is enough to close the subpath straight.
    for kept in [0, 1] {
        let mut one = quiet.subpaths[0].clone();
        one.anchors[0].h_in = if kept == 0 { Point::new(1e9, 0.0) } else { one.anchors[0].p };
        one.anchors[1].h_out = if kept == 1 { Point::new(1e9, 0.0) } else { one.anchors[1].p };
        assert_eq!(rings::touch(&rings::closed_straight(&PathData::new(vec![one]))), 1e-6, "handle kept at anchor {kept}");
    }
    // Beside two squares 3e-6 apart near the origin (farther apart than twice 1e-6), they don't
    // send the profile through the boolean; read unclosed, the end handles would.
    let mut quiet_squares = quiet.clone();
    quiet_squares.subpaths.extend([rect(0.0, 100.0, 10.0, 110.0), rect(10.0 + 3e-6, 100.0, 20.0, 110.0)]);
    assert!(!meets(&quiet_squares));
}

/// The boolean's rings are turned as a whole so the filled side is on their left: holes stay
/// against their outlines, and a profile wound the other way round is turned over.
#[test]
fn the_merged_rings_are_turned_as_a_whole() {
    let outline = vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)];
    let hole = vec![Point::new(2.0, 2.0), Point::new(2.0, 8.0), Point::new(8.0, 8.0), Point::new(8.0, 2.0)];
    let turned = rings::wound::<Vec<Point>>(vec![outline.iter().rev().copied().collect(), hole.iter().rev().copied().collect()]);
    assert!(rings::signed_area(&turned[0]) > 0.0 && rings::signed_area(&turned[1]) < 0.0);
    let kept = rings::wound(vec![outline.clone(), hole.clone()]);
    assert_eq!(kept, vec![outline, hole]);
}

/// A subpath of one anchor draws nothing, so it builds nothing, beside other art or not.
#[test]
fn a_one_anchor_subpath_builds_nothing() {
    let loop_ = vectorcraft_geom::SubPath {
        anchors: vec![vectorcraft_geom::Anchor {
            p: Point::new(0.0, 0.0),
            h_in: Point::new(-20.0, 30.0),
            h_out: Point::new(20.0, 30.0),
            kind: vectorcraft_geom::AnchorKind::Corner,
        }],
        closed: true,
    };
    let p = PathData::new(vec![loop_]);
    let b = Rect::new(-20.0, 0.0, 20.0, 30.0);
    assert_eq!(area(&plane(&p, FillRule::NonZero, b, front()).unwrap()), 0.0);
}
