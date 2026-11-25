//! Diamond gradients, which SVG has no element for: fills and strokes of opaque stops are written
//! as four linear gradients, one for each quadrant of the diamond's frame, clipped to the shape
//! (or the stroke's outline), others as an image, with a warning; read back, the SVG looks as the
//! canvas paints the gradient.

use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientMethod, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId};
use vectorcraft_geom::{Point, Rect, shapes};
use vectorcraft_svg::{ExportOptions, export_full, import};
use vectorcraft_testkit::raster::{assert_similar, render_artboard};

/// `gradient` (red to blue when `None`), from (100, 80) out to a square turned and stretched
/// across its vector.
fn diamond(gradient: Option<Gradient>) -> Paint {
    let ramp = Gradient::new(
        GradientKind::Diamond,
        vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))],
    );
    let mut g = GradientPaint::new(gradient.unwrap_or(ramp));
    g.geom = Some(GradientGeom { start: Point::new(100.0, 80.0), end: Point::new(150.0, 50.0), aspect: 0.7, focal: None });
    g.swatch = Some("Gem".into());
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
fn a_diamond_fill_is_four_linear_gradients_clipped_to_the_shape() {
    let shape = shapes::ellipse(Rect::new(20.0, 10.0, 180.0, 150.0));
    let d = doc_with(Node::path(NodeId(0), shape, Appearance::basic(diamond(None), Paint::None, 0.0)));
    let o = export_full(&d, &ExportOptions::default(), None);
    assert_eq!(o.svg.matches("<linearGradient").count(), 4, "{}", o.svg);
    assert!(!o.svg.contains("<radialGradient") && !o.svg.contains("<image "), "no stand-in");
    assert!(o.svg.contains("<clipPath"), "clipped to the shape");
    // Not named after the swatch: none of the four is its gradient.
    assert!(!o.svg.contains("id=\"Gem"), "{}", o.svg);
    assert!(o.warnings.iter().any(|w| w.contains("diamond gradients are written as four linear gradients")), "{:?}", o.warnings);
    // Read back, it looks as the canvas paints the gradient, with no hairline between quadrants.
    assert_similar(&render_artboard(&d), &render_artboard(&import(&o.svg).unwrap()), 4.0, 0.01);
}

#[test]
fn a_diamond_stroke_is_four_linear_gradients_clipped_to_the_stroke_outline() {
    let shape = shapes::rectangle(Rect::new(30.0, 30.0, 170.0, 130.0));
    let d = doc_with(Node::path(NodeId(0), shape, Appearance::basic(Paint::None, diamond(None), 12.0)));
    let o = export_full(&d, &ExportOptions::default(), None);
    assert!(o.svg.contains("<clipPath") && o.svg.matches("<linearGradient").count() == 4, "{}", o.svg);
    assert_similar(&render_artboard(&d), &render_artboard(&import(&o.svg).unwrap()), 8.0, 0.01);
}

