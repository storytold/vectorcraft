//! Simple shapes export as SVG basic shapes (#4652): axis-aligned rectangles, circles and
//! ellipses, straight lines, and polygons/polylines of straight segments. Everything else, and
//! rectangles and ellipses whose dashes would start elsewhere, stays `<path>`; the picture and the
//! reimported geometry don't change.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Dash, Document, FillLayer, Node, StrokeLayer};
use vectorcraft_geom::{Affine, PathData, Point, Rect, SubPath, shapes};
use vectorcraft_svg::{ExportOptions, export, import};
use vectorcraft_testkit::raster::{Image, assert_similar, render_artboard};

fn painted(dash: bool) -> Appearance {
    let mut st = StrokeLayer::new(Paint::solid(Color::rgb(0.1, 0.2, 0.6)), 3.0);
    if dash {
        st.dash = Some(Dash { pattern: vec![6.0, 4.0], offset: 0.0, align_corners: false });
    }
    Appearance {
        items: vec![AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::rgb(0.9, 0.5, 0.1)))), AppearanceItem::Stroke(st)],
        ..Default::default()
    }
}

fn doc(paths: Vec<(PathData, bool)>) -> Document {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    for (p, dash) in paths {
        let n = Node::path(d.alloc_id(), p, painted(dash));
        d.insert(Some(l), usize::MAX, n).unwrap();
    }
    d
}

fn svg_of(p: PathData, dash: bool) -> String {
    export(&doc(vec![(p, dash)]), &ExportOptions::default())
}

fn poly(pts: &[(f64, f64)], closed: bool) -> PathData {
    let pts: Vec<Point> = pts.iter().map(|&(x, y)| Point::new(x, y)).collect();
    PathData::single(SubPath::polyline(&pts, closed))
}

#[test]
fn each_simple_shape_writes_its_element() {
    let cases = [
        (shapes::rectangle(Rect::new(10.0, 20.0, 110.0, 70.0)), "<rect x=\"10\" y=\"20\" width=\"100\" height=\"50\""),
        (shapes::ellipse(Rect::new(50.0, 50.0, 90.0, 90.0)), "<circle cx=\"70\" cy=\"70\" r=\"20\""),
        (shapes::ellipse(Rect::new(10.0, 10.0, 70.0, 40.0)), "<ellipse cx=\"40\" cy=\"25\" rx=\"30\" ry=\"15\""),
        (shapes::line(Point::new(10.0, 10.0), Point::new(90.5, 40.0)), "<line x1=\"10\" y1=\"10\" x2=\"90.5\" y2=\"40\""),
        (poly(&[(60.0, 185.0), (100.0, 185.0), (80.0, 135.0)], true), "<polygon points=\"60,185 100,185 80,135\""),
        (poly(&[(10.0, 10.0), (50.0, 30.0), (90.0, 10.0)], false), "<polyline points=\"10,10 50,30 90,10\""),
    ];
    for (p, want) in cases {
        let svg = svg_of(p, false);
        assert!(svg.contains(want), "{want}:\n{svg}");
        assert!(!svg.contains("<path"), "{want}:\n{svg}");
    }
}

#[test]
fn numbers_are_the_written_precision() {
    let opts = ExportOptions { decimals: 2, ..Default::default() };
    let svg = export(&doc(vec![(shapes::rectangle(Rect::new(10.123, 20.0, 30.456, 40.0)), false)]), &opts);
    assert!(svg.contains("<rect x=\"10.12\" y=\"20\" width=\"20.34\" height=\"20\""), "{svg}");
}

#[test]
fn turned_curved_and_dashed_shapes_keep_their_exact_form() {
    let turned = |p: PathData| p.transformed(Affine::rotate_about(0.5, Point::new(60.0, 60.0)));
    // A turned rectangle is still straight segments: a polygon, as the reference app writes one.
    let svg = svg_of(turned(shapes::rectangle(Rect::new(20.0, 40.0, 100.0, 80.0))), false);
    assert!(svg.contains("<polygon") && !svg.contains("<rect"), "{svg}");
    // Polygons start their dashes where the path does; rectangles and ellipses don't.
    let svg = svg_of(shapes::rectangle(Rect::new(20.0, 40.0, 100.0, 80.0)), true);
    assert!(svg.contains("<polygon points=\"20,40 100,40 100,80 20,80\""), "{svg}");
    let ellipse = shapes::ellipse(Rect::new(20.0, 40.0, 100.0, 80.0));
    for (p, dash) in [(turned(ellipse.clone()), false), (ellipse, true)] {
        let svg = svg_of(p, dash);
        assert!(svg.contains("<path") && !svg.contains("<ellipse"), "{svg}");
    }
    // A curve and a zero-length line stay paths.
    let mut curve = shapes::ellipse(Rect::new(20.0, 40.0, 100.0, 80.0));
    curve.subpaths[0].anchors[0].h_out.y += 5.0;
    for p in [curve, poly(&[(5.0, 5.0), (5.0, 5.0)], false)] {
        assert!(svg_of(p, false).contains("<path"));
    }
}

/// The exported SVG rendered by resvg on white at 1 px/pt.
fn resvg_render(svg: &str) -> Image {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).expect("parse");
    let mut pm = resvg::tiny_skia::Pixmap::new(200, 200).unwrap();
    pm.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pm.as_mut());
    Image { width: 200, height: 200, rgba: pm.data().to_vec() }
}

fn shapes_doc() -> Document {
    doc(vec![
        (shapes::rectangle(Rect::new(10.0, 20.0, 110.0, 70.0)), false),
        (shapes::rectangle(Rect::new(120.0, 20.0, 190.0, 70.0)), true),
        (shapes::ellipse(Rect::new(10.0, 80.0, 50.0, 120.0)), false),
        (shapes::ellipse(Rect::new(60.0, 80.0, 140.0, 120.0)), false),
        (shapes::line(Point::new(150.0, 80.0), Point::new(190.0, 120.0)), true),
        (poly(&[(20.0, 185.0), (60.0, 185.0), (40.0, 135.0)], true), false),
        (poly(&[(80.0, 140.0), (120.0, 180.0), (160.0, 140.0)], false), true),
    ])
}

#[test]
fn shape_elements_render_like_the_canvas() {
    let d = shapes_doc();
    let svg = export(&d, &ExportOptions::default());
    assert!(!svg.contains("<path"), "{svg}");
    assert_similar(&render_artboard(&d), &resvg_render(&svg), 24.0, 0.002);
}

#[test]
fn shape_elements_reimport_with_the_same_geometry() {
    let d = shapes_doc();
    let back = import(&export(&d, &ExportOptions::default())).unwrap();
    let bounds = |d: &Document| {
        let mut v = Vec::new();
        d.walk(|n| {
            if !n.is_layer() {
                v.push(n.geometric_bounds().unwrap());
            }
        });
        v
    };
    let (a, b) = (bounds(&d), bounds(&back));
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(&b) {
        let far = [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().any(|v| v.abs() > 1e-3);
        assert!(!far, "{a:?} came back as {b:?}");
    }
}
