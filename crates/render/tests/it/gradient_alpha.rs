//! Gradient stops with opacity: colour and opacity blend each on its own (not premultiplied), as
//! the model samples them ([`Gradient::sample`]) and as the SVG and PDF exports draw them. The colour
//! of a transparent stop counts: red fading into transparent black darkens as it fades.

use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, Document};
use vectorcraft_geom::{Point, Rect, shapes};
use vectorcraft_testkit::fixtures::DocBuilder;
use vectorcraft_testkit::raster::{Image, render_view};

/// A 100 pt square filled with `gradient` on `geom`.
fn filled(gradient: &Gradient, geom: GradientGeom) -> Document {
    let mut gp = GradientPaint::new(gradient.clone());
    gp.geom = Some(geom);
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(Paint::Gradient(Box::new(gp)), Paint::None, 0.0), |_| {});
    b.build()
}

fn stop(offset: f32, color: Color, opacity: f32, midpoint: f32) -> GradientStop {
    GradientStop { opacity, midpoint, ..GradientStop::new(offset, color) }
}

/// Every 7th pixel is the model's colour at its opacity over white (pixel centres).
fn assert_follows_model(img: &Image, gradient: &Gradient, geom: &GradientGeom) {
    for y in (3..100).step_by(7) {
        for x in (3..100).step_by(7) {
            let t = geom.param_at(gradient.kind, Point::new(x as f64 + 0.5, y as f64 + 0.5)).clamp(0.0, 1.0);
            let (c, o) = gradient.sample(t as f32);
            let want = c.to_rgb().map(|v| ((v * o + 1.0 - o) * 255.0).round() as u8);
            let got = img.over_white(x, y);
            let off = (0..3).map(|k| got[k].abs_diff(want[k])).max().unwrap_or(0);
            assert!(off <= 4, "{:?} ({x},{y}) t={t:.3}: got {got:?}, want {want:?}", gradient.kind);
        }
    }
}

#[test]
fn transparent_stops_blend_colour_and_opacity_apart() {
    let red = Color::rgb(1.0, 0.0, 0.0);
    let fades = [
        // Transparent white into black: mid-grey at half opacity halfway, light over white.
        vec![stop(0.0, Color::WHITE, 0.0, 0.5), stop(1.0, Color::BLACK, 1.0, 0.5)],
        // Red into transparent black: darker red as it fades.
        vec![stop(0.0, red, 1.0, 0.5), stop(1.0, Color::BLACK, 0.0, 0.5)],
        // A midpoint off centre (drawn as an extra stop) follows the same rule.
        vec![stop(0.0, red, 1.0, 0.3), stop(1.0, Color::BLACK, 0.0, 0.5)],
    ];
    for stops in fades {
        for (kind, geom) in [
            (GradientKind::Linear, GradientGeom { start: Point::new(0.0, 50.0), end: Point::new(100.0, 50.0), aspect: 1.0, focal: None }),
            (GradientKind::Radial, GradientGeom { start: Point::new(50.0, 50.0), end: Point::new(100.0, 50.0), aspect: 1.0, focal: None }),
        ] {
            let gradient = Gradient::new(kind, stops.clone());
            assert_follows_model(&render_view(&filled(&gradient, geom), 100, 100), &gradient, &geom);
        }
    }
}