/// A stroke's miter reaches past its paint box (the path's box grown by half the weight): the
/// quadrants of a diamond, the image of a Perceptual one and the wedges of a conic gradient cover
/// it, and read back it is painted as the canvas paints it.
#[test]
fn a_stroke_past_its_paint_box_is_painted_to_its_tip() {
    // The center lies below the art, so the miter's tip is farther from it than any corner of the
    // paint box.
    let c = Point::new(100.0, 200.0);
    let geom = GradientGeom { start: c, end: c + vectorcraft_geom::Vec2::new(200.0, 0.0), aspect: 1.0, focal: None };
    let ramp = |kind, method| {
        let stops = vec![GradientStop::new(0.0, Color::WHITE), GradientStop::new(1.0, Color::BLACK)];
        let mut g = GradientPaint::new(Gradient { method, ..Gradient::new(kind, stops) });
        g.geom = Some(geom);
        Paint::Gradient(Box::new(g))
    };
    let mut wrong = vec![];
    for (case, paint) in [
        ("diamond", ramp(GradientKind::Diamond, GradientMethod::Classic)),
        ("perceptual diamond", ramp(GradientKind::Diamond, GradientMethod::Perceptual)),
        ("conic", ramp(GradientKind::Conic, GradientMethod::Classic)),
    ] {
        let mut ap = Appearance::basic(Paint::None, paint, 8.0);
        for it in &mut ap.items {
            if let vectorcraft_doc::AppearanceItem::Stroke(s) = it {
                s.miter_limit = 10.0;
            }
        }
        // The miter's tip runs up to y 23.5; the paint box starts at y 36. Rows 30 and 31 are
        // covered across pixel column 100.
        let peak = vectorcraft_geom::SubPath::polyline(&[Point::new(85.0, 100.0), Point::new(100.0, 40.0), Point::new(115.0, 100.0)], false);
        let d = doc_with(Node::path(NodeId(0), vectorcraft_geom::PathData::single(peak), ap));
        let o = export_full(&d, &ExportOptions::default(), None);
        let (want, got) = (render_artboard(&d), render_artboard(&import(&o.svg).unwrap()));
        for y in [30, 31] {
            let (a, b) = (want.over_white(100, y), got.over_white(100, y));
            assert!(a[0] < 200, "{case} at y {y}: the canvas paints the tip: {a:?}");
            if !a.iter().zip(b).all(|(p, q)| p.abs_diff(q) <= 12) {
                wrong.push(format!("{case} at y {y}: {a:?} vs {b:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// Translucent stops (the overlapping quadrants would add up their opacity), dither, the
/// Perceptual method and a diamond without an extent give an image clipped to the shape.
#[test]
fn a_diamond_that_quadrants_cannot_draw_is_an_image() {
    let mut clear = GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0));
    clear.opacity = 0.0;
    let ramp = || Gradient::new(GradientKind::Diamond, vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::BLACK)]);
    let translucent = Gradient::new(GradientKind::Diamond, vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), clear]);
    let dithered = Gradient { dither: true, ..ramp() };
    let perceptual = Gradient { method: GradientMethod::Perceptual, ..ramp() };
    let flat = {
        let Paint::Gradient(mut g) = diamond(None) else { panic!("a gradient") };
        g.geom = g.geom.map(|geom| GradientGeom { aspect: 0.0, ..geom });
        Paint::Gradient(g)
    };
    for (case, paint) in [
        ("translucent", diamond(Some(translucent))),
        ("dithered", diamond(Some(dithered))),
        ("perceptual", diamond(Some(perceptual))),
        ("flat", flat),
    ] {
        let shape = shapes::rectangle(Rect::new(20.0, 20.0, 180.0, 140.0));
        let d = doc_with(Node::path(NodeId(0), shape, Appearance::basic(paint, Paint::None, 0.0)));
        let o = export_full(&d, &ExportOptions::default(), None);
        assert_eq!(o.svg.matches("<image ").count(), 1, "{case}");
        assert!(o.warnings.iter().any(|w| w.contains("diamond") && w.contains("images")), "{case}: {:?}", o.warnings);
        assert_similar(&render_artboard(&d), &render_artboard(&import(&o.svg).unwrap()), 12.0, 0.02);
    }
}

/// A diamond on characters, which SVG paints with a gradient element, is the radial gradient
/// through its corners, with a warning, centered even when the diamond carries a focal point (a
/// plug-in's or a file's, which a diamond doesn't draw).
#[test]
fn a_diamond_on_characters_is_written_radial() {
    use vectorcraft_doc::{CharStyle, NodeKind, TextObject};
    let Paint::Gradient(mut g) = diamond(None) else { panic!("a gradient") };
    g.geom = g.geom.map(|geom| GradientGeom { focal: Some(Point::new(110.0, 75.0)), ..geom });
    let st = CharStyle { fill: Paint::Gradient(g), size: 48.0, ..CharStyle::default() };
    let d = doc_with(Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 80.0), "Gem", st)))));
    let o = export_full(&d, &ExportOptions::default(), None);
    assert!(o.svg.contains("<radialGradient"), "{}", o.svg);
    assert!(!o.svg.contains(" fx=\""), "{}", o.svg);
    assert!(o.warnings.iter().any(|w| w.contains("diamond gradients on characters")), "{:?}", o.warnings);
}
