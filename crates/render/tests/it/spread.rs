//! Gradients that repeat or reflect: the canvas draws each kind's stops `repeats` times along its
//! vector and on past its ends, as the model's parameter gives them, on fills and strokes.

use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientSpread, Paint};
use vectorcraft_doc::{Appearance, Document};
use vectorcraft_geom::{Point, Rect, Vec2, shapes};
use vectorcraft_testkit::fixtures::DocBuilder;
use vectorcraft_testkit::raster::{Image, render_view};

/// A white-to-black gradient of `kind` on `geom` that spreads by `spread`, twice.
fn gradient(kind: GradientKind, spread: GradientSpread, geom: GradientGeom, dither: bool) -> GradientPaint {
    let mut gp = GradientPaint::new(Gradient { kind, spread, repeats: 2, dither, ..Gradient::default() });
    gp.geom = Some(geom);
    gp
}

/// A 100 pt square filled with `fill` and stroked `width` wide with `stroke`.
fn square(fill: Paint, stroke: Paint, width: f64) -> Document {
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 90.0)), Appearance::basic(fill, stroke, width), |_| {});
    b.build()
}

/// Every 5th pixel inside `inside`, away from where Repeat jumps from black back to white and
/// from a conic's center, has the gray the model gives it (pixel centers), within what a pixel's
/// step on the ramp and dither's noise allow; at least `least` pixels are checked.
fn assert_follows_model(img: &Image, g: &GradientPaint, inside: Rect, least: usize, what: &str) {
    let geom = g.geom.unwrap();
    let kind = g.gradient.kind;
    let mut checked = 0;
    for y in (2..100).step_by(5) {
        for x in (2..100).step_by(5) {
            let p = Point::new(x as f64 + 0.5, y as f64 + 0.5);
            if !inside.contains(p) || (kind == GradientKind::Conic && p.distance(geom.start) < 12.0) {
                continue;
            }
            let at = |q: Point| g.gradient.spread_param(geom.param_at(kind, q));
            let t = at(p);
            if g.gradient.spread == GradientSpread::Repeat && !(0.06..0.94).contains(&t) {
                continue;
            }
            // How far the gray moves across a pixel there: pixels drawn from a grid of cells (a
            // diamond's, a dithered fill's) are sampled up to half a pixel away.
            let step = [Vec2::new(0.5, 0.0), Vec2::new(0.0, 0.5), Vec2::new(-0.5, 0.0), Vec2::new(0.0, -0.5)]
                .map(|d| 2.0 * (at(p + d) - t).abs())
                .into_iter()
                .fold(0.0, f64::max);
            let tolerance = 6 + (step * 255.0).ceil() as u8;
            let want = ((1.0 - t.clamp(0.0, 1.0)) * 255.0).round() as u8;
            let got = img.over_white(x, y)[0];
            assert!(got.abs_diff(want) <= tolerance, "{what} ({x},{y}) t={t:.3}: got {got}, want {want} (within {tolerance})");
            checked += 1;
        }
    }
    assert!(checked >= least, "{what}: only {checked} pixels checked");
}

fn geoms() -> [(GradientKind, GradientGeom); 5] {
    let c = Point::new(50.0, 50.0);
    let level = GradientGeom { start: c, end: c + Vec2::new(36.0, 12.0), aspect: 0.7, focal: None };
    [
        (GradientKind::Linear, GradientGeom { start: Point::new(20.0, 30.0), end: Point::new(70.0, 45.0), aspect: 1.0, focal: None }),
        (GradientKind::Radial, level),
        (GradientKind::Radial, GradientGeom { focal: Some(c + Vec2::new(-10.0, 6.0)), ..level }),
        (GradientKind::Conic, level),
        (GradientKind::Diamond, level),
    ]
}

/// Linear, radial (centered and off-center), conic and diamond fills repeat and reflect as the
/// model says, inside the vector and past its ends; so does a dithered fill, drawn as pixels.
#[test]
fn fills_repeat_and_reflect_for_every_kind() {
    for (kind, geom) in geoms() {
        for spread in [GradientSpread::Repeat, GradientSpread::Reflect] {
            for dither in [false, true] {
                let g = gradient(kind, spread, geom, dither);
                let img = render_view(&square(Paint::Gradient(Box::new(g.clone())), Paint::None, 0.0), 100, 100);
                assert_follows_model(&img, &g, Rect::new(12.0, 12.0, 88.0, 88.0), 100, &format!("{kind:?} {spread:?} dither {dither}"));
            }
        }
    }
}

/// A stroke's gradient repeats out to its miters too.
#[test]
fn a_stroke_repeats_out_past_the_paint_box() {
    for (kind, geom) in geoms() {
        let g = gradient(kind, GradientSpread::Reflect, geom, false);
        let img = render_view(&square(Paint::None, Paint::Gradient(Box::new(g.clone())), 12.0), 100, 100);
        assert_follows_model(&img, &g, Rect::new(5.0, 5.0, 15.0, 95.0), 20, &format!("{kind:?} stroke"));
    }
}
