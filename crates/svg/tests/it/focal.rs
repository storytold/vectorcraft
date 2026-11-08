//! Radial gradients' focal points as `fx`/`fy`, out and back in.

use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, Document, Node};
use vectorcraft_geom::{Point, Rect, shapes};
use vectorcraft_svg::{ExportOptions, export, import};

/// One square filled with a radial on `start`→`end` at `aspect`, its focal point at `focal`.
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

fn close(a: Point, b: Point) -> bool {
    a.distance(b) < 0.01
}

#[test]
fn a_circle_writes_its_focal_point_as_fx_fy_and_reads_it_back() {
    let d = doc(Point::new(100.0, 100.0), Point::new(180.0, 100.0), 1.0, Point::new(70.0, 110.0));
    let svg = export(&d, &ExportOptions::default());
    assert!(svg.contains("cx=\"100\" cy=\"100\" r=\"80\" fx=\"70\" fy=\"110\""), "{svg}");
    let g = geom(&import(&svg).unwrap());
    assert!(close(g.start, Point::new(100.0, 100.0)) && close(g.focal.expect("focal kept"), Point::new(70.0, 110.0)), "{g:?}");
    // A centred radial writes none.
    let centred = doc(Point::new(100.0, 100.0), Point::new(180.0, 100.0), 1.0, Point::new(100.0, 100.0));
    let svg = export(&centred, &ExportOptions::default());
    assert!(!svg.contains("fx="), "{svg}");
    assert_eq!(geom(&import(&svg).unwrap()).focal, None);
}

#[test]
fn an_ellipse_writes_its_focal_point_in_the_gradient_space() {
    let d = doc(Point::new(100.0, 100.0), Point::new(100.0, 40.0), 0.5, Point::new(110.0, 120.0));
    let svg = export(&d, &ExportOptions::default());
    assert!(svg.contains("fx=") && svg.contains("gradientTransform"), "{svg}");
    let g = geom(&import(&svg).unwrap());
    assert!(close(g.focal.expect("focal kept"), Point::new(110.0, 120.0)), "{g:?}");
    assert!((g.aspect - 0.5).abs() < 1e-3, "{g:?}");
}

#[test]
fn imported_focal_points_outside_the_circle_are_pulled_in() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">
        <radialGradient id="g" cx="100" cy="100" r="50" fx="0" fy="100" gradientUnits="userSpaceOnUse">
            <stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#000"/></radialGradient>
        <rect width="200" height="200" fill="url(#g)"/></svg>"##;
    let g = geom(&import(svg).unwrap());
    let f = g.focal.expect("focal kept");
    assert!(f.y == 100.0 && f.x > 50.0 && f.x < 51.0, "{g:?}");
}
