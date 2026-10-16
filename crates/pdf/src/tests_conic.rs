//! Conic gradients as function-based shadings: PDF has no conic gradient, but a type 1 shading
//! whose PostScript function turns the angle around the centre into the ramp draws one exactly.

use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, Document, Node};
use vectorcraft_geom::{Point, Rect, shapes};

use crate::*;

/// A page-sized square filled with a white-to-black conic on `geom`.
fn doc(geom: GradientGeom) -> Document {
    let mut gp = GradientPaint::new(Gradient { kind: GradientKind::Conic, ..Gradient::default() });
    gp.geom = Some(geom);
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let ap = Appearance::basic(Paint::Gradient(Box::new(gp)), Paint::None, 0.0);
    d.insert(Some(l), 0, Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 200.0)), ap)).unwrap();
    d
}

#[test]
fn a_conic_gradient_is_written_as_a_function_based_shading() {
    let geom = GradientGeom { start: Point::new(100.0, 100.0), end: Point::new(180.0, 60.0), aspect: 0.6, focal: None };
    let report = export_with_report(&doc(geom), &PdfOptions::uncompressed()).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let text = String::from_utf8_lossy(&report.bytes);
    assert!(text.contains("/ShadingType 1"), "a function-based shading");
    assert!(text.contains("/FunctionType 4") && text.contains("atan"), "its function turns the angle into the ramp");
    assert!(!text.contains("/ShadingType 2") && !text.contains("/ShadingType 3"), "no axial or radial stand-in");
}

#[test]
fn a_conic_gradient_without_extent_still_exports() {
    let c = Point::new(100.0, 100.0);
    let report = export_with_report(&doc(GradientGeom { start: c, end: c, aspect: 1.0, focal: None }), &PdfOptions::uncompressed()).unwrap();
    assert!(String::from_utf8_lossy(&report.bytes).contains("/ShadingType 1"));
}
