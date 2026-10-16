//! Conic (angle) gradients: the pixels sweep clockwise around the centre from the vector, a hard
//! seam where the last stop meets the first, and follow the model's parameter when the extent is
//! squashed, rotated or mirrored.

use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, Document};
use vectorcraft_geom::{Point, Rect, shapes};
use vectorcraft_testkit::fixtures::DocBuilder;
use vectorcraft_testkit::raster::{Image, render_view};

/// A 100 pt square filled with a white-to-black conic on `geom`.
fn conic(geom: GradientGeom) -> Document {
    let mut gp = GradientPaint::new(Gradient { kind: GradientKind::Conic, ..Gradient::default() });
    gp.geom = Some(geom);
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(Paint::Gradient(Box::new(gp)), Paint::None, 0.0), |_| {});
    b.build()
}

/// Centred on the square, the sweep starting at `deg` (counter-clockwise from the right).
fn at(deg: f64) -> GradientGeom {
    GradientGeom::fit(GradientKind::Conic, Rect::new(0.0, 0.0, 100.0, 100.0), deg)
}

fn red(img: &Image, x: u32, y: u32) -> u8 {
    img.over_white(x, y)[0]
}

/// Grey (red channel) the white-to-black ramp has at turn `t`.
fn grey(t: f64) -> u8 {
    ((1.0 - t) * 255.0).round() as u8
}

/// Every 7th pixel away from the seam and the centre matches the model's parameter there.
fn assert_follows_model(img: &Image, geom: &GradientGeom) {
    for y in (3..100).step_by(7) {
        for x in (3..100).step_by(7) {
            let p = Point::new(x as f64 + 0.5, y as f64 + 0.5);
            if p.distance(geom.start) < 6.0 {
                continue;
            }
            let t = geom.param_at(GradientKind::Conic, p);
            if !(0.02..0.98).contains(&t) {
                continue;
            }
            let got = red(img, x, y);
            assert!(got.abs_diff(grey(t)) <= 6, "({x},{y}) t={t:.3}: got {got}, want {}", grey(t));
        }
    }
}

#[test]
fn the_sweep_runs_clockwise_from_the_vector_at_0_90_180_and_270_degrees() {
    // (dx, dy) on the page from the centre, and the turn clockwise from a vector pointing right.
    let probes = [(30i32, 0i32, 0.0f64), (0, 30, 0.25), (-30, 0, 0.5), (0, -30, 0.75)];
    for (k, deg) in [0.0, 90.0, 180.0, 270.0].into_iter().enumerate() {
        let img = render_view(&conic(at(deg)), 100, 100);
        assert_follows_model(&img, &at(deg));
        // A vector at `deg` counter-clockwise starts the sweep k quarter turns back.
        for (dx, dy, t) in probes {
            if (dx, dy, t) == probes[(4 - k) % 4] {
                continue; // On the seam.
            }
            let t = (t + 0.25 * k as f64).fract();
            // Half a pixel past the probe direction, so the probe sits clearly on one side.
            let (x, y) = ((50 + dx) as u32, (50 + dy) as u32);
            let got = red(&img, x, y);
            assert!(got.abs_diff(grey(t)) <= 8, "{deg}°: ({x},{y}) got {got}, want {} (t {t})", grey(t));
        }
    }
}

#[test]
fn a_hard_seam_where_the_last_stop_meets_the_first() {
    let img = render_view(&conic(at(0.0)), 100, 100);
    // Along the vector (right of the centre): white just below it, black just above it.
    for x in [70, 80, 95] {
        assert!(red(&img, x, 52) > 235, "x {x} below: {:?}", img.over_white(x, 52));
        assert!(red(&img, x, 47) < 20, "x {x} above: {:?}", img.over_white(x, 47));
    }
    // Nowhere else does the ramp jump.
    for x in [5, 20, 30] {
        assert!(red(&img, x, 47).abs_diff(red(&img, x, 52)) < 20, "x {x}");
    }
}

#[test]
fn a_squashed_rotated_or_mirrored_extent_renders_as_its_model() {
    let squashed = GradientGeom { start: Point::new(45.0, 55.0), end: Point::new(75.0, 70.0), aspect: 0.4, focal: None };
    assert_follows_model(&render_view(&conic(squashed), 100, 100), &squashed);
    // A negative aspect (a mirrored conic) sweeps counter-clockwise.
    let mirrored = GradientGeom { aspect: -1.0, ..at(0.0) };
    let img = render_view(&conic(mirrored), 100, 100);
    assert_follows_model(&img, &mirrored);
    assert!(red(&img, 50, 20) > 180, "a quarter turn counter-clockwise is light: {:?}", img.over_white(50, 20));
}

#[test]
fn stops_and_opacity_follow_the_ramp() {
    let red_stop = GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0));
    let mut blue = GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0));
    blue.opacity = 0.0;
    let mut gp = GradientPaint::new(Gradient { kind: GradientKind::Conic, stops: vec![red_stop, blue] });
    gp.geom = Some(at(0.0));
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(Paint::Gradient(Box::new(gp)), Paint::None, 0.0), |_| {});
    let img = render_view(&b.build(), 100, 100);
    // Just past the start: opaque red. Just before the seam: nearly transparent (white paper).
    let start = img.over_white(90, 52);
    assert!(start[0] > 235 && start[1] < 30 && start[2] < 30, "{start:?}");
    let end = img.over_white(90, 47);
    assert!(end.iter().take(3).all(|&c| c > 200), "{end:?}");
}
