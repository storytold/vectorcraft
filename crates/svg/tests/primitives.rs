//! A rectangle, circle, or ellipse exports as that element only when the path is that shape.
//! A bowtie, a repeated corner, and a near-ellipse stay paths. resvg must paint the real shapes
//! the way the canvas does.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use kurbo::Vec2;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, FillLayer, Node};
use vectorcraft_geom::{PathData, Point, Rect, SubPath, shapes};
use vectorcraft_svg::{ExportOptions, export};
use vectorcraft_testkit::raster::{Image, assert_similar, render_artboard};

fn filled(path: PathData) -> Document {
    let mut d = Document::new(120.0, 90.0);
    let n = Node::path(
        d.alloc_id(),
        path,
        Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::BLACK)))], ..Default::default() },
    );
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, n).unwrap();
    d
}

fn resvg_render(svg: &str) -> Image {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).expect("parse");
    let mut pm = resvg::tiny_skia::Pixmap::new(120, 90).unwrap();
    pm.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pm.as_mut());
    Image { width: 120, height: 90, rgba: pm.data().to_vec() }
}

fn paints_like_the_canvas(doc: &Document, svg: &str) {
    assert_similar(&render_artboard(doc), &resvg_render(svg), 24.0, 0.002);
}

#[test]
fn a_rectangle_circle_and_ellipse_export_as_those_elements() {
    let rect = filled(shapes::rectangle(Rect::new(10.0, 10.0, 60.0, 40.0)));
    let rect_svg = export(&rect, &ExportOptions::default());
    assert!(rect_svg.contains("<rect "), "{rect_svg}");
    paints_like_the_canvas(&rect, &rect_svg);

    let circle = filled(shapes::ellipse(Rect::new(20.0, 15.0, 70.0, 65.0)));
    let circle_svg = export(&circle, &ExportOptions::default());
    assert!(circle_svg.contains("<circle "), "{circle_svg}");
    paints_like_the_canvas(&circle, &circle_svg);

    let ellipse = filled(shapes::ellipse(Rect::new(15.0, 20.0, 100.0, 60.0)));
    let ellipse_svg = export(&ellipse, &ExportOptions::default());
    assert!(ellipse_svg.contains("<ellipse "), "{ellipse_svg}");
    paints_like_the_canvas(&ellipse, &ellipse_svg);
}

#[test]
fn a_bowtie_a_repeated_corner_and_a_near_ellipse_stay_paths() {
    let bowtie = filled(PathData::single(SubPath::polyline(
        &[Point::new(10.0, 10.0), Point::new(60.0, 40.0), Point::new(60.0, 10.0), Point::new(10.0, 40.0)],
        true,
    )));
    let bowtie_svg = export(&bowtie, &ExportOptions::default());
    assert!(bowtie_svg.contains("<path "), "{bowtie_svg}");
    assert!(!bowtie_svg.contains("<rect "), "{bowtie_svg}");

    let triangle = filled(PathData::single(SubPath::polyline(
        &[Point::new(10.0, 10.0), Point::new(60.0, 10.0), Point::new(60.0, 40.0), Point::new(60.0, 40.0)],
        true,
    )));
    let triangle_svg = export(&triangle, &ExportOptions::default());
    assert!(triangle_svg.contains("<path "), "{triangle_svg}");
    assert!(!triangle_svg.contains("<rect "), "{triangle_svg}");

    let mut path = shapes::ellipse(Rect::new(40.0, 40.0, 42.0, 42.0));
    path.subpaths[0].anchors[0].h_out += Vec2::new(0.75, 0.0);
    let near = filled(path);
    let near_svg = export(&near, &ExportOptions::default());
    assert!(near_svg.contains("<path "), "{near_svg}");
    assert!(!near_svg.contains("<circle ") && !near_svg.contains("<ellipse "), "{near_svg}");
}
