//! Gradients that repeat or reflect: SVG export writes `spreadMethod` over one run of the stops
//! (a conic's wedges and a diamond's quarters repeat with them), and an independent renderer
//! draws the file as the canvas draws the gradient; SVG import reads `spreadMethod` as the
//! gradient's spread; CSS writes repeating gradients.

use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientSpread, GradientStop, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeKind};
use vectorcraft_geom::{Point, Rect, Vec2, shapes};
use vectorcraft_svg::{ExportOptions, export_full, import};
use vectorcraft_testkit::raster::{Image, assert_similar, render_artboard};

/// The SVG rendered by resvg on white at 1 px/pt.
fn resvg_render(svg: &str, w: u32, h: u32) -> Image {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).expect("parse");
    let mut pm = resvg::tiny_skia::Pixmap::new(w, h).unwrap();
    pm.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pm.as_mut());
    Image { width: w, height: h, rgba: pm.data().to_vec() }
}

/// A red-to-blue gradient of `kind` that spreads by `spread` 3 times, placed at `geom`.
fn paint(kind: GradientKind, spread: GradientSpread, geom: GradientGeom) -> Paint {
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let mut g = GradientPaint::new(Gradient { spread, repeats: 3, ..Gradient::new(kind, stops) });
    g.geom = Some(geom);
    Paint::Gradient(Box::new(g))
}

/// A 200 × 160 document holding a rectangle painted with `fill` and `stroke`.
fn doc(fill: Paint, stroke: Paint, width: f64) -> Document {
    let mut d = Document::new(200.0, 160.0);
    let n = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(20.0, 20.0, 180.0, 140.0)), Appearance::basic(fill, stroke, width));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

fn cases() -> Vec<(&'static str, GradientKind, GradientGeom)> {
    let c = Point::new(100.0, 80.0);
    let geom = GradientGeom { start: c, end: c + Vec2::new(50.0, -20.0), aspect: 0.7, focal: None };
    vec![
        ("linear", GradientKind::Linear, GradientGeom { start: Point::new(60.0, 60.0), end: Point::new(150.0, 90.0), aspect: 1.0, focal: None }),
        ("radial", GradientKind::Radial, geom),
        ("off-center radial", GradientKind::Radial, GradientGeom { focal: Some(c + Vec2::new(-12.0, 10.0)), ..geom }),
        ("conic", GradientKind::Conic, geom),
        ("diamond", GradientKind::Diamond, geom),
    ]
}

/// Each kind's fill and stroke, repeating and reflecting, exported and drawn by resvg, looks as the
/// canvas paints it, inside the vector and past its ends.
#[test]
fn an_independent_renderer_draws_the_export_as_the_canvas_does() {
    for (what, kind, geom) in cases() {
        for spread in [GradientSpread::Repeat, GradientSpread::Reflect] {
            for stroke in [false, true] {
                let d = if stroke { doc(Paint::None, paint(kind, spread, geom), 14.0) } else { doc(paint(kind, spread, geom), Paint::None, 0.0) };
                let o = export_full(&d, &ExportOptions::default(), None);
                if matches!(kind, GradientKind::Linear | GradientKind::Radial | GradientKind::Diamond) {
                    let attr = if spread == GradientSpread::Repeat { "spreadMethod=\"repeat\"" } else { "spreadMethod=\"reflect\"" };
                    assert!(o.svg.contains(attr), "{what} {spread:?}: {}", o.svg);
                }
                // Hard edges (where Repeat starts the stops again, and between a conic's wedges)
                // differ by a pixel's antialiasing at most.
                assert_similar(&render_artboard(&d), &resvg_render(&o.svg, 200, 160), 10.0, 0.03);
            }
        }
    }
}

/// One run lies along the vector: a linear gradient's `x2` is a third of the way along it.
#[test]
fn the_written_vector_holds_one_run() {
    let geom = GradientGeom { start: Point::new(30.0, 50.0), end: Point::new(120.0, 50.0), aspect: 1.0, focal: None };
    let o = export_full(&doc(paint(GradientKind::Linear, GradientSpread::Repeat, geom), Paint::None, 0.0), &ExportOptions::default(), None);
    assert!(o.svg.contains("x1=\"30\" y1=\"50\" x2=\"60\" y2=\"50\" spreadMethod=\"repeat\""), "{}", o.svg);
}

/// On type characters, where SVG writes a conic as a linear gradient along its vector, the runs
/// lie along that vector: a third of it each.
#[test]
fn a_conic_on_characters_repeats_along_its_vector() {
    use vectorcraft_doc::{CharStyle, TextObject};
    let geom = GradientGeom { start: Point::new(30.0, 50.0), end: Point::new(120.0, 50.0), aspect: 1.0, focal: None };
    let Paint::Gradient(g) = paint(GradientKind::Conic, GradientSpread::Repeat, geom) else { panic!("a gradient") };
    let st = CharStyle { fill: Paint::Gradient(g), size: 48.0, ..CharStyle::default() };
    let mut d = Document::new(200.0, 160.0);
    let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 80.0), "Fan", st))));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let o = export_full(&d, &ExportOptions::default(), None);
    assert!(o.svg.contains("x1=\"30\" y1=\"50\" x2=\"60\" y2=\"50\" spreadMethod=\"repeat\""), "{}", o.svg);
}

/// `spreadMethod` reads as the gradient's spread, with its stops and vector as written.
#[test]
fn import_reads_spread_method_as_the_spread() {
    for (attr, want) in [("repeat", GradientSpread::Repeat), ("reflect", GradientSpread::Reflect), ("pad", GradientSpread::Pad)] {
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120"><defs><linearGradient id="g" x1="40" y1="0" x2="70" y2="0" gradientUnits="userSpaceOnUse" spreadMethod="{attr}"><stop offset="0.2" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient></defs><rect x="10" y="10" width="180" height="100" fill="url(#g)"/></svg>"##
        );
        let d = import(&svg).unwrap();
        let mut found = None;
        d.walk(|n| {
            if let (NodeKind::Path { .. }, Paint::Gradient(g)) = (&n.kind, n.appearance.fill_paint()) {
                found = Some(g.clone());
            }
        });
        let g = found.unwrap_or_else(|| panic!("{attr}: no gradient"));
        assert_eq!((g.gradient.spread, g.gradient.repeat_count(), g.gradient.stops.len()), (want, 1, 2), "{attr}");
        let geom = g.geom.unwrap();
        assert_eq!((geom.start, geom.end), (Point::new(40.0, 0.0), Point::new(70.0, 0.0)), "{attr}");
    }
}

/// Exported and read back, a repeating gradient keeps its spread; its count becomes part of the
/// vector, so it looks the same.
#[test]
fn a_spread_survives_export_and_import() {
    let geom = GradientGeom { start: Point::new(30.0, 50.0), end: Point::new(150.0, 90.0), aspect: 1.0, focal: None };
    let d = doc(paint(GradientKind::Linear, GradientSpread::Reflect, geom), Paint::None, 0.0);
    let o = export_full(&d, &ExportOptions::default(), None);
    let back = import(&o.svg).unwrap();
    let mut spreads = vec![];
    back.walk(|n| {
        if let Paint::Gradient(g) = n.appearance.fill_paint() {
            spreads.push(g.gradient.spread);
        }
    });
    assert_eq!(spreads, [GradientSpread::Reflect]);
    assert_similar(&render_artboard(&d), &render_artboard(&back), 4.0, 0.01);
}
