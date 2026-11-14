//! Diamond gradients: the pixels run from the center out to a square with a corner at the end of
//! the vector, and follow the model's parameter when the square is stretched or turned, on fills
//! and strokes; a stroke gets no dither noise, and a diamond without an extent draws its last stop.

use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, Document};
use vectorcraft_geom::{Point, Rect, Vec2, shapes};
use vectorcraft_testkit::fixtures::DocBuilder;
use vectorcraft_testkit::raster::{Image, render_view};

/// A white-to-black diamond on `geom`, dithered when `dither`.
fn paint(geom: GradientGeom, dither: bool) -> Paint {
    let mut gp = GradientPaint::new(Gradient { kind: GradientKind::Diamond, dither, ..Gradient::default() });
    gp.geom = Some(geom);
    Paint::Gradient(Box::new(gp))
}

/// A 100 pt square filled with `fill` and stroked `width` wide with `stroke`.
fn square(fill: Paint, stroke: Paint, width: f64) -> Document {
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 90.0)), Appearance::basic(fill, stroke, width), |_| {});
    b.build()
}

/// Every 7th pixel inside `inside` has the white-to-black ramp's gray at the model's parameter
/// there (pixel centers).
fn assert_follows_model(img: &Image, geom: &GradientGeom, inside: Rect) {
    for y in (3..100).step_by(7) {
        for x in (3..100).step_by(7) {
            let p = Point::new(x as f64 + 0.5, y as f64 + 0.5);
            if !inside.contains(p) {
                continue;
            }
            let t = geom.param_at(GradientKind::Diamond, p).clamp(0.0, 1.0);
            let want = ((1.0 - t) * 255.0).round() as u8;
            let got = img.over_white(x, y)[0];
            assert!(got.abs_diff(want) <= 6, "({x},{y}) t={t:.3}: got {got}, want {want}");
        }
    }
}

#[test]
fn a_diamond_fill_follows_the_squares() {
    let c = Point::new(50.0, 50.0);
    let level = GradientGeom { start: c, end: c + Vec2::new(40.0, 0.0), aspect: 1.0, focal: None };
    let stretched = GradientGeom { aspect: 0.5, ..level };
    let turned = GradientGeom { end: c + Vec2::new(30.0, 30.0), aspect: 0.7, ..level };
    for geom in [level, stretched, turned] {
        let img = render_view(&square(paint(geom, false), Paint::None, 0.0), 100, 100);
        assert_follows_model(&img, &geom, Rect::new(12.0, 12.0, 88.0, 88.0));
    }
    // Level: white at the center, mid gray halfway to a corner of the square, along an axis and
    // along a diagonal alike (pixel centers 20 pt from the center, counted along the axes).
    let img = render_view(&square(paint(level, false), Paint::None, 0.0), 100, 100);
    assert!(img.over_white(50, 50)[0] > 240, "{:?}", img.over_white(50, 50));
    for (x, y) in [(69, 49), (49, 69), (60, 59), (39, 40)] {
        assert!(img.over_white(x, y)[0].abs_diff(128) <= 6, "({x},{y}): {:?}", img.over_white(x, y));
    }
}

/// A stroke follows the squares too, without the noise dither puts on fills.
#[test]
fn a_diamond_stroke_follows_the_squares_without_dither() {
    let c = Point::new(50.0, 50.0);
    let geom = GradientGeom { start: c, end: c + Vec2::new(60.0, 0.0), aspect: 1.0, focal: None };
    let doc = square(Paint::None, paint(geom, true), 6.0);
    let img = render_view(&doc, 100, 100);
    // Along the top edge (y 10, inside the stroke's inner half), the gray follows the model and
    // two pixels the same distance from the center are the same.
    for x in [20, 35, 50, 65, 80] {
        let p = Point::new(x as f64 + 0.5, 10.5);
        let want = ((1.0 - geom.param_at(GradientKind::Diamond, p).clamp(0.0, 1.0)) * 255.0).round() as u8;
        assert!(img.over_white(x, 10)[0].abs_diff(want) <= 6, "x {x}: {:?} vs {want}", img.over_white(x, 10));
    }
    assert_eq!(img.over_white(30, 10), img.over_white(69, 10));
}

/// A stroke's miter reaches past its paint box (the path's box grown by half the weight): the
/// diamond's grays run on there, as the model gives them, instead of repeating the box's edge.
#[test]
fn a_diamond_stroke_follows_the_squares_past_its_paint_box() {
    let c = Point::new(50.0, 50.0);
    let geom = GradientGeom { start: c, end: c + Vec2::new(45.0, 0.0), aspect: 1.0, focal: None };
    let mut ap = Appearance::basic(Paint::None, paint(geom, false), 8.0);
    for it in &mut ap.items {
        if let vectorcraft_doc::AppearanceItem::Stroke(s) = it {
            s.miter_limit = 10.0;
        }
    }
    let mut b = DocBuilder::new(100.0, 100.0);
    let peak = vectorcraft_geom::SubPath::polyline(&[Point::new(20.0, 80.0), Point::new(50.0, 20.0), Point::new(80.0, 80.0)], false);
    b.path(vectorcraft_geom::PathData::single(peak), ap, |_| {});
    let img = render_view(&b.build(), 100, 100);
    // The miter's tip runs up to y 11.06; the paint box starts at y 16. Rows 13 and 14 are
    // covered across pixel column 50.
    for y in [13, 14] {
        let p = Point::new(50.5, y as f64 + 0.5);
        let want = ((1.0 - geom.param_at(GradientKind::Diamond, p).clamp(0.0, 1.0)) * 255.0).round() as u8;
        assert!(img.over_white(50, y)[0].abs_diff(want) <= 6, "y {y}: {:?} vs {want}", img.over_white(50, y));
    }
}

/// Without an extent (no vector, or a flat square) every point is past the square: the last stop.
#[test]
fn a_diamond_without_an_extent_draws_its_last_stop() {
    let c = Point::new(50.0, 50.0);
    for geom in [
        GradientGeom { start: c, end: c, aspect: 1.0, focal: None },
        GradientGeom { start: c, end: c + Vec2::new(30.0, 0.0), aspect: 0.0, focal: None },
        GradientGeom { start: c, end: c + Vec2::new(f64::INFINITY, 0.0), aspect: 1.0, focal: None },
    ] {
        let img = render_view(&square(paint(geom, false), Paint::None, 0.0), 100, 100);
        for (x, y) in [(50, 50), (20, 80), (85, 15)] {
            assert!(img.over_white(x, y)[0] < 6, "{geom:?} ({x},{y}): {:?}", img.over_white(x, y));
        }
    }
}
