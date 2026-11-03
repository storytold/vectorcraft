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

/// The paths of the objects in `d`, in order.
fn paths(d: &Document) -> Vec<PathData> {
    let mut v = Vec::new();
    d.walk(|n| v.extend(n.path_data().cloned()));
    v
}

#[test]
fn circles_and_ellipses_import_as_the_ellipse_drawn() {
    // What export writes comes back with the anchors and handles it was written from.
    let drawn = [shapes::ellipse(Rect::new(10.0, 80.0, 50.0, 120.0)), shapes::ellipse(Rect::new(60.0, 80.0, 140.0, 120.0))];
    let d = doc(drawn.iter().map(|p| (p.clone(), false)).collect());
    let back = paths(&import(&export(&d, &ExportOptions::default())).unwrap());
    assert_eq!(back.len(), 2);
    for (want, got) in drawn.iter().zip(&back) {
        let [w, g] = [want, got].map(|p| &p.subpaths[0].anchors);
        assert!(got.subpaths.len() == 1 && got.subpaths[0].closed && g.len() == 4, "{got:?}");
        // The same four anchors, starting on the right as an SVG ellipse does.
        for a in g {
            let same = w.iter().any(|b| a.p.distance(b.p) < 1e-4 && a.h_in.distance(b.h_in) < 1e-4 && a.h_out.distance(b.h_out) < 1e-4);
            assert!(same, "{a:?} is not one of {w:?}");
        }
        assert!((g[0].p.x - want.bounds().unwrap().x1).abs() < 1e-4, "{g:?}");
    }
    // From another app, with a transform: four anchors on the axes, transformed.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">
        <circle cx="50" cy="50" r="40" transform="translate(10 20)"/>
        <ellipse cx="100" cy="100" rx="60" ry="20" transform="rotate(30 100 100)"/>
        <ellipse cx="36.255" cy="154.118" rx="36.255" ry="5.421"/>
        <circle cx="40000000" cy="0" r="400"/>
        <ellipse cx="1000000" cy="1000000" rx="36" ry="1"/></svg>"#;
    let back = paths(&import(svg).unwrap());
    assert_eq!(back.len(), 5);
    let circle = &back[0].subpaths[0].anchors;
    let want: Vec<Point> = [(100.0, 70.0), (60.0, 110.0), (20.0, 70.0), (60.0, 30.0)].map(|(x, y)| Point::new(x, y)).into();
    assert_eq!(circle.len(), 4);
    for (a, w) in circle.iter().zip(&want) {
        assert!(a.p.distance(*w) < 1e-4, "{circle:?}");
    }
    let turned = &back[1].subpaths[0].anchors;
    let r = Affine::rotate_about(30f64.to_radians(), Point::new(100.0, 100.0));
    assert!(turned.len() == 4 && turned[0].p.distance(r * Point::new(160.0, 100.0)) < 1e-3, "{turned:?}");
    // However many curves the arcs of a flat ellipse make.
    let flat = &back[2].subpaths[0].anchors;
    assert!(flat.len() == 4 && flat[0].p.distance(Point::new(72.51, 154.118)) < 1e-3, "{flat:?}");
    // A small circle far from the origin too.
    let far = &back[3].subpaths[0].anchors;
    assert!(far.len() == 4 && far[0].p.distance(Point::new(40_000_400.0, 0.0)) < 8.0, "{far:?}");
    // A radius of a few of f32's steps at its coordinates, with handles of the standard length.
    let thin = &back[4].subpaths[0].anchors;
    let handle = thin.first().map(|a| a.h_out - a.p);
    assert!(thin.len() == 4 && thin[0].p.distance(Point::new(1_000_036.0, 1_000_000.0)) < 0.25, "{thin:?}");
    assert!(handle.is_some_and(|h| h.x.abs() < 1e-6 && (h.y - shapes::KAPPA).abs() < 1e-6), "{thin:?}");
}

#[test]
fn circles_import_without_changing_ids_or_style_matches() {
    // The file's own ids stay unique: the `<use>` shows the rectangle, not the circle.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="200" height="200">
        <circle cx="30" cy="30" r="20" fill="#0000ff"/>
        <defs><rect id="vectorcraft-id-0" width="40" height="40" fill="#ff0000"/></defs>
        <use xlink:href="#vectorcraft-id-0" x="120" y="120"/></svg>"##;
    let img = render_artboard(&import(svg).unwrap());
    assert_eq!(img.over_white(30, 30), [0, 0, 255]);
    assert_eq!(img.over_white(140, 140), [255, 0, 0]);
    // A style sheet's `[id]` selector still matches only the circle that has an id.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">
        <style>circle[id] { fill: #ff0000 }</style>
        <circle cx="30" cy="30" r="20" fill="#0000ff"/>
        <circle id="named" cx="130" cy="30" r="20" fill="#0000ff"/></svg>"##;
    let img = render_artboard(&import(svg).unwrap());
    assert_eq!(img.over_white(30, 30), [0, 0, 255]);
    assert_eq!(img.over_white(130, 30), [255, 0, 0]);
}

#[test]
fn circles_import_without_an_id_the_file_names() {
    // A style sheet's rule for an id no element has does not reach the circle.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">
        <style>#vectorcraft-id-0 { fill: #ff0000 }</style>
        <circle cx="30" cy="30" r="20" fill="#000000"/></svg>"##;
    let img = render_artboard(&import(svg).unwrap());
    assert_eq!(img.over_white(30, 30), [0, 0, 0]);
}

#[test]
fn circles_import_without_an_id_a_use_names() {
    // A `<use>` of an id no element has does not show the circle, nor does one that writes the
    // id with a character reference.
    for href in ["#vectorcraft-id-0", "#vectorcraft-id-&#48;"] {
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="200" height="200">
            <circle cx="30" cy="30" r="20" fill="#000000"/>
            <use xlink:href="{href}" x="100"/></svg>"##
        );
        let img = render_artboard(&import(&svg).unwrap());
        assert_eq!(img.over_white(30, 30), [0, 0, 0], "{href}");
        assert_eq!(img.over_white(130, 30), [255, 255, 255], "{href}");
    }
}
