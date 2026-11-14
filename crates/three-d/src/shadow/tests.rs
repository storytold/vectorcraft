use vectorcraft_geom::{BezPath, FillRule, PathData, Point, Rect, Shape};

use super::*;
use crate::{Extrude, Inflate, Light, Lights, Revolve, View};

fn square(x: f64, y: f64, s: f64) -> PathData {
    let mut b = BezPath::new();
    b.move_to((x, y));
    b.line_to((x + s, y));
    b.line_to((x + s, y + s));
    b.line_to((x, y + s));
    b.close_path();
    PathData::from_bezpath(&b)
}

/// The view straight on: no rotation, the default light.
fn front() -> View {
    View { rotation_x: 0.0, rotation_y: 0.0, rotation_z: 0.0, ..View::default() }
}

fn extruded(view: View, shadow: Shadow) -> Option<PathData> {
    let path = square(100.0, 100.0, 100.0);
    let mut caster = ShadowCaster::new();
    caster.extrude(&path, FillRule::NonZero, Rect::new(100.0, 100.0, 200.0, 200.0), Extrude::default(), view).unwrap();
    caster.path(shadow)
}

/// The area the shadow covers, sampled on a grid over `r` with the nonzero rule.
fn covered(path: &PathData, r: Rect, steps: usize) -> f64 {
    let bp = path.to_bezpath();
    let (dx, dy) = (r.width() / steps as f64, r.height() / steps as f64);
    let mut hits = 0;
    for i in 0..steps {
        for j in 0..steps {
            let p = Point::new(r.x0 + (i as f64 + 0.5) * dx, r.y0 + (j as f64 + 0.5) * dy);
            if bp.winding(p) != 0 {
                hits += 1;
            }
        }
    }
    hits as f64 * dx * dy
}

#[test]
fn a_light_straight_in_front_casts_the_silhouette_behind_the_object() {
    let view = View { light_height: 90.0, ..front() };
    let path = extruded(view, Shadow::default()).expect("a shadow");
    let b = path.bounds().unwrap();
    for (got, want) in [(b.x0, 100.0), (b.y0, 100.0), (b.x1, 200.0), (b.y1, 200.0)] {
        assert!((got - want).abs() < 1e-6, "{b:?}");
    }
    // Every face of the box projects into the square, and the nonzero union fills it whole.
    let area = covered(&path, Rect::new(90.0, 90.0, 210.0, 210.0), 120);
    assert!((area - 10_000.0).abs() < 300.0, "{area}");
}

#[test]
fn the_shadow_behind_falls_away_from_the_light_and_moves_with_the_distance() {
    // The default light is up and to the left, in front: the shadow falls down and to the right.
    let near = extruded(front(), Shadow::default()).unwrap().bounds().unwrap();
    assert!(near.x1 > 200.0 + 1.0 && near.y1 > 200.0 + 1.0, "{near:?}");
    assert!(near.x0 >= 100.0 - 1e-6 && near.y0 >= 100.0 - 1e-6, "{near:?}");
    let far = extruded(front(), Shadow { distance: 1.0, ..Shadow::default() }).unwrap().bounds().unwrap();
    assert!(far.x1 > near.x1 + 10.0 && far.y1 > near.y1 + 10.0, "{near:?} {far:?}");
}

#[test]
fn a_light_behind_the_object_casts_no_shadow_behind_it() {
    assert!(extruded(View { light_behind: true, ..front() }, Shadow::default()).is_none());
    let behind = Light { behind: true, ..Light::default() };
    let lights = Lights::new(&[behind, Light::default()]).unwrap();
    // The first light casts the shadow.
    assert!(extruded(View { lights: Some(lights), ..front() }, Shadow::default()).is_none());
    let lights = Lights::new(&[Light::default(), behind]).unwrap();
    assert!(extruded(View { lights: Some(lights), ..front() }, Shadow::default()).is_some());
}

