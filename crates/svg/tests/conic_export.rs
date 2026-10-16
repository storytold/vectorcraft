//! Conic gradients, which SVG has no element for: fills and strokes are written as solid wedges
//! around the centre clipped to the shape (or the stroke's outline), with a warning, and the SVG
//! reads back looking as the canvas paints the gradient, its seam still sharp.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId};
use vectorcraft_geom::{Point, Rect, shapes};
use vectorcraft_svg::{ExportOptions, export_full, import};
use vectorcraft_testkit::raster::{assert_similar, render_artboard};

/// White to black (or `stops`), sweeping clockwise from the right around (100, 80).
fn conic(stops: Option<Vec<GradientStop>>) -> Paint {
    let mut g = GradientPaint::new(Gradient { kind: GradientKind::Conic, stops: stops.unwrap_or_else(|| Gradient::default().stops) });
    g.geom = Some(GradientGeom { start: Point::new(100.0, 80.0), end: Point::new(160.0, 80.0), aspect: 1.0, focal: None });
    Paint::Gradient(Box::new(g))
}

/// A 200 × 160 document holding `n`.
fn doc_with(mut n: Node) -> Document {
    let mut d = Document::new(200.0, 160.0);
    n.id = d.alloc_id();
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

#[test]
fn a_conic_fill_is_solid_wedges_clipped_to_the_shape() {
    let shape = shapes::ellipse(Rect::new(20.0, 10.0, 180.0, 150.0));
    let d = doc_with(Node::path(NodeId(0), shape, Appearance::basic(conic(None), Paint::None, 0.0)));
    let o = export_full(&d, &ExportOptions::default(), None);
    let svg = &o.svg;
    assert!(!svg.contains("Gradient") && !svg.contains("<image "), "no gradient element or image stands in");
    assert!(svg.contains("<clipPath") && svg.contains("clip-path=\"url(#clip-path-1)\""), "clipped to the shape");
    assert_eq!(svg.matches(" fill=\"#").count(), 360, "a wedge per degree");
    assert!(o.warnings.iter().any(|w| w.contains("conic")), "{:?}", o.warnings);
    // Read back, it looks as the canvas paints the gradient: the seam (to the right of the
    // centre) included.
    let (canvas, back) = (render_artboard(&d), render_artboard(&import(svg).unwrap()));
    assert_similar(&canvas, &back, 6.0, 0.01);
    for (x, y) in [(150, 82), (150, 77), (100, 130), (50, 80), (100, 30)] {
        let (a, b) = (canvas.over_white(x, y), back.over_white(x, y));
        assert!(a[0].abs_diff(b[0]) <= 6, "({x},{y}): {a:?} vs {b:?}");
    }
}

#[test]
fn a_conic_stroke_is_wedges_clipped_to_the_stroke_outline() {
    let shape = shapes::rectangle(Rect::new(30.0, 30.0, 170.0, 130.0));
    let d = doc_with(Node::path(NodeId(0), shape, Appearance::basic(Paint::None, conic(None), 12.0)));
    let o = export_full(&d, &ExportOptions::default(), None);
    assert!(o.svg.contains("<clipPath") && !o.svg.contains("Gradient"), "{}", o.svg);
    assert_similar(&render_artboard(&d), &render_artboard(&import(&o.svg).unwrap()), 8.0, 0.01);
}

#[test]
fn translucent_stops_give_an_image_clipped_to_the_shape() {
    let mut clear = GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0));
    clear.opacity = 0.0;
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), clear];
    let d = doc_with(Node::path(
        NodeId(0),
        shapes::rectangle(Rect::new(20.0, 20.0, 180.0, 140.0)),
        Appearance::basic(conic(Some(stops)), Paint::None, 0.0),
    ));
    let o = export_full(&d, &ExportOptions::default(), None);
    // Wedges drawn over each other would add up their opacity: an image of the sweep instead.
    assert_eq!(o.svg.matches("<image ").count(), 1, "{}", o.svg);
    assert!(o.warnings.iter().any(|w| w.contains("conic") && w.contains("images")), "{:?}", o.warnings);
    assert_similar(&render_artboard(&d), &render_artboard(&import(&o.svg).unwrap()), 12.0, 0.02);
}
