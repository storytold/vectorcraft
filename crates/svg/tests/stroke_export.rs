//! Strokes in SVG export look as on the canvas: arrowheads, width profiles, dots and brushes are
//! written as the canvas's filled outlines or art, aligned strokes are clipped or masked, and open
//! paths stroke centred. Checked by rendering the exported SVG with resvg.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Arrowhead, Dash, Document, LineCap, LineJoin, Node, StrokeAlign, StrokeLayer, WidthProfile};
use vectorcraft_geom::{PathData, Point, SubPath, shapes};
use vectorcraft_svg::{ExportOptions, export};
use vectorcraft_testkit::raster::{Image, assert_similar, render_artboard};

fn add(d: &mut Document, path: PathData, f: impl FnOnce(&mut StrokeLayer)) {
    let mut st = StrokeLayer::new(Paint::solid(Color::rgb(0.1, 0.2, 0.6)), 4.0);
    f(&mut st);
    let n = Node::path(d.alloc_id(), path, Appearance { items: vec![AppearanceItem::Stroke(st)], ..Default::default() });
    let l = d.layers[0].id;
    d.insert(Some(l), usize::MAX, n).unwrap();
}

fn line(x0: f64, y: f64, x1: f64) -> PathData {
    shapes::line(Point::new(x0, y), Point::new(x1, y))
}

/// The exported SVG rendered by resvg on white at 1 px/pt. usvg defaults to 96 dpi (1 pt = 4/3 px), which
/// would scale a pt-typed SVG to 75% of its declared size; pin dpi to 72 so pt and px match.
fn resvg_render(svg: &str, w: u32, h: u32) -> Image {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options { dpi: 72.0, ..Default::default() }).expect("parse");
    let mut pm = resvg::tiny_skia::Pixmap::new(w, h).unwrap();
    pm.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pm.as_mut());
    Image { width: w, height: h, rgba: pm.data().to_vec() }
}

#[test]
fn exported_strokes_render_like_the_canvas() {
    let mut d = Document::new(200.0, 200.0);
    add(&mut d, line(20.0, 20.0, 150.0), |s| {
        s.end_arrow = Some(Arrowhead::Triangle);
        s.start_arrow = Some(Arrowhead::CircleOpen);
        s.opacity = 0.5;
    });
    add(&mut d, line(20.0, 50.0, 180.0), |s| {
        s.width = 14.0;
        s.profile = Some(WidthProfile::lens());
    });
    add(&mut d, line(20.0, 80.0, 180.0), |s| {
        s.cap = LineCap::Square;
        s.dash = Some(Dash { pattern: vec![0.0, 12.0], offset: 0.0, align_corners: false });
    });
    add(&mut d, line(20.0, 110.0, 180.0), |s| s.brush = Some("Tapered Stroke".into()));
    let tri = SubPath::polyline(&[Point::new(60.0, 185.0), Point::new(100.0, 185.0), Point::new(80.0, 135.0)], true);
    add(&mut d, PathData::single(tri), |s| {
        s.align = StrokeAlign::Outside;
        s.join = LineJoin::Miter;
        s.width = 6.0;
    });
    let svg = export(&d, &ExportOptions::default());
    assert!(!svg.contains("stroke-dasharray"), "dots are outlines:\n{svg}");
    let canvas = render_artboard(&d);
    let written = resvg_render(&svg, 200, 200);
    assert_similar(&canvas, &written, 24.0, 0.002);
    // The comparison is meaningful: every stroke drew something.
    for y in [20, 50, 80, 110, 160] {
        assert!((0..200).any(|x| canvas.over_white(x, y).iter().any(|c| *c < 200)), "row {y} has ink");
    }
}

#[test]
fn an_open_path_with_inside_alignment_exports_a_plain_stroke() {
    let mut d = Document::new(200.0, 100.0);
    add(&mut d, line(20.0, 50.0, 180.0), |s| s.align = StrokeAlign::Inside);
    let svg = export(&d, &ExportOptions::default());
    assert!(svg.contains("stroke-width=\"4\""), "{svg}");
    assert!(!svg.contains("clip-path") && !svg.contains("mask"), "{svg}");
}

#[test]
fn a_lens_profile_exports_a_fill() {
    let mut d = Document::new(200.0, 100.0);
    add(&mut d, line(20.0, 50.0, 180.0), |s| s.profile = Some(WidthProfile::lens()));
    let svg = export(&d, &ExportOptions::default());
    assert!(svg.contains("fill=\"#1a3399\"") && !svg.contains("stroke="), "{svg}");
}