#[test]
fn a_light_that_lights_nothing_casts_no_shadow() {
    assert!(extruded(View { light_intensity: 0.0, ..front() }, Shadow::default()).is_none());
    assert!(extruded(View { shade: false, ..front() }, Shadow::default()).is_none());
    let dark = Light { intensity: 0.0, ..Light::default() };
    let lights = Lights::new(&[dark, Light::default()]).unwrap();
    assert!(extruded(View { lights: Some(lights), ..front() }, Shadow::default()).is_none());
}

#[test]
fn the_shadow_below_lies_under_the_object_on_its_floor() {
    let below = Shadow { position: ShadowPosition::Below, ..Shadow::default() };
    // Seen level, the floor is edge-on: nothing to draw.
    assert!(extruded(front(), below).is_none());
    // Seen from above (the effects' default Off-Axis Front view), the floor shows.
    let off_axis = View { rotation_x: -21.88, rotation_y: -23.4, rotation_z: 9.06, ..View::default() };
    assert!(extruded(off_axis, below).is_some());
    // Seen from straight above, the floor is square to the view, under the object: with the light
    // straight in front too, the shadow is the object's outline, as the one behind it is.
    let top = View { rotation_x: -90.0, light_height: 90.0, ..front() };
    let under = extruded(top, below).unwrap().bounds().unwrap();
    let back = extruded(top, Shadow::default()).unwrap().bounds().unwrap();
    for (a, b) in [(under.x0, back.x0), (under.y0, back.y0), (under.x1, back.x1), (under.y1, back.y1)] {
        assert!((a - b).abs() < 1e-6, "{under:?} {back:?}");
    }
    // A light from below the floor casts nothing on it.
    let from_below = View { light_rotation: -90.0, light_height: 0.0, ..off_axis };
    assert!(extruded(from_below, below).is_none());
}

#[test]
fn shadow_bounds_limit_the_plane_the_shadow_falls_on() {
    // A low light casts a long shadow; the plane cuts it.
    let low = View { light_height: 5.0, ..front() };
    let small = extruded(low, Shadow { bounds: 0.1, ..Shadow::default() }).unwrap().bounds().unwrap();
    let large = extruded(low, Shadow { bounds: 4.0, ..Shadow::default() }).unwrap().bounds().unwrap();
    // At 10 % the square reaches 0.2 × the size (100) from the foot of the center (150, 150).
    for v in [small.x0, small.y0, small.x1, small.y1] {
        assert!((130.0 - 1e-6..=170.0 + 1e-6).contains(&v), "{small:?}");
    }
    assert!(large.width() > small.width() * 5.0, "{small:?} {large:?}");
    for v in [large.x0, large.y0, large.x1, large.y1] {
        assert!((-650.0 - 1e-6..=950.0 + 1e-6).contains(&v), "{large:?}");
    }
}

#[test]
fn bad_settings_are_sanitized_and_an_empty_caster_casts_nothing() {
    assert!(ShadowCaster::new().path(Shadow::default()).is_none());
    let wild = Shadow { distance: f64::NAN, bounds: f64::INFINITY, ..Shadow::default() };
    assert_eq!(wild.sanitized(), Shadow { bounds: 1.0, ..Shadow::default() });
    assert_eq!(Shadow { distance: -3.0, bounds: 99.0, ..Shadow::default() }.sanitized(), Shadow { bounds: 4.0, ..Shadow::default() });
    assert_eq!(ShadowPosition::parse("below"), Some(ShadowPosition::Below));
    assert_eq!(ShadowPosition::parse("under"), None);
}

#[test]
fn the_faces_one_shadow_is_cast_from_are_bounded() {
    // An open profile of 500 points revolves into about 64,000 faces at 128 segments: five fit,
    // the sixth does not.
    let mut b = BezPath::new();
    b.move_to((110.0, 0.0));
    for i in 1..500 {
        b.line_to((110.0 + f64::from(i % 2) * 5.0, f64::from(i)));
    }
    let path = PathData::from_bezpath(&b);
    let bounds = path.bounds().unwrap();
    let options = Revolve { segments: 128, offset: 10.0, ..Revolve::default() };
    let mut caster = ShadowCaster::new();
    for _ in 0..5 {
        let faces = caster.revolve(&path, FillRule::NonZero, bounds, options).unwrap();
        assert!(faces.len() > 60_000, "{faces:?}");
    }
    let err = caster.revolve(&path, FillRule::NonZero, bounds, options).unwrap_err();
    assert!(err.contains("too complex"), "{err}");
    assert!(caster.len() <= MAX_SHADOW_FACES);
    assert!(caster.path(Shadow::default()).is_some());
}

