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

/// An Inflate shape that casts nothing still places the eye with its puff, but its solid is not
/// built (nor, with no volume, its puff solved); the shapes that cast keep the very faces and
/// projection they have when every shape casts.
#[test]
fn an_inflate_shape_that_casts_nothing_builds_no_solid() {
    let (a, b) = (square(100.0, 100.0, 100.0), square(150.0, 150.0, 30.0));
    let shapes = [(&a, FillRule::NonZero), (&b, FillRule::NonZero)];
    let bounds = Rect::new(100.0, 100.0, 200.0, 200.0);
    let view = View { perspective: 60.0, rotation_y: 30.0, ..front() };
    let built = || crate::solid::INFLATED.with(std::cell::Cell::get);
    let puffed = || crate::solid::PUFFED.with(std::cell::Cell::get);
    for o in [Inflate::default(), Inflate { volume: 0.0, depth: 20.0, ..Inflate::default() }] {
        let mut all = ShadowCaster::new();
        let every = all.inflate(&shapes, &[true, true], bounds, o, view);
        let (start, solved) = (built(), puffed());
        let mut some = ShadowCaster::new();
        let kept = some.inflate(&shapes, &[false, true], bounds, o, view);
        assert_eq!(built(), start + 1, "{o:?}: only the casting shape's solid is built");
        if o.volume == 0.0 {
            assert_eq!(puffed(), solved + 1, "{o:?}: only the casting shape's puff is solved");
        }
        let (Ok(first), Ok(second)) = (&every[0], &every[1]) else { panic!("{every:?}") };
        assert!(!first.is_empty() && !second.is_empty());
        assert_eq!(kept, vec![Ok(0..0), Ok(0..second.len())]);
        assert_eq!(format!("{:?}", some.faces), format!("{:?}", &all.faces[second.clone()]));
        assert_eq!(some.projection, all.projection);
        let mut none = ShadowCaster::new();
        assert_eq!(none.inflate(&shapes, &[false, false], bounds, o, view), vec![Ok(0..0), Ok(0..0)]);
        assert!(none.is_empty() && none.projection.is_none());
    }
}
