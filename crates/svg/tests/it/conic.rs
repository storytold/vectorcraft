//! Conic gradients, which SVG has no element for: fills and strokes of opaque stops are written
//! as solid wedges around the center clipped to the shape (or the stroke's outline), others as an
//! image, with a warning; read back, the SVG looks as the canvas paints the gradient, its seam
//! still sharp.

use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientMethod, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId};
use vectorcraft_geom::{Point, Rect, shapes};
use vectorcraft_svg::{ExportOptions, export_full, import};
use vectorcraft_testkit::raster::{assert_similar, render_artboard};

/// `gradient` (white to black when `None`), sweeping clockwise from the right around (100, 80).
fn conic(gradient: Option<Gradient>) -> Paint {
    let mut g = GradientPaint::new(gradient.unwrap_or(Gradient { kind: GradientKind::Conic, ..Gradient::default() }));
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
    assert!(!o.svg.contains("Gradient") && !o.svg.contains("<image "), "no gradient element or image stands in");
    assert!(o.svg.contains("<clipPath"), "clipped to the shape");
    assert_eq!(o.svg.matches(" fill=\"#").count(), 360, "a wedge a degree");
    assert!(o.warnings.iter().any(|w| w.contains("conic gradients are written as solid wedges")), "{:?}", o.warnings);
    // Read back, it looks as the canvas paints the gradient, the seam (right of the center) too.
    let (canvas, back) = (render_artboard(&d), render_artboard(&import(&o.svg).unwrap()));
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

/// Translucent stops (wedges drawn over each other would add up their opacity), dither and the
/// Perceptual method give an image of the sweep clipped to the shape.
#[test]
fn a_conic_that_wedges_cannot_draw_is_an_image() {
    let mut clear = GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0));
    clear.opacity = 0.0;
    let translucent = Gradient::new(GradientKind::Conic, vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), clear]);
    let dithered = Gradient { dither: true, ..Gradient { kind: GradientKind::Conic, ..Gradient::default() } };
    let perceptual = Gradient { method: GradientMethod::Perceptual, ..Gradient { kind: GradientKind::Conic, ..Gradient::default() } };
    for g in [translucent, dithered, perceptual] {
        let shape = shapes::rectangle(Rect::new(20.0, 20.0, 180.0, 140.0));
        let d = doc_with(Node::path(NodeId(0), shape, Appearance::basic(conic(Some(g.clone())), Paint::None, 0.0)));
        let o = export_full(&d, &ExportOptions::default(), None);
        assert_eq!(o.svg.matches("<image ").count(), 1, "{g:?}");
        assert!(o.warnings.iter().any(|w| w.contains("conic") && w.contains("images")), "{:?}", o.warnings);
        assert_similar(&render_artboard(&d), &render_artboard(&import(&o.svg).unwrap()), 12.0, 0.02);
    }
}

/// A conic on characters, which SVG paints with a gradient element, is a linear gradient, with a
/// warning.
#[test]
fn a_conic_on_characters_is_written_linear() {
    use vectorcraft_doc::{CharStyle, NodeKind, TextObject};
    let st = CharStyle { fill: conic(None), size: 48.0, ..CharStyle::default() };
    let d = doc_with(Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 80.0), "Sweep", st)))));
    let o = export_full(&d, &ExportOptions::default(), None);
    assert!(o.svg.contains("<linearGradient"), "{}", o.svg);
    assert!(o.warnings.iter().any(|w| w.contains("conic gradients on characters")), "{:?}", o.warnings);
}