#[test]
fn a_small_solid_beside_a_huge_one_keeps_its_shadow() {
    let bounds = Rect::new(0.0, 0.0, 1_000_010.0, 10.0);
    let view = View { light_height: 90.0, ..front() };
    let mut caster = ShadowCaster::new();
    let small = caster.extrude(&square(0.0, 0.0, 10.0), FillRule::NonZero, bounds, Extrude::default(), view).unwrap();
    let huge = caster.extrude(&square(1_000_000.0, 0.0, 10.0), FillRule::NonZero, bounds, Extrude::default(), view).unwrap();
    let all = [small.clone(), huge];
    let plane = caster.shadow_plane(Shadow { bounds: 4.0, ..Shadow::default() }, &all).unwrap();
    let path = caster.path_on(&plane, std::slice::from_ref(&small)).expect("the small solid's shadow");
    let b = path.bounds().unwrap();
    assert!((b.width() - 10.0).abs() < 1e-3 && (b.height() - 10.0).abs() < 1e-3, "{b:?}");
}

#[test]
fn every_effect_casts_a_shadow() {
    let path = square(100.0, 100.0, 100.0);
    let bounds = Rect::new(100.0, 100.0, 200.0, 200.0);
    let mut plane = ShadowCaster::new();
    plane.plane(&path, FillRule::NonZero, bounds, View::default()).unwrap();
    assert!(plane.path(Shadow::default()).is_some());
    let mut inflate = ShadowCaster::new();
    let results = inflate.inflate(&[(&path, FillRule::NonZero)], &[true], bounds, Inflate::default(), View::default());
    assert!(results.iter().all(Result::is_ok));
    assert!(inflate.path(Shadow::default()).is_some());
    let mut revolve = ShadowCaster::new();
    revolve.revolve(&path, FillRule::NonZero, bounds, Revolve::default()).unwrap();
    let b = revolve.path(Shadow::default()).unwrap().bounds().unwrap();
    // The surface turns about the square's left edge, reaching 100 to either side of it.
    assert!(b.x1 > 100.0 && b.x0 < 100.0 + 1e-6, "{b:?}");
}

/// The faces `caster` gathered and the projection it keeps, every number to the bit.
fn gathered(caster: &ShadowCaster) -> String {
    format!("{:?} {:?}", caster.faces, caster.projection)
}

/// What a solid kept for its shadow casts ([`casts`]).
fn cast_from(solid: SolidFaces) -> Result<(String, Option<PathData>, Option<PathData>), String> {
    let mut caster = ShadowCaster::new();
    caster.add_solid(solid)?;
    Ok(casts(&caster))
}

/// The faces `caster` gathered, and the shadows they cast behind and below.
fn casts(caster: &ShadowCaster) -> (String, Option<PathData>, Option<PathData>) {
    let below = Shadow { position: ShadowPosition::Below, distance: 0.3, ..Shadow::default() };
    (gathered(caster), caster.path(Shadow::default()), caster.path(below))
}

/// A star of `points` points about (100, 100), its edges curved, with a square hole.
fn star(points: usize) -> PathData {
    let mut b = BezPath::new();
    for k in 0..2 * points {
        let (s, c) = (k as f64 * std::f64::consts::PI / points as f64).sin_cos();
        let r = if k % 2 == 0 { 90.0 } else { 45.0 };
        let p = (100.0 + r * c, 100.0 + r * s);
        if k == 0 { b.move_to(p) } else { b.curve_to((p.0 + 3.0, p.1), (p.0, p.1 + 3.0), p) }
    }
    b.close_path();
    rect(&mut b, 90.0, 90.0, 20.0, 20.0);
    PathData::from_bezpath(&b)
}

