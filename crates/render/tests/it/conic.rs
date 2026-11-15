//! Conic gradients: the pixels sweep clockwise around the center from the vector, with a hard seam
//! where the last stop meets the first, and follow the model's parameter when the extent ellipse
//! is squashed, turned or mirrored, or holds numbers out of range.

use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, Document};
use vectorcraft_geom::{Point, Rect, shapes};
use vectorcraft_testkit::fixtures::DocBuilder;
use vectorcraft_testkit::raster::{Image, render_view};

/// A 100 pt square filled with `gradient` placed on `geom`.
fn square(gradient: Gradient, geom: GradientGeom) -> Document {
    let mut gp = GradientPaint::new(gradient);
    gp.geom = Some(geom);
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(Paint::Gradient(Box::new(gp)), Paint::None, 0.0), |_| {});
    b.build()
}

/// A white-to-black conic on `geom`.
fn conic(geom: GradientGeom) -> Document {
    square(Gradient { kind: GradientKind::Conic, ..Gradient::default() }, geom)
}

/// Centered on the square, the sweep starting at `deg` (counterclockwise from the right).
fn at(deg: f64) -> GradientGeom {
    GradientGeom::fit(GradientKind::Conic, Rect::new(0.0, 0.0, 100.0, 100.0), deg)
}

/// Every 7th pixel away from the center and the seam has the white-to-black ramp's gray at the
/// model's parameter there (pixel centers).
fn assert_follows_model(img: &Image, geom: &GradientGeom) {
    for y in (3..100).step_by(7) {
        for x in (3..100).step_by(7) {
            let p = Point::new(x as f64 + 0.5, y as f64 + 0.5);
            let t = geom.param_at(GradientKind::Conic, p);
            if p.distance(geom.start) < 6.0 || !(0.02..0.98).contains(&t) {
                continue;
            }
            let want = ((1.0 - t) * 255.0).round() as u8;
            let got = img.over_white(x, y)[0];
            assert!(got.abs_diff(want) <= 6, "({x},{y}) t={t:.3}: got {got}, want {want}");
        }
    }
}

#[test]
fn the_sweep_runs_clockwise_from_the_vector() {
    for deg in [0.0, 90.0, 180.0, 270.0, 33.0] {
        let geom = at(deg);
        let img = render_view(&conic(geom), 100, 100);
        assert_follows_model(&img, &geom);
    }
    // From a vector pointing right: a quarter turn on (below the center) is three quarters white,
    // three quarters on (above it) a quarter white.
    let img = render_view(&conic(at(0.0)), 100, 100);
    assert!(img.over_white(50, 80)[0].abs_diff(191) <= 6, "{:?}", img.over_white(50, 80));
    assert!(img.over_white(50, 20)[0].abs_diff(64) <= 6, "{:?}", img.over_white(50, 20));
}

#[test]
fn the_seam_is_hard_where_the_last_stop_meets_the_first() {
    let img = render_view(&conic(at(0.0)), 100, 100);
    // Along the vector (right of the center): white just below it, black just above it.
    for x in [70, 80, 95] {
        assert!(img.over_white(x, 52)[0] > 235, "x {x} below: {:?}", img.over_white(x, 52));
        assert!(img.over_white(x, 47)[0] < 20, "x {x} above: {:?}", img.over_white(x, 47));
    }
    // Left of the center the ramp runs on smoothly.
    for x in [5, 20, 30] {
        assert!(img.over_white(x, 47)[0].abs_diff(img.over_white(x, 52)[0]) < 20, "x {x}");
    }
}

#[test]
fn a_squashed_turned_or_mirrored_extent_renders_as_its_model() {
    let squashed = GradientGeom { start: Point::new(45.0, 55.0), end: Point::new(75.0, 70.0), aspect: 0.4, focal: None };
    assert_follows_model(&render_view(&conic(squashed), 100, 100), &squashed);
    // A negative aspect turns the sweep counterclockwise: a quarter turn on is above the center.
    let mirrored = GradientGeom { aspect: -1.0, ..at(0.0) };
    let img = render_view(&conic(mirrored), 100, 100);
    assert_follows_model(&img, &mirrored);
    assert!(img.over_white(50, 20)[0].abs_diff(191) <= 6, "{:?}", img.over_white(50, 20));
}

#[test]
fn stop_colors_and_opacity_follow_the_ramp() {
    let mut clear_blue = GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0));
    clear_blue.opacity = 0.0;
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), clear_blue];
    let img = render_view(&square(Gradient::new(GradientKind::Conic, stops), at(0.0)), 100, 100);
    // Just past the start: opaque red. Just before the seam: nearly clear, the paper showing.
    let start = img.over_white(90, 52);
    assert!(start[0] > 235 && start[1] < 30 && start[2] < 30, "{start:?}");
    assert!(img.over_white(90, 47).iter().all(|&c| c > 200), "{:?}", img.over_white(90, 47));
}

/// A conic whose extent a file left without length, flat or out of range draws as the model
/// samples it: a sweep about its start from its vector's direction, or from the right when the
/// vector has no length.
#[test]
fn a_conic_without_an_extent_sweeps_from_its_vector() {
    let c = Point::new(50.0, 50.0);
    for geom in [
        GradientGeom { start: c, end: c, aspect: 1.0, focal: None },
        GradientGeom { start: c, end: Point::new(80.0, 20.0), aspect: 0.0, focal: None },
        GradientGeom { start: c, end: Point::new(80.0, 20.0), aspect: f64::NAN, focal: None },
        GradientGeom { start: c, end: Point::new(80.0, 20.0), aspect: f64::INFINITY, focal: None },
        GradientGeom { start: c, end: c, aspect: -1.0, focal: None },
    ] {
        let img = render_view(&conic(geom), 100, 100);
        assert_follows_model(&img, &geom);
    }
    // Without a vector, from the right: a quarter turn on is below the center.
    let img = render_view(&conic(GradientGeom { start: c, end: c, aspect: 1.0, focal: None }), 100, 100);
    assert!(img.over_white(50, 80)[0].abs_diff(191) <= 6, "{:?}", img.over_white(50, 80));
}
