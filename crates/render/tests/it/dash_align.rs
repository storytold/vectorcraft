//! Pixel tests for dashes fitted to corners and for width profiles with dashes and joins, drawn
//! on the canvas from the shared stroke geometry (`vectorcraft_effects::stroke`).

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Dash, Document, LineJoin, StrokeLayer, WidthProfile};
use vectorcraft_geom::{PathData, Point, Rect, SubPath, shapes};
use vectorcraft_testkit::fixtures::DocBuilder;
use vectorcraft_testkit::raster::{Image, render_region};

/// Pixels per point in these tests.
const S: f64 = 4.0;

fn doc(path: PathData, width: f64, f: impl FnOnce(&mut StrokeLayer)) -> Document {
    let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), width);
    f(&mut st);
    let mut b = DocBuilder::new(160.0, 120.0);
    b.path(path, Appearance { items: vec![AppearanceItem::Stroke(st)], ..Default::default() }, |_| {});
    b.build()
}

fn render(d: &Document) -> Image {
    render_region(d, Rect::new(0.0, 0.0, 160.0, 120.0), S)
}

/// Is the pixel at document point (x, y) dark?
fn dark(img: &Image, x: f64, y: f64) -> bool {
    img.over_white((x * S) as u32, (y * S) as u32)[0] < 128
}

fn rect_doc(align_corners: bool) -> Document {
    doc(shapes::rectangle(Rect::new(20.0, 20.0, 120.0, 70.0)), 2.0, |s| s.dash = Some(Dash { pattern: vec![12.0, 6.0], offset: 0.0, align_corners }))
}

#[test]
fn fitted_dashes_wrap_every_corner_of_a_rectangle() {
    let img = render(&rect_doc(true));
    // Half a dash (6 · 100/108) runs each way from every corner, then a gap of the same length.
    let half = 6.0 * 100.0 / 108.0;
    for (c, dirs) in [
        ((20.0, 20.0), [(1.0, 0.0), (0.0, 1.0)]),
        ((120.0, 20.0), [(-1.0, 0.0), (0.0, 1.0)]),
        ((120.0, 70.0), [(-1.0, 0.0), (0.0, -1.0)]),
        ((20.0, 70.0), [(1.0, 0.0), (0.0, -1.0)]),
    ] {
        for (dx, dy) in dirs {
            let at = |d: f64| (c.0 + dx * d, c.1 + dy * d);
            let (x, y) = at(half / 2.0);
            assert!(dark(&img, x, y), "dash beside corner {c:?}");
            let (x, y) = at(half * 1.5);
            assert!(!dark(&img, x, y), "gap after corner {c:?} along ({dx}, {dy})");
        }
    }
    // Exact dashes keep their lengths instead (the exact golden covers how they look).
    assert_ne!(render(&rect_doc(false)).rgba, img.rgba);
}

#[test]
fn dashes_of_a_lens_stroke_widen_towards_the_middle() {
    let img = render(&doc(shapes::line(Point::new(30.0, 60.0), Point::new(130.0, 60.0)), 10.0, |s| {
        s.profile = Some(WidthProfile::lens());
        s.dash = Some(Dash { pattern: vec![10.0, 10.0], offset: 0.0, align_corners: false });
    }));
    // The dash over 40..50 of the 100 pt line is nearly full width, the one over 0..10 thin.
    assert!(dark(&img, 75.0, 60.0 - 3.8) && dark(&img, 75.0, 60.0 + 3.8));
    assert!(dark(&img, 35.0, 60.0) && !dark(&img, 35.0, 60.0 - 1.5));
    assert!(!dark(&img, 45.0, 60.0), "gap");
}

#[test]
fn a_round_join_on_a_profile_stroke_has_no_spike() {
    let l = SubPath::polyline(&[Point::new(20.0, 40.0), Point::new(100.0, 40.0), Point::new(100.0, 110.0)], false);
    let img = render(&doc(PathData::single(l), 20.0, |s| {
        s.profile = Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)] });
        s.join = LineJoin::Round;
    }));
    assert!(dark(&img, 106.0, 34.0), "the round join covers the corner");
    assert!(!dark(&img, 109.0, 31.0), "but not the miter's point");
}