/// A closed rectangle from (`x`, `y`), `w` wide and `h` high.
fn rect(b: &mut BezPath, x: f64, y: f64, w: f64, h: f64) {
    b.move_to((x, y));
    b.line_to((x + w, y));
    b.line_to((x + w, y + h));
    b.line_to((x, y + h));
    b.close_path();
}

/// The views the solids are compared in: straight on, turned, with perspective, and with colored
/// lights.
fn views() -> Vec<View> {
    let turned = View { rotation_x: -25.0, rotation_y: 35.0, rotation_z: 10.0, ..View::default() };
    vec![front(), turned, View { perspective: 60.0, ..turned }, View { light_color: [1.0, 0.5, 0.25], ..turned }]
}

/// Extrude & Bevel built once for its facets and its shadow gives the facets [`crate::extrude`]
/// gives and casts what [`ShadowCaster::extrude`] casts, to the bit, errors included.
#[test]
fn an_extruded_solid_and_its_shadow_match_the_two_built_apart() {
    use crate::{Bevel, BevelShape};
    let bevel = Bevel { shape: BevelShape::Rounded, width: 6.0, height: 5.0, ..Bevel::default() };
    let options = [
        Extrude::default(),
        Extrude { depth: 0.0, ..Extrude::default() },
        Extrude { cap: false, ..Extrude::default() },
        Extrude { hidden: true, ..Extrude::default() },
        Extrude { twist: 90.0, taper: 0.6, ..Extrude::default() },
        Extrude { bevel: Some(bevel), ..Extrude::default() },
        Extrude { bevel: Some(Bevel { both_sides: true, inside: true, ..bevel }), twist: 45.0, ..Extrude::default() },
    ];
    let path = star(7);
    let r = path.bounds().unwrap();
    for view in views() {
        for o in options {
            let (facets, faces) = extrude_shadowed(&path, FillRule::NonZero, r, o, view).unwrap();
            let apart = crate::extrude(&path, FillRule::NonZero, r, o, view).unwrap();
            assert_eq!(format!("{facets:?}"), format!("{apart:?}"), "{o:?} {view:?}");
            let mut caster = ShadowCaster::new();
            caster.extrude(&path, FillRule::NonZero, r, o, view).unwrap();
            assert!(!caster.is_empty());
            assert_eq!(cast_from(faces), Ok(casts(&caster)), "{o:?} {view:?}");
        }
    }
    let flat = Rect::new(0.0, 0.0, f64::INFINITY, 10.0);
    let error = crate::extrude(&path, FillRule::NonZero, flat, Extrude::default(), front()).err();
    assert!(error.is_some());
    assert_eq!(extrude_shadowed(&path, FillRule::NonZero, flat, Extrude::default(), front()).err(), error);
}

/// Rotate's plane built once for its facet and its shadow matches the two built apart.
#[test]
fn a_rotated_plane_and_its_shadow_match_the_two_built_apart() {
    let path = star(5);
    let r = path.bounds().unwrap();
    for view in views() {
        for rule in [FillRule::NonZero, FillRule::EvenOdd] {
            let (facets, faces) = plane_shadowed(&path, rule, r, view).unwrap();
            assert_eq!(format!("{facets:?}"), format!("{:?}", crate::plane(&path, rule, r, view).unwrap()), "{view:?}");
            let mut caster = ShadowCaster::new();
            caster.plane(&path, rule, r, view).unwrap();
            assert_eq!(cast_from(faces), Ok(casts(&caster)), "{view:?}");
        }
    }
}

