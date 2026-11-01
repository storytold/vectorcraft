//! Off-centre (focal) radial gradients: the pixels follow the model's gradient parameter, with
//! and without an aspect ratio, and differ from the centred gradient.

use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, Document};
use vectorcraft_geom::{Point, Rect, shapes};
use vectorcraft_testkit::fixtures::DocBuilder;
use vectorcraft_testkit::raster::{Image, render_view};

/// A 100 pt square filled with a white-to-black radial on `geom`.
fn radial(geom: GradientGeom) -> Document {
    let mut gp = GradientPaint::new(Gradient { kind: GradientKind::Radial, ..Gradient::default() });
    gp.geom = Some(geom);
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(Paint::Gradient(Box::new(gp)), Paint::None, 0.0), |_| {});
    b.build()
}

/// A circle of radius 40 at (50, 50) with its focal point at `focal`.
fn off_centre(focal: Option<Point>) -> GradientGeom {
    let mut g = GradientGeom { start: Point::new(50.0, 50.0), end: Point::new(90.0, 50.0), aspect: 1.0, focal: None };
    g.set_focal(focal);
    g
}

/// Every 7th pixel's red matches white→black at the model's parameter there (pixel centres).
fn assert_follows_model(img: &Image, geom: &GradientGeom) {
    for y in (3..100).step_by(7) {
        for x in (3..100).step_by(7) {
            let t = geom.param_at(GradientKind::Radial, Point::new(x as f64 + 0.5, y as f64 + 0.5)).clamp(0.0, 1.0);
            let want = ((1.0 - t) * 255.0).round() as u8;
            let got = img.over_white(x, y)[0];
            assert!(got.abs_diff(want) <= 6, "({x},{y}) t={t:.3}: got {got}, want {want}");
        }
    }
}

#[test]
fn an_off_centre_focal_point_renders_as_its_model() {
    let geom = off_centre(Some(Point::new(30.0, 40.0)));
    let img = render_view(&radial(geom), 100, 100);
    assert_follows_model(&img, &geom);
    // White at the focal point, and the light side faces it.
    assert!(img.over_white(30, 40)[0] > 245, "{:?}", img.over_white(30, 40));
    let centred = render_view(&radial(off_centre(None)), 100, 100);
    assert!(img.over_white(35, 50)[0] > centred.over_white(35, 50)[0] + 20);
    assert!(img.over_white(65, 50)[0] + 20 < centred.over_white(65, 50)[0]);
}

#[test]
fn a_focal_point_on_a_squashed_rotated_ellipse_renders_as_its_model() {
    let mut geom = GradientGeom { start: Point::new(50.0, 50.0), end: Point::new(80.0, 75.0), aspect: 0.5, focal: None };
    geom.set_focal(Some(Point::new(60.0, 50.0)));
    assert!(geom.focal.is_some());
    assert_follows_model(&render_view(&radial(geom), 100, 100), &geom);
}
