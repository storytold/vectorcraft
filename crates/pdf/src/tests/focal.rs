//! Radial gradients' focal points as two-point radial shadings, out and back in.

use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, Document, Node};
use vectorcraft_geom::{Point, Rect, shapes};
use vectorcraft_testkit::raster::{assert_similar, render_artboard};

use crate::*;

/// A page-sized square filled with a radial on `start`→`end` at `aspect`, focal point `focal`.
fn doc(start: Point, end: Point, aspect: f64, focal: Point) -> Document {
    let mut geom = GradientGeom { start, end, aspect, focal: None };
    geom.set_focal(Some(focal));
    let mut gp = GradientPaint::new(Gradient { kind: GradientKind::Radial, ..Gradient::default() });
    gp.geom = Some(geom);
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let ap = Appearance::basic(Paint::Gradient(Box::new(gp)), Paint::None, 0.0);
    d.insert(Some(l), 0, Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 200.0)), ap)).unwrap();
    d
}

fn geom(d: &Document) -> GradientGeom {
    let mut out = None;
    d.walk(|n| {
        if let Paint::Gradient(g) = n.appearance.fill_paint() {
            out = g.geom;
        }
    });
    out.expect("a gradient fill")
}

#[test]
fn a_focal_point_round_trips_as_a_two_point_radial_shading() {
    for (end, aspect) in [(Point::new(180.0, 100.0), 1.0), (Point::new(100.0, 40.0), 0.5)] {
        let d = doc(Point::new(100.0, 100.0), end, aspect, Point::new(80.0, 110.0));
        let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
        let g = geom(&back);
        let f = g.focal.expect("focal kept");
        assert!(f.distance(Point::new(80.0, 110.0)) < 0.05 && g.start.distance(Point::new(100.0, 100.0)) < 0.05, "{g:?}");
        // The same ellipse (its axes may come back swapped, with the aspect inverted).
        let want = geom(&d);
        for p in [Point::new(30.0, 100.0), Point::new(100.0, 170.0), Point::new(140.0, 60.0), Point::new(90.0, 125.0)] {
            let (a, b) = (g.param_at(GradientKind::Radial, p), want.param_at(GradientKind::Radial, p));
            assert!((a - b).abs() < 1e-3, "{p:?}: {a} vs {b}: {g:?}");
        }
        // And it looks the same.
        assert_similar(&render_artboard(&back), &render_artboard(&d), 3.0, 0.01);
    }
}

#[test]
fn a_centred_radial_imports_without_a_focal_point() {
    let d = doc(Point::new(100.0, 100.0), Point::new(180.0, 100.0), 1.0, Point::new(100.0, 100.0));
    assert_eq!(geom(&import(&export(&d, &PdfOptions::default()).unwrap()).unwrap()).focal, None);
}