/// Inflate's solids built once for their facets and shadows match the two built apart: several
/// puffs under one eye, both sides, open backs, and twisted sides cut finer than room is left for.
#[test]
fn inflated_solids_and_their_shadows_match_the_two_built_apart() {
    let star = star(5);
    let box_ = square(200.0, 20.0, 60.0);
    let ring: Vec<Point> = (0..600)
        .map(|k| {
            let (s, c) = (k as f64 * std::f64::consts::TAU / 600.0).sin_cos();
            Point::new(100.0 + 90.0 * c, 100.0 + 90.0 * s)
        })
        .collect();
    let ring = PathData::single(vectorcraft_geom::SubPath::polyline(&ring, true));
    let sets = [vec![(&star, FillRule::NonZero), (&box_, FillRule::NonZero), (&star, FillRule::EvenOdd)], vec![(&ring, FillRule::NonZero)]];
    let bounds = Rect::new(0.0, 0.0, 280.0, 200.0);
    let options = [
        Inflate::default(),
        Inflate { depth: 30.0, both_sides: true, ..Inflate::default() },
        Inflate { depth: 20.0, cap: false, volume: 80.0, ..Inflate::default() },
        Inflate { depth: 40.0, twist: 360.0, taper: 0.7, both_sides: true, ..Inflate::default() },
        // Thin and barely twisted: the star's sides want more pieces than room is left after its
        // front and back, and the shadow, which keeps the faces turned away, has less room left.
        Inflate { depth: 0.1, twist: 0.1, both_sides: true, ..Inflate::default() },
    ];
    for shapes in &sets {
        for view in views() {
            for o in options {
                let built: Vec<_> = inflate_shadowed(shapes, bounds, o, view).collect();
                let facets: Vec<_> = crate::inflate_each(shapes, bounds, o, view).collect();
                let mut caster = ShadowCaster::new();
                let added = caster.inflate(shapes, &vec![true; shapes.len()], bounds, o, view);
                assert!(added.iter().all(Result::is_ok));
                assert_eq!(built.len(), shapes.len());
                let mut one = ShadowCaster::new();
                for (b, f) in built.into_iter().zip(&facets) {
                    let ((facets, faces), f) = (b.unwrap(), f.as_ref().unwrap());
                    assert_eq!(format!("{facets:?}"), format!("{f:?}"), "{o:?} {view:?}");
                    one.add_solid(faces).unwrap();
                }
                assert_eq!(casts(&one), casts(&caster), "{o:?} {view:?}");
            }
        }
    }
}

/// Revolve's surface swept once for its faces and its shadow gives the faces [`crate::revolve`]
/// gives and casts what [`ShadowCaster::revolve`] casts, to the bit: an open profile with caps,
/// closed contours that cross (their walls cut where they do), a hole, partial sweeps, smooth and
/// flat shading, and the cut side that graphics take.
#[test]
fn a_revolved_surface_and_its_shadow_match_the_two_swept_apart() {
    let mut open = BezPath::new();
    open.move_to((120.0, 0.0));
    open.curve_to((160.0, 20.0), (100.0, 60.0), (140.0, 90.0));
    open.line_to((125.0, 120.0));
    let mut crossing = BezPath::new();
    rect(&mut crossing, 110.0, 0.0, 60.0, 80.0);
    rect(&mut crossing, 150.0, 40.0, 40.0, 80.0);
    let mut holed = BezPath::new();
    holed.move_to((110.0, 0.0));
    holed.curve_to((200.0, 0.0), (200.0, 120.0), (110.0, 120.0));
    holed.close_path();
    rect(&mut holed, 130.0, 40.0, 20.0, 30.0);
    let paths = [open, crossing, holed].map(|b| PathData::from_bezpath(&b));
    let turned = Revolve { rotation_x: -20.0, rotation_y: 30.0, ..Revolve::default() };
    let options = [
        Revolve::default(),
        turned,
        Revolve { angle: 200.0, perspective: 40.0, ..turned },
        Revolve { smooth_shading: false, segments: 12, ..turned },
        Revolve { cap_ends: false, offset: 15.0, ..turned },
        Revolve { map_surface: true, edge: crate::Edge::Right, ..turned },
        Revolve { map_surface: true, angle: 300.0, curve_samples: Some(5), ..turned },
    ];
    for path in &paths {
        let bounds = path.bounds().unwrap();
        for o in options {
            let (faces, kept) = revolve_shadowed(path, bounds, o).unwrap();
            assert_eq!(format!("{faces:?}"), format!("{:?}", crate::revolve(path, bounds, o).unwrap()), "{o:?}");
            let (facets, also) = revolve_facets_shadowed(path, bounds, o).unwrap();
            assert_eq!(format!("{facets:?}"), format!("{:?}", crate::revolve_facets(path, bounds, o).unwrap()), "{o:?}");
            assert_eq!(cast_from(also), cast_from(kept.clone()), "{o:?}");
            let mut caster = ShadowCaster::new();
            caster.revolve(path, FillRule::NonZero, bounds, o).unwrap();
            assert!(!caster.is_empty());
            assert_eq!(cast_from(kept), Ok(casts(&caster)), "{o:?}");
        }
    }
}

