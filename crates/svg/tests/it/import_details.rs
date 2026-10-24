//! SVG import details: linked images read from the SVG's folder and kept linked (a placeholder
//! and a warning when missing), SVG images as vector art, nested clips, undisplayed objects as
//! hidden objects, and reflected or repeated gradients.

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
            NodeKind::Image(im) => Some((n, &**im)),
            _ => None,
        })
        .collect()
}

/// The SVG rendered by resvg on white at 1 px/pt.
fn resvg_render(svg: &str, w: u32, h: u32) -> Image {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).expect("parse");
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

/// Gzipped SVG images are unpacked before usvg reads them, within the cap of an SVGZ file.
#[test]
fn svgz_images_import_as_vector_art() {
    let dir = folder("svgzimage");
    let inner =
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10" viewBox="0 0 20 10"><rect width="10" height="10" fill="#f00"/></svg>"##;
    let gz = vectorcraft_svg::compress(inner);
    std::fs::write(dir.join("img").join("logo.svgz"), &gz).unwrap();
    let data = format!("data:image/svg+xml;base64,{}", vectorcraft_svg::base64_encode(&gz));
    let body =
        format!(r#"<image href="img/logo.svgz" x="10" y="10" width="40" height="20"/><image href="{data}" x="100" y="10" width="40" height="20"/>"#);
    let (d, w) = open_in(&dir, &body);
    assert!(w.is_empty(), "{w:?}");
    assert!(images(&d).is_empty());
    let mut boxes: Vec<Rect> = art(&d).iter().filter(|n| n.path_data().is_some()).filter_map(|n| n.geometric_bounds()).collect();
    boxes.sort_by(|a, b| a.x0.total_cmp(&b.x0));
    assert_eq!(boxes.len(), 2, "{:?}", art(&d));
    assert!((boxes[0].x0 - 10.0).abs() < 1e-6 && (boxes[0].width() - 20.0).abs() < 1e-6, "{boxes:?}");
    assert!((boxes[1].x0 - 100.0).abs() < 1e-6 && (boxes[1].height() - 20.0).abs() < 1e-6, "{boxes:?}");
}

const SVG_NS: &str = r#"xmlns="http://www.w3.org/2000/svg""#;

/// An image showing `svg`, from a base64 data URL of type `mime` (none when empty).
fn image_of(mime: &str, svg: &str) -> String {
    format!(r#"<image href="data:{mime};base64,{}" width="20" height="10"/>"#, vectorcraft_svg::base64_encode(svg.as_bytes()))
}

/// An SVG showing a chain of `levels` SVG images from data URLs of type `mime`: each holds a
/// rectangle in `n` groups in one another, and the next image beside it.
fn nested_images(levels: usize, n: usize, mime: &str) -> String {
    let mut inner = String::new();
    for _ in 0..levels {
        let (open, close) = ("<g>".repeat(n), "</g>".repeat(n));
        let svg = format!(r#"<svg {SVG_NS} width="20" height="10">{open}<rect width="10" height="10"/>{inner}{close}</svg>"#);
        inner = image_of(mime, &svg);
    }
    format!(r#"<svg {SVG_NS} width="20" height="10">{inner}</svg>"#)
}

fn rects(d: &Document) -> usize {
    art(d).iter().filter(|n| n.path_data().is_some()).count()
}

/// usvg reads an SVG image while it reads the SVG showing it, on the same stack, so images in
/// images share one depth cap, whatever type their data URL has (usvg reads SVG from
/// `image/svg+xml`, `text/plain` and untyped data): what is past it is left out, with a warning,
/// where it overflowed the stack, an abort (#3562).
#[test]
fn svg_images_in_svg_images_share_the_depth_cap() {
    let most = vectorcraft_svg::MAX_ELEMENT_DEPTH;
    let left_out = |w: &[String]| w.iter().any(|w| w.contains("SVG images were left out"));
    for mime in ["image/svg+xml", "text/plain", ""] {
        // Within the cap together: all of it.
        let (d, w) = import_with_report(&nested_images(3, 20, mime)).unwrap();
        assert_eq!(rects(&d), 3, "{mime}");
        assert!(!left_out(&w), "{mime}: {w:?}");
        // Each image past the cap; each within it, but not together (the file, the image, then
        // the groups and the rectangle in each).
        for (levels, n, kept) in [(3, 1000, 0), (12, most - 4, 1)] {
            let (d, w) = import_with_report(&nested_images(levels, n, mime)).unwrap();
            assert_eq!(rects(&d), kept, "{mime}: {levels} × {n}");
            assert!(left_out(&w), "{mime}: {levels} × {n}: {w:?}");
        }
    }
    // Shallow XML, a deep tree: a chain of `<use>`, measured as in the file itself.
    let links: String = (0..3000).map(|i| format!("<g id='u{i}'><use href='#u{}'/></g>", i + 1)).collect();
    let chain = format!(r##"<svg {SVG_NS} width="9" height="9"><defs>{links}<rect id="u3000" width="1" height="1"/></defs><use href="#u0"/></svg>"##);
    let (d, w) = import_with_report(&svg(&image_of("image/svg+xml", &chain))).unwrap();
    assert_eq!(rects(&d), 0);
    assert!(left_out(&w), "{w:?}");
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

/// #1557: `transform` as a CSS property, in CSS syntax, moves the object (and wins over the
/// attribute, as in CSS), from a `style` attribute or a `<style>` rule.
#[test]
fn css_transforms_move_objects() {
    let d = import_with(
        &svg(r#"<style>.r { transform: rotate(90deg) }</style>
            <rect id="a" x="0" y="0" width="50" height="50" fill="red" style="transform: translate(100px, 100px)"/>
            <rect id="b" x="0" y="0" width="50" height="50" fill="blue" transform="translate(0 100)" style="transform: translateX(100px)"/>
            <rect id="c" class="r" x="0" y="0" width="50" height="20" fill="green"/>"#),
        &ImportOptions::default(),
    )
    .unwrap()
    .0;
    let b: Vec<Rect> = art(&d).iter().map(|n| n.geometric_bounds().unwrap()).collect();
    let near =
        |r: Rect, x0: f64, y0: f64, x1: f64, y1: f64| [(r.x0, x0), (r.y0, y0), (r.x1, x1), (r.y1, y1)].iter().all(|(a, b)| (a - b).abs() < 1e-6);
    assert!(near(b[0], 100.0, 100.0, 150.0, 150.0), "{:?}", b[0]);
    assert!(near(b[1], 100.0, 0.0, 150.0, 50.0), "{:?}", b[1]);
    assert!(near(b[2], -20.0, 0.0, 0.0, 50.0), "{:?}", b[2]);
}

/// #1557: style sheet rules with a `:not()` selector apply, with CSS precedence: against other
/// rules, the `style` attribute, `!important`, and for a shape, a group (inherited) and a hidden
/// object.
#[test]
fn not_selectors_apply() {
    let d = import_with(
        &svg(r##"<style>
            rect:not(.keep) { fill: #00ff00 }
            #b { fill: #0000ff }
            g:not(.x) { fill: #00ffff }
            .s:not(#z) { fill: #ff00ff }
            .h:not(.shown) { display: none }
            .t:not(.u) { transform: translate(100px, 0) }
            .i:not(.u) { fill: #00ff00 !important }
            </style>
            <rect id="a" x="0" y="0" width="20" height="20" fill="#ff0000"/>
            <rect id="b" x="20" y="0" width="20" height="20" fill="#ff0000"/>
            <rect class="keep" x="40" y="0" width="20" height="20" fill="#ff0000"/>
            <g><circle cx="70" cy="10" r="10"/></g>
            <rect class="s" x="80" y="0" width="20" height="20" style="fill:#ff0000"/>
            <rect class="i" x="100" y="0" width="20" height="20" style="fill:#ff0000"/>
            <rect class="h" x="0" y="40" width="20" height="20" fill="#000000"/>
            <rect class="t" x="0" y="80" width="20" height="20" fill="#000000"/>"##),
        &ImportOptions::default(),
    )
    .unwrap()
    .0;
    let im = render_artboard(&d);
    let rgb = |x: u32, y: u32| {
        let p = im.pixel(x, y);
        [p[0], p[1], p[2]]
    };
    assert_eq!(rgb(10, 10), [0, 255, 0], "rect:not(.keep) beats the fill attribute");
    assert_eq!(rgb(30, 10), [0, 0, 255], "#b is more specific");
    assert_eq!(rgb(50, 10), [255, 0, 0], ".keep is left alone");
    assert_eq!(rgb(70, 10), [0, 255, 255], "inherited from the group");
    assert_eq!(rgb(90, 10), [255, 0, 0], "the style attribute wins");
    assert_eq!(rgb(110, 10), [0, 255, 0], "!important beats the style attribute");
    assert_eq!(rgb(10, 90), [255, 255, 255], "moved by the CSS transform");
    assert_eq!(rgb(110, 90), [0, 255, 0]);
    let hidden: Vec<&Node> = art(&d).into_iter().filter(|n| !n.visible).collect();
    assert_eq!(hidden.len(), 1, "display: none from a :not() rule comes in as a hidden object");
}

/// #1561: percentages in a `<symbol>` with a `viewBox` are of the `viewBox`, so the symbol draws as
/// the same one written with numbers.
#[test]
fn percentages_in_a_symbol_are_of_its_view_box() {
    let drawing = |w: &str, h: &str, cx: &str, cy: &str| {
        let s = svg(&format!(
            r##"<defs><symbol id="s" viewBox="0 0 100 50"><rect width="{w}" height="{h}" fill="red"/><circle cx="{cx}" cy="{cy}" r="20" fill="blue"/></symbol></defs>
            <use xlink:href="#s" x="20" y="10" width="160" height="100"/>"##
        ));
        render_artboard(&import_with(&s, &ImportOptions::default()).unwrap().0)
    };
    assert_similar(&drawing("100%", "100%", "25%", "50%"), &drawing("100", "50", "25", "25"), 24.0, 0.001);
}

/// #1598: an unsupported filter's warning names the filter, its primitives and the object, and
/// says it is drawn without it (not "a group": usvg wraps the element in one).
#[test]
fn unsupported_filters_are_named_in_the_warning() {
    let body = r#"<filter id="f"><feColorMatrix type="saturate" values="0"/></filter><filter id="t"><feTurbulence baseFrequency="0.05"/></filter>
        <rect x="20" y="20" width="60" height="60" fill="red" filter="url(#f)"/>
        <rect id="r" x="100" y="20" width="60" height="60" fill="blue" filter="url(#t)"/>"#;
    let (d, w) = import_with_report(&svg(body)).unwrap();
    assert!(w.iter().any(|w| w == "filter 'f' (feColorMatrix) on an object isn't supported; drawn without it"), "{w:?}");
    assert!(w.iter().any(|w| w == "filter 't' (feTurbulence) on 'r' isn't supported; drawn without it"), "{w:?}");
    assert_eq!(art(&d).iter().filter(|n| n.path_data().is_some()).count(), 2, "both drawn");
}

/// #1601: Inkscape's pages become artboards, at their places (through the viewBox) and with their
/// labels.
#[test]
fn inkscape_pages_become_artboards() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd" width="200" height="200" viewBox="0 0 100 100">
        <sodipodi:namedview><inkscape:page x="0" y="0" width="100" height="100" inkscape:label="Front"/><inkscape:page x="120" y="0" width="60" height="150"/><inkscape:page x="0" y="0" width="0" height="10"/></sodipodi:namedview>
        <rect x="10" y="10" width="80" height="80" fill="red"/></svg>"#;
    let d = import_with(svg, &ImportOptions::default()).unwrap().0;
    let b: Vec<(String, Rect)> = d.artboards.iter().map(|a| (a.name.clone(), a.rect)).collect();
    assert_eq!(b, vec![("Front".to_string(), Rect::new(0.0, 0.0, 200.0, 200.0)), ("Artboard 2".to_string(), Rect::new(240.0, 0.0, 360.0, 300.0))]);
    // Hostile boxes (too large to add up, not numbers, negative) are no pages, and at most 1,000 are read.
    let pages = r#"<inkscape:page x="1e308" y="0" width="1e308" height="10"/><inkscape:page x="NaN" y="0" width="10" height="10"/><inkscape:page width="inf" height="10"/><inkscape:page width="-5" height="10"/>"#.to_string()
        + &r#"<inkscape:page width="10" height="10"/>"#.repeat(1500);
    let hostile = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" width="50" height="50"><g>{pages}</g></svg>"#
    );
    let d = import_with(&hostile, &ImportOptions::default()).unwrap().0;
    assert_eq!(d.artboards.len(), 1000);
    assert!(d.artboards.iter().all(|a| a.rect == Rect::new(0.0, 0.0, 10.0, 10.0)), "{:?}", d.artboards.first());
    // A file without pages keeps its one artboard.
    let d = import_with(&svg_doc(), &ImportOptions::default()).unwrap().0;
    assert_eq!(d.artboards.len(), 1);
}

fn svg_doc() -> String {
    svg(r#"<rect x="10" y="10" width="20" height="20"/>"#)
}
