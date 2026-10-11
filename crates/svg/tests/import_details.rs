//! SVG import details: linked images read from the SVG's folder and kept linked (a placeholder
//! and a warning when missing), SVG images as vector art, nested clips, undisplayed objects as
//! hidden objects, and reflected or repeated gradients.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use vectorcraft_doc::{Document, ImageObject, LinkInfo, Node, NodeKind};
use vectorcraft_geom::Rect;
use vectorcraft_svg::{ImportOptions, import_with, import_with_report};
use vectorcraft_testkit::raster::{Image, assert_similar, render_artboard};

fn svg(body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="200" height="120">{body}</svg>"#)
}

/// All non-layer nodes in paint order.
fn art(d: &Document) -> Vec<&Node> {
    let mut v = Vec::new();
    d.walk(|n| {
        if !n.is_layer() {
            v.push(n)
        }
    });
    v
}

fn images(d: &Document) -> Vec<(&Node, &ImageObject)> {
    art(d)
        .into_iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Image(im) => Some((n, im)),
            _ => None,
        })
        .collect()
}

/// The SVG rendered by resvg on white at 1 px/pt.
fn resvg_render(svg: &str, w: u32, h: u32) -> Image {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options { dpi: 72.0, ..Default::default() }).expect("parse");
    let mut pm = resvg::tiny_skia::Pixmap::new(w, h).unwrap();
    pm.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pm.as_mut());
    Image { width: w, height: h, rgba: pm.data().to_vec() }
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![];
    image::RgbaImage::from_pixel(w, h, image::Rgba([10, 200, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// A fresh folder for one test's files.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vectorcraft-svg-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("img")).unwrap();
    dir
}

/// Reads files as the app does (the link knows the size and hash).
fn read(path: &str) -> Option<(Vec<u8>, LinkInfo)> {
    let bytes = std::fs::read(path).ok()?;
    let link = LinkInfo { size: Some(bytes.len() as u64), hash: Some(vectorcraft_doc::links::hash_bytes(&bytes)), ..LinkInfo::new(path) };
    Some((bytes, link))
}

fn open_in(dir: &Path, body: &str) -> (Document, Vec<String>) {
    let folder = dir.to_string_lossy();
    import_with(&svg(body), &ImportOptions { folder: Some(&folder), read: Some(&read) }).unwrap()
}

#[test]
fn a_linked_png_resolves_against_the_svg_folder_and_stays_linked() {
    let dir = folder("linked");
    let bytes = png(40, 20);
    std::fs::write(dir.join("img").join("photo one.png"), &bytes).unwrap();
    let (d, w) = open_in(&dir, r#"<image href="img/photo%20one.png" x="10" y="20" width="80" height="40"/>"#);
    assert!(w.is_empty(), "{w:?}");
    let [(_, im)] = images(&d)[..] else { panic!("{:?}", art(&d)) };
    let link = im.link.as_ref().unwrap();
    assert_eq!(Path::new(&link.path), dir.join("img").join("photo one.png"));
    assert_eq!(link.hash, Some(vectorcraft_doc::links::hash_bytes(&bytes)));
    assert_eq!((im.width, im.height), (40, 20));
    assert_eq!(*d.images[&im.key].bytes, bytes, "the file's pixels");
    let b = im.xf.transform_rect_bbox(Rect::new(0.0, 0.0, 40.0, 20.0));
    assert!((b.x0 - 10.0).abs() < 1e-6 && (b.y0 - 20.0).abs() < 1e-6 && (b.width() - 80.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn a_missing_link_warns_and_keeps_a_placeholder_in_its_box() {
    let dir = folder("missing");
    let (d, w) = open_in(&dir, r#"<image href="img/gone.png" x="10" y="20" width="100" height="50"/>"#);
    assert!(w.iter().any(|w| w.contains("gone.png") && w.contains("not found")), "{w:?}");
    let [(_, im)] = images(&d)[..] else { panic!("{:?}", art(&d)) };
    assert_eq!(Path::new(&im.link.as_ref().unwrap().path), dir.join("img").join("gone.png"));
    assert!(d.images[&im.key].is_proxy(), "a placeholder, not the file");
    // Stretched to the image's box, whatever its aspect.
    let b = im.xf.transform_rect_bbox(Rect::new(0.0, 0.0, im.width as f64, im.height as f64));
    assert!((b.x0 - 10.0).abs() < 1e-6 && (b.width() - 100.0).abs() < 1e-6 && (b.height() - 50.0).abs() < 1e-6, "{b:?}");
    // Without a reader nothing is read: the same placeholder.
    let (d, w) = import_with_report(&svg(r#"<image href="/abs/photo.png" width="10" height="10"/>"#)).unwrap();
    assert_eq!(images(&d)[0].1.link.as_ref().unwrap().path, "/abs/photo.png");
    assert!(!w.is_empty());
}

#[test]
fn svg_images_import_as_vector_art() {
    let dir = folder("svgimage");
    let inner =
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10" viewBox="0 0 20 10"><rect width="10" height="10" fill="#f00"/></svg>"##;
    std::fs::write(dir.join("img").join("logo.svg"), inner).unwrap();
    let data = format!("data:image/svg+xml;base64,{}", vectorcraft_svg::base64_encode(inner.as_bytes()));
    let body =
        format!(r#"<image href="img/logo.svg" x="10" y="10" width="40" height="20"/><image href="{data}" x="100" y="10" width="40" height="20"/>"#);
    let (d, w) = open_in(&dir, &body);
    assert!(w.is_empty(), "{w:?}");
    assert!(images(&d).is_empty());
    let mut boxes: Vec<Rect> = art(&d).iter().filter(|n| n.path_data().is_some()).filter_map(|n| n.geometric_bounds()).collect();
    boxes.sort_by(|a, b| a.x0.total_cmp(&b.x0));
    assert_eq!(boxes.len(), 2, "{:?}", art(&d));
    assert!((boxes[0].x0 - 10.0).abs() < 1e-6 && (boxes[0].width() - 20.0).abs() < 1e-6, "{boxes:?}");
    assert!((boxes[1].x0 - 100.0).abs() < 1e-6 && (boxes[1].height() - 20.0).abs() < 1e-6, "{boxes:?}");
}

#[test]
fn nested_clips_intersect() {
    let body = r##"<defs><clipPath id="inner"><rect x="60" y="0" width="200" height="200"/></clipPath>
        <clipPath id="outer" clip-path="url(#inner)"><circle cx="70" cy="60" r="40"/></clipPath></defs>
        <rect width="200" height="120" fill="#06c" clip-path="url(#outer)"/>"##;
    let (d, w) = import_with_report(&svg(body)).unwrap();
    assert!(w.is_empty(), "{w:?}");
    let a = art(&d);
    assert!(matches!(&a[0].kind, NodeKind::Group { clip: true, children } if matches!(children[1].kind, NodeKind::Group { clip: true, .. })));
    let mine = render_artboard(&d);
    assert_similar(&mine, &resvg_render(&svg(body), 200, 120), 24.0, 0.003);
    // The left half of the circle is cut.
    assert_eq!(mine.over_white(45, 60), [255, 255, 255]);
    assert_ne!(mine.over_white(90, 60), [255, 255, 255]);
}

#[test]
fn undisplayed_objects_import_hidden() {
    let body = r##"<rect width="10" height="10" display="none"/><g style="display:none"><circle cx="50" cy="50" r="5"/></g>
        <path id="p" d="M0 0H5V5Z" style="display: none"/><image width="4" height="4" display="none" href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGD4DwABBAEAwS2OUAAAAABJRU5ErkJggg=="/>
        <defs><rect id="t" width="3" height="3" display="none"/></defs><rect x="20" width="10" height="10"/>"##;
    let (d, w) = import_with_report(&svg(body)).unwrap();
    assert!(w.is_empty(), "{w:?}");
    let top = d.layers[0].children().unwrap();
    let hidden: Vec<bool> = top.iter().map(|n| !n.visible).collect();
    assert_eq!(hidden, [true, true, true, true, false], "{top:?}");
    // An id-less group of one object is that object.
    assert!(top[1].path_data().is_some());
    assert!(matches!(top[3].kind, NodeKind::Image(_)));
    assert_eq!(top[2].name.as_deref(), Some("p"), "named after its id");
    assert!(top[0].name.is_none() && top[1].name.is_none(), "made-up ids aren't names");
}

#[test]
fn reflect_and_repeat_match_resvg() {
    for (spread, grad) in [
        (
            "reflect",
            r##"<linearGradient id="g" x1="40" y1="0" x2="70" y2="0" gradientUnits="userSpaceOnUse" spreadMethod="reflect"><stop offset="0.2" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient>"##,
        ),
        (
            "repeat",
            r##"<linearGradient id="g" x1="0.3" y1="0" x2="0.5" y2="0.3" spreadMethod="repeat"><stop offset="0" stop-color="#fc0"/><stop offset="0.5" stop-color="#063"/><stop offset="1" stop-color="#fff"/></linearGradient>"##,
        ),
        (
            "radial",
            r##"<radialGradient id="g" cx="100" cy="60" r="25" fx="90" fy="55" gradientUnits="userSpaceOnUse" spreadMethod="reflect"><stop offset="0" stop-color="#000"/><stop offset="1" stop-color="#0cf"/></radialGradient>"##,
        ),
    ] {
        let body = format!(r#"<defs>{grad}</defs><rect x="10" y="10" width="180" height="100" fill="url(#g)"/>"#);
        let (d, w) = import_with_report(&svg(&body)).unwrap();
        assert!(w.is_empty(), "{spread}: {w:?}");
        assert_similar(&render_artboard(&d), &resvg_render(&svg(&body), 200, 120), 24.0, 0.01);
    }
}