/// A surface whose shadow is past the limit on the profile's points (its walls cut where they
/// cross another contour) still draws every face, and its shadow gives the error the shadow swept
/// alone gives.
#[test]
fn a_surface_whose_shadow_is_too_complex_still_draws() {
    let mut b = BezPath::new();
    b.move_to((110.0, 0.0));
    for i in 1..6000 {
        b.line_to((110.0 + f64::from(i) * 0.02, if i % 2 == 0 { 0.0 } else { 10.0 }));
    }
    b.line_to((110.0, 20.0));
    b.close_path();
    rect(&mut b, 100.0, 4.0, 140.0, 2.0);
    let path = PathData::from_bezpath(&b);
    let bounds = path.bounds().unwrap();
    let o = Revolve { segments: 4, smooth_shading: false, ..Revolve::default() };
    let (faces, kept) = revolve_shadowed(&path, bounds, o).unwrap();
    let drawn = crate::revolve(&path, bounds, o).unwrap();
    assert!(!drawn.is_empty());
    assert!(faces == drawn);
    let alone = ShadowCaster::new().revolve(&path, FillRule::NonZero, bounds, o).unwrap_err();
    assert!(alone.contains("too complex"), "{alone}");
    assert_eq!(cast_from(kept).unwrap_err(), alone);
}

/// One build gives the facets and the faces of the shadow: each solid is built once.
#[test]
fn a_solid_and_its_shadow_are_built_once() {
    let path = star(5);
    let r = path.bounds().unwrap();
    let count = |n: &'static std::thread::LocalKey<std::cell::Cell<usize>>| n.with(std::cell::Cell::get);
    let start = count(&crate::solid::EXTRUDED);
    let (facets, faces) = extrude_shadowed(&path, FillRule::NonZero, r, Extrude::default(), front()).unwrap();
    assert!(!facets.is_empty() && cast_from(faces).is_ok_and(|c| c.1.is_some()));
    assert_eq!(count(&crate::solid::EXTRUDED), start + 1);
    let start = count(&crate::solid::PLANES);
    let (facets, faces) = plane_shadowed(&path, FillRule::NonZero, r, front()).unwrap();
    assert!(!facets.is_empty() && cast_from(faces).is_ok_and(|c| c.1.is_some()));
    assert_eq!(count(&crate::solid::PLANES), start + 1);
    let start = count(&crate::solid::INFLATED);
    let box_ = square(200.0, 20.0, 60.0);
    let shapes = [(&path, FillRule::NonZero), (&box_, FillRule::NonZero)];
    for solid in inflate_shadowed(&shapes, Rect::new(0.0, 0.0, 280.0, 200.0), Inflate::default(), front()) {
        let (facets, faces) = solid.unwrap();
        assert!(!facets.is_empty() && cast_from(faces).is_ok_and(|c| c.1.is_some()));
    }
    assert_eq!(count(&crate::solid::INFLATED), start + 2);
    let start = count(&crate::REVOLVED);
    let (drawn, faces) = revolve_shadowed(&path, r, Revolve::default()).unwrap();
    assert!(!drawn.is_empty() && cast_from(faces).is_ok_and(|c| c.1.is_some()));
    assert_eq!(count(&crate::REVOLVED), start + 1);
}
