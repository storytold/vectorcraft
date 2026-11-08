//! Paints SVG can't express: freeform gradients fill and stroke objects as an `<image>` of their
//! colour field clipped to the shape (or the stroke's outline), and the SVG reads back looking as
//! the canvas paints it.

use vectorcraft_color::freeform::spread_scale;
use vectorcraft_color::{Color, Freeform, FreeformPoint, Gradient, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{Point, Rect, shapes};
use vectorcraft_svg::{ExportOptions, ImageMode, export_full, import};
use vectorcraft_testkit::format::base64_decode;
use vectorcraft_testkit::raster::{assert_similar, render_artboard};

/// Red at the top left, blue at the bottom right, a half-transparent green at the top right.
fn freeform() -> Paint {
    let p = |x, y, c, opacity| FreeformPoint { opacity, spread: 0.1, ..FreeformPoint::new(Point::new(x, y), c) };
    let f = Freeform {
        points: vec![
            p(30.0, 30.0, Color::rgb(1.0, 0.0, 0.0), 1.0),
            p(170.0, 130.0, Color::rgb(0.0, 0.0, 1.0), 1.0),
            p(170.0, 30.0, Color::rgb(0.0, 1.0, 0.0), 0.5),
        ],
        ..Default::default()
    };
    let mut g = GradientPaint::new(Gradient { kind: GradientKind::Freeform, ..Default::default() });
    g.freeform = Some(f);
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

/// The data of the one embedded PNG.
fn png_of(svg: &str) -> image::RgbaImage {
    let data = svg.split("data:image/png;base64,").nth(1).expect("an embedded PNG").split('"').next().unwrap();
    image::load_from_memory(&base64_decode(data).unwrap()).unwrap().to_rgba8()
}

#[test]
fn a_freeform_fill_is_an_image_clipped_to_the_shape() {
    let shape = shapes::ellipse(Rect::new(20.0, 20.0, 180.0, 140.0));
    let d = doc_with(Node::path(NodeId(0), shape, Appearance::basic(freeform(), Paint::None, 0.0)));
    let o = export_full(&d, &ExportOptions::default(), None);
    let svg = &o.svg;
    assert!(!svg.contains("linearGradient"), "{svg}");
    assert_eq!(svg.matches("<image ").count(), 1, "{svg}");
    assert!(svg.contains("<clipPath") && svg.contains("<g clip-path=\"url(#clip-path-1)\">"), "{svg}");
    assert!(o.warnings.iter().any(|w| w.contains("freeform")), "{:?}", o.warnings);
    // The image is the colour field over the shape's box, straight alpha: the green point is
    // half transparent.
    let png = png_of(svg);
    let (w, h) = png.dimensions();
    assert!((8..=256).contains(&w) && (8..=256).contains(&h), "{w}×{h}");
    let b = Rect::new(20.0, 20.0, 180.0, 140.0);
    let Paint::Gradient(g) = freeform() else { panic!("a gradient") };
    let field = g.freeform.unwrap().field(spread_scale(b));
    for (x, y) in [(2, 2), (w / 2, h / 2), (w - 3, 2), (w - 3, h - 3)] {
        let at = Point::new(b.x0 + (x as f64 + 0.5) * b.width() / w as f64, b.y0 + (y as f64 + 0.5) * b.height() / h as f64);
        let ([r, g, bl], a) = field.sample(at);
        let want = [r, g, bl, a].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as i32);
        let got = png.get_pixel(x, y).0.map(i32::from);
        assert!(want.iter().zip(got).all(|(w, g)| (w - g).abs() <= 1), "({x}, {y}): {got:?} vs {want:?}");
    }
    // Read back, it looks as the canvas paints the gradient.
    let back = import(svg).unwrap();
    let mut images = 0;
    back.walk(|n| images += usize::from(matches!(n.kind, NodeKind::Image(_))));
    assert_eq!(images, 1, "the image comes back (in its clip group)");
    assert_similar(&render_artboard(&d), &render_artboard(&back), 12.0, 0.01);
}

#[test]
fn a_freeform_stroke_is_an_image_clipped_to_the_stroke_outline() {
    let shape = shapes::rectangle(Rect::new(40.0, 40.0, 160.0, 120.0));
    let mut ap = Appearance::basic(Paint::solid(Color::WHITE), freeform(), 16.0);
    if let Some(AppearanceItem::Stroke(s)) = ap.items.get_mut(1) {
        s.opacity = 0.8;
    }
    let d = doc_with(Node::path(NodeId(0), shape, ap));
    let svg = export_full(&d, &ExportOptions::default(), None).svg;
    assert_eq!(svg.matches("<image ").count(), 1, "{svg}");
    assert!(svg.contains("opacity=\"0.8\"") && !svg.contains("linearGradient"), "{svg}");
    let back = import(&svg).unwrap();
    assert_similar(&render_artboard(&d), &render_artboard(&back), 12.0, 0.01);
}

#[test]
fn linked_images_write_the_gradient_next_to_the_svg_and_characters_stay_linear() {
    let d = doc_with(Node::path(NodeId(0), shapes::rectangle(Rect::new(20.0, 20.0, 120.0, 120.0)), Appearance::basic(freeform(), Paint::None, 0.0)));
    let o = export_full(&d, &ExportOptions { images: ImageMode::Link, ..Default::default() }, None);
    assert_eq!(o.linked.len(), 1);
    assert!(o.linked[0].name.ends_with(".png") && o.svg.contains(&format!("xlink:href=\"{}\"", o.linked[0].name)), "{}", o.svg);
    // Characters keep a (linear) gradient paint, with a warning.
    let style = vectorcraft_doc::CharStyle { size: 30.0, fill: freeform(), ..Default::default() };
    let t = vectorcraft_doc::TextObject::point(Point::new(20.0, 60.0), "Hi", style);
    let o = export_full(&doc_with(Node::new(NodeId(0), NodeKind::Text(Box::new(t)))), &ExportOptions::default(), None);
    assert!(o.svg.contains("linearGradient") && !o.svg.contains("<image"), "{}", o.svg);
    assert!(o.warnings.iter().any(|w| w.contains("characters")), "{:?}", o.warnings);
}

/// A shallow grey ramp, dithered or not.
fn shallow(dither: bool) -> Paint {
    let stops = vec![
        vectorcraft_color::GradientStop::new(0.0, Color::rgb8(0x40, 0x40, 0x40)),
        vectorcraft_color::GradientStop::new(1.0, Color::rgb8(0x4a, 0x4a, 0x4a)),
    ];
    let mut g = Gradient::new(GradientKind::Linear, stops);
    g.dither = dither;
    Paint::Gradient(Box::new(GradientPaint::new(g)))
}

/// Illustrator's documentation describes dither as the gradient converted to pixels with subtle
/// noise, at the Document Raster Effects resolution
/// (https://helpx.adobe.com/illustrator/desktop/paint-and-fill/create-and-edit-gradients/control-dither-in-gradients.html):
/// a dithered fill is written as an image of the gradient with its noise, clipped to the shape; a
/// stroke (dither applies to fills only) and an undithered fill stay vector gradients.
#[test]
fn a_dithered_fill_is_an_image_at_the_raster_effects_resolution() {
    let shape = shapes::rectangle(Rect::new(20.0, 20.0, 180.0, 140.0));
    let mut d = doc_with(Node::path(NodeId(0), shape.clone(), Appearance::basic(shallow(true), shallow(true), 4.0)));
    d.raster_effects_ppi = 144.0;
    let o = export_full(&d, &ExportOptions::default(), None);
    let svg = &o.svg;
    assert_eq!(svg.matches("<image ").count(), 1, "the fill: {svg}");
    assert_eq!(svg.matches("<linearGradient").count(), 1, "the stroke keeps its gradient: {svg}");
    assert!(o.warnings.iter().any(|w| w.contains("drawn as pixels are written as images")), "{:?}", o.warnings);
    // Two pixels a point at 144 ppi, the levels around the ramp's, with noise along each row.
    let png = png_of(svg);
    assert_eq!(png.dimensions(), (320, 240));
    let row: Vec<u8> = (0..320).map(|x| png.get_pixel(x, 120).0[0]).collect();
    let longest = row.chunk_by(|a, b| a == b).map(<[_]>::len).max().unwrap();
    assert!(longest <= 12, "no bands: longest run {longest}");
    assert!(row.iter().all(|v| (0x3f..=0x4b).contains(v)), "{row:?}");
    // Undithered: a vector gradient, no image.
    let plain = doc_with(Node::path(NodeId(0), shape, Appearance::basic(shallow(false), Paint::None, 0.0)));
    let svg = export_full(&plain, &ExportOptions::default(), None).svg;
    assert!(!svg.contains("<image ") && svg.contains("<linearGradient"), "{svg}");
}

/// Blue to yellow, left to right, mixed `method`'s way.
fn blue_yellow(method: vectorcraft_color::GradientMethod) -> Paint {
    let stops = vec![
        vectorcraft_color::GradientStop::new(0.0, Color::rgb(0.0, 0.0, 1.0)),
        vectorcraft_color::GradientStop::new(1.0, Color::rgb(1.0, 1.0, 0.0)),
    ];
    let mut g = Gradient::new(GradientKind::Linear, stops);
    g.method = method;
    Paint::Gradient(Box::new(GradientPaint::new(g)))
}

/// Illustrator's documentation: perceptual gradients aren't supported in SVG exports, and are
/// rasterised when saved that way
/// (https://helpx.adobe.com/illustrator/desktop/paint-and-fill/create-and-edit-gradients/control-perceptual-interpolation-in-gradients.html).
/// A Perceptual fill and stroke are written as images of the gradient (at the raster effects
/// resolution, clipped to the shape and the stroke's outline) mixed as the canvas mixes them; a
/// Classic gradient stays a vector gradient.
#[test]
fn perceptual_gradients_are_images_and_classic_ones_stay_vector() {
    use vectorcraft_color::GradientMethod;
    let shape = shapes::rectangle(Rect::new(20.0, 20.0, 180.0, 140.0));
    let p = blue_yellow(GradientMethod::Perceptual);
    let mut d = doc_with(Node::path(NodeId(0), shape.clone(), Appearance::basic(p.clone(), p, 4.0)));
    d.raster_effects_ppi = 144.0;
    let o = export_full(&d, &ExportOptions::default(), None);
    let svg = &o.svg;
    assert_eq!(svg.matches("<image ").count(), 2, "the fill and the stroke: {svg}");
    assert!(!svg.contains("<linearGradient"), "{svg}");
    assert!(o.warnings.iter().any(|w| w.contains("drawn as pixels are written as images")), "{:?}", o.warnings);
    // The fill's image: two pixels a point at 144 ppi, the middle a blue-grey (OKLab), not the
    // mid grey a display RGB mix gives.
    let png = png_of(svg);
    assert_eq!(png.dimensions(), (320, 240));
    let [r, g, b, a] = png.get_pixel(160, 120).0.map(i32::from);
    assert_eq!(a, 255);
    assert!(b > r + 20 && b > g + 10, "a blue-grey: {:?}", [r, g, b]);
    let Paint::Gradient(want) = blue_yellow(GradientMethod::Perceptual) else { panic!("a gradient") };
    let [wr, wg, wb] = want.gradient.sample(160.5 / 320.0).0.to_rgb().map(|v| (v * 255.0).round() as i32);
    assert!((r - wr).abs() <= 1 && (g - wg).abs() <= 1 && (b - wb).abs() <= 1, "{:?} vs {:?}", [r, g, b], [wr, wg, wb]);
    // Read back, it looks as the canvas paints it.
    let back = import(svg).unwrap();
    assert_similar(&render_artboard(&d), &render_artboard(&back), 12.0, 0.01);
    // Classic: a vector gradient, no image.
    let c = blue_yellow(GradientMethod::Classic);
    let plain = doc_with(Node::path(NodeId(0), shape, Appearance::basic(c.clone(), c, 4.0)));
    let svg = export_full(&plain, &ExportOptions::default(), None).svg;
    assert!(!svg.contains("<image ") && svg.matches("<linearGradient").count() == 2, "{svg}");
}

/// A dithered Perceptual gradient: its fill is an image of the perceptual mix with dither's noise
/// (within a colour level or so of the mix, and not the undithered pixels), its stroke an image of
/// the perceptual mix without noise (dither applies to fills only).
#[test]
fn a_dithered_perceptual_fill_is_the_perceptual_mix_with_noise() {
    use vectorcraft_color::GradientMethod;
    let paint = |dither| {
        let Paint::Gradient(mut g) = blue_yellow(GradientMethod::Perceptual) else { panic!("a gradient") };
        g.gradient.dither = dither;
        Paint::Gradient(g)
    };
    let png = |dither, stroke: bool| {
        let shape = shapes::rectangle(Rect::new(20.0, 20.0, 180.0, 140.0));
        let (fill, stroke) = if stroke { (Paint::None, paint(dither)) } else { (paint(dither), Paint::None) };
        let mut d = doc_with(Node::path(NodeId(0), shape, Appearance::basic(fill, stroke, 4.0)));
        d.raster_effects_ppi = 144.0;
        let svg = export_full(&d, &ExportOptions::default(), None).svg;
        assert_eq!(svg.matches("<image ").count(), 1, "{svg}");
        assert!(!svg.contains("<linearGradient"), "{svg}");
        png_of(&svg)
    };
    let (dithered, plain) = (png(true, false), png(false, false));
    assert_eq!(dithered.dimensions(), (320, 240));
    let Paint::Gradient(want) = paint(false) else { panic!("a gradient") };
    let mut noisy = 0;
    for x in 0..320 {
        let (p, q) = (dithered.get_pixel(x, 120).0, plain.get_pixel(x, 120).0);
        let w = want.gradient.sample((x as f32 + 0.5) / 320.0).0.to_rgb();
        for k in 0..3 {
            let off = (f32::from(p[k]) - w[k].clamp(0.0, 1.0) * 255.0).abs();
            assert!(off <= 2.0, "column {x}, channel {k}: {p:?} vs {w:?}");
        }
        noisy += usize::from(p != q);
    }
    assert!(noisy > 100, "dither's noise: {noisy} of 320 pixels differ");
    assert_eq!(png(true, true), png(false, true), "a stroke has no dither");
}
