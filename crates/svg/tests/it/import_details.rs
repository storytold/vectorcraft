//! SVG import details: linked images read from the SVG's folder and kept linked (a placeholder
//! and a warning when missing), SVG images as vector art, nested clips, undisplayed objects as
//! hidden objects, and reflected or repeated gradients.

use std::path::{Path, PathBuf};

use vectorcraft_doc::{Document, ImageObject, LinkInfo, Node, NodeKind};
use vectorcraft_geom::Rect;
use vectorcraft_svg::{ImportOptions, MAX_NESTED_SVG_IMAGES, MAX_NESTED_SVG_READS, SvgError, import_with, import_with_report, import_within};
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
fn hrefs_spelling_one_file_differently_read_it_once() {
    let dir = folder("spellings");
    std::fs::write(dir.join("img").join("a.png"), png(4, 4)).unwrap();
    let reads = std::sync::atomic::AtomicUsize::new(0);
    let counting = |path: &str| {
        reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        read(path)
    };
    let folder = dir.to_string_lossy();
    let body = r#"<image href="img/a.png" width="4" height="4"/><image href="./img/a.png" width="4" height="4"/><image href="img/%61.png" width="4" height="4"/>"#;
    let (d, w) = import_with(&svg(body), &ImportOptions { folder: Some(&folder), read: Some(&counting) }).unwrap();
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(images(&d).len(), 3);
    assert_eq!(reads.into_inner(), 1, "one read for the three spellings");
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

/// Missing files show the same placeholder, each under a key of its own: the images of one file
/// share a key, and those of another file have a different one.
#[test]
fn each_missing_file_has_its_own_placeholder_key() {
    let dir = folder("missing-keys");
    let body = r#"<image href="img/a.png" width="10" height="10"/><image href="img/b.png" x="20" width="10" height="10"/><image href="img/a.png" x="40" width="10" height="10"/>"#;
    let (d, _) = open_in(&dir, body);
    let [(_, a), (_, b), (_, a2)] = images(&d)[..] else { panic!("{:?}", art(&d)) };
    assert_eq!(Path::new(&a.link.as_ref().unwrap().path), dir.join("img").join("a.png"));
    assert_eq!(Path::new(&b.link.as_ref().unwrap().path), dir.join("img").join("b.png"));
    assert_ne!(a.key, b.key, "two files, two keys");
    assert_eq!(a.key, a2.key, "one file, one key");
    assert!(d.images[&a.key].is_proxy() && d.images[&b.key].is_proxy(), "placeholders, not the files");
    assert_eq!(d.images[&a.key].bytes, d.images[&b.key].bytes, "the same placeholder");
}

/// Two files with the same pixels are linked under keys of their own: the images of one file share
/// a key, and those of the other file have a different one.
#[test]
fn files_with_the_same_pixels_have_keys_of_their_own() {
    let dir = folder("same-pixels");
    let bytes = png(40, 20);
    std::fs::write(dir.join("img").join("a.png"), &bytes).unwrap();
    std::fs::write(dir.join("img").join("b.png"), &bytes).unwrap();
    let body = r#"<image href="img/a.png" width="10" height="10"/><image href="img/b.png" x="20" width="10" height="10"/><image href="img/a.png" x="40" width="10" height="10"/>"#;
    let (d, w) = open_in(&dir, body);
    assert!(w.is_empty(), "{w:?}");
    let [(_, a), (_, b), (_, a2)] = images(&d)[..] else { panic!("{:?}", art(&d)) };
    assert_ne!(a.key, b.key, "two files, two keys");
    assert_eq!(a.key, a2.key, "one file, one key");
    assert_eq!(*d.images[&a.key].bytes, bytes);
    assert_eq!(*d.images[&b.key].bytes, bytes);
}

#[test]
fn hrefs_spelling_one_missing_file_differently_each_warn_and_keep_their_own_link() {
    let dir = folder("missing_twice");
    let reads = std::sync::atomic::AtomicUsize::new(0);
    let counting = |path: &str| {
        reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        read(path)
    };
    let folder = dir.to_string_lossy();
    let body = r#"<image href="img/gone.png" width="4" height="4"/><image href="./img/gone.png" width="4" height="4"/>"#;
    let (d, w) = import_with(&svg(body), &ImportOptions { folder: Some(&folder), read: Some(&counting) }).unwrap();
    assert_eq!(w.iter().filter(|w| w.contains("gone.png") && w.contains("not found")).count(), 2, "{w:?}");
    assert_eq!(reads.into_inner(), 1, "the failed read is cached");
    let found = images(&d);
    assert_eq!(found.len(), 2);
    for (_, im) in found {
        assert_eq!(Path::new(&im.link.as_ref().unwrap().path), dir.join("img").join("gone.png"));
    }
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

/// An SVG file of 20 × 20 holding a 5 × 5 rectangle and `images`.
fn svg_file(images: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><rect width="5" height="5"/>{images}</svg>"#)
}

/// Reads as [`read`] does, counting the reads of each file name.
fn counting(reads: &std::sync::Mutex<Vec<String>>) -> impl Fn(&str) -> Option<(Vec<u8>, LinkInfo)> + Sync + '_ {
    move |path: &str| {
        if let Ok(mut reads) = reads.lock() {
            reads.push(Path::new(path).file_name().unwrap().to_string_lossy().into_owned());
        }
        read(path)
    }
}

/// A raster an SVG file links to is read from that file's folder and stays linked, in its place in
/// the SVG file's art.
#[test]
fn a_raster_linked_from_a_linked_svg_file_is_read_from_its_folder() {
    let dir = folder("nested-link");
    let bytes = png(40, 20);
    std::fs::write(dir.join("img").join("photo.png"), &bytes).unwrap();
    std::fs::write(dir.join("img").join("inner.svg"), svg_file(r#"<image href="photo.png" x="2" y="4" width="10" height="5"/>"#)).unwrap();
    let (d, w) = open_in(&dir, r#"<image href="img/inner.svg" x="10" y="10" width="20" height="20"/>"#);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(rects(&d), 1, "the SVG file's art");
    let [(_, im)] = images(&d)[..] else { panic!("{:?}", art(&d)) };
    let link = im.link.as_ref().unwrap();
    assert_eq!(Path::new(&link.path), dir.join("img").join("photo.png"));
    assert_eq!(*d.images[&im.key].bytes, bytes, "the file's pixels");
    let b = im.xf.transform_rect_bbox(Rect::new(0.0, 0.0, 40.0, 20.0));
    assert!((b.x0 - 12.0).abs() < 1e-6 && (b.y0 - 14.0).abs() < 1e-6 && (b.width() - 10.0).abs() < 1e-6, "{b:?}");
}

/// One href in two SVG files of different folders names two files: each shows its own.
#[test]
fn one_href_in_svg_files_of_different_folders_shows_each_folders_file() {
    let dir = folder("nested-same-href");
    let (outer, inner) = (png(4, 4), png(6, 6));
    std::fs::write(dir.join("photo.png"), &outer).unwrap();
    std::fs::write(dir.join("img").join("photo.png"), &inner).unwrap();
    std::fs::write(dir.join("img").join("inner.svg"), svg_file(r#"<image href="photo.png" width="10" height="10"/>"#)).unwrap();
    let (d, w) = open_in(&dir, r#"<image href="photo.png" width="10" height="10"/><image href="img/inner.svg" x="50" width="20" height="20"/>"#);
    assert!(w.is_empty(), "{w:?}");
    let [(_, a), (_, b)] = images(&d)[..] else { panic!("{:?}", art(&d)) };
    assert_eq!(Path::new(&a.link.as_ref().unwrap().path), dir.join("photo.png"));
    assert_eq!(*d.images[&a.key].bytes, outer);
    assert_eq!(Path::new(&b.link.as_ref().unwrap().path), dir.join("img").join("photo.png"));
    assert_eq!(*d.images[&b.key].bytes, inner);
}

/// A raster an SVG file links to that can't be read (missing, or refused by the reader) gets the
/// placeholder, stretched to its box, its link and a warning, as one the imported SVG links to.
#[test]
fn a_missing_raster_linked_from_a_linked_svg_file_keeps_a_placeholder() {
    let dir = folder("nested-missing");
    std::fs::write(dir.join("img").join("secret.png"), png(4, 4)).unwrap();
    let links = r#"<image href="gone.png" x="2" y="4" width="10" height="5"/><image href="secret.png" x="2" y="12" width="4" height="4"/>"#;
    std::fs::write(dir.join("img").join("inner.svg"), svg_file(links)).unwrap();
    let refusing = |path: &str| if path.ends_with("secret.png") { None } else { read(path) };
    let folder = dir.to_string_lossy();
    let body = r#"<image href="img/inner.svg" x="10" y="10" width="20" height="20"/>"#;
    let (d, w) = import_with(&svg(body), &ImportOptions { folder: Some(&folder), read: Some(&refusing) }).unwrap();
    for name in ["gone.png", "secret.png"] {
        assert!(w.iter().any(|w| w.contains(&format!("'{name}' not found"))), "{name}: {w:?}");
    }
    let [(_, gone), (_, secret)] = images(&d)[..] else { panic!("{:?}", art(&d)) };
    assert_eq!(Path::new(&gone.link.as_ref().unwrap().path), dir.join("img").join("gone.png"));
    assert_eq!(Path::new(&secret.link.as_ref().unwrap().path), dir.join("img").join("secret.png"));
    assert!(d.images[&gone.key].is_proxy() && d.images[&secret.key].is_proxy(), "placeholders, not the files");
    let b = gone.xf.transform_rect_bbox(Rect::new(0.0, 0.0, gone.width as f64, gone.height as f64));
    assert!((b.x0 - 12.0).abs() < 1e-6 && (b.width() - 10.0).abs() < 1e-6 && (b.height() - 5.0).abs() < 1e-6, "{b:?}");
}

/// An SVG file that links to itself, or to an SVG file that links back to it, is read once each
/// time it is shown: the link back gets the placeholder and a warning.
#[test]
fn svg_files_that_link_back_keep_a_placeholder_where_they_lead_back() {
    let dir = folder("nested-cycle");
    std::fs::write(dir.join("img").join("self.svg"), svg_file(r#"<image href="self.svg" width="10" height="10"/>"#)).unwrap();
    std::fs::write(dir.join("img").join("a.svg"), svg_file(r#"<image href="b.svg" width="10" height="10"/>"#)).unwrap();
    std::fs::write(dir.join("img").join("b.svg"), svg_file(r#"<image href="./a.svg" width="10" height="10"/>"#)).unwrap();
    let folder = dir.to_string_lossy();
    for (body, files, back) in [("img/self.svg", vec!["self.svg"], "self.svg"), ("img/a.svg", vec!["a.svg", "b.svg"], "./a.svg")] {
        let reads = std::sync::Mutex::new(vec![]);
        let counted = counting(&reads);
        let body = format!(r#"<image href="{body}" width="20" height="20"/><image href="./{body}" x="50" width="20" height="20"/>"#);
        let (d, w) = import_with(&svg(&body), &ImportOptions { folder: Some(&folder), read: Some(&counted) }).unwrap();
        assert_eq!(*reads.lock().unwrap(), [files.clone(), files.clone()].concat());
        let warning = format!("linked image '{back}' leads back to an SVG file that shows it: a placeholder keeps its place until it is relinked");
        assert_eq!(w, [warning.clone(), warning]);
        assert_eq!(rects(&d), 2 * files.len(), "each file's art once in each place");
        let found = images(&d);
        assert_eq!(found.len(), 2);
        for (_, im) in found {
            assert!(d.images[&im.key].is_proxy());
            assert_eq!(Path::new(&im.link.as_ref().unwrap().path), dir.join("img").join(back.trim_start_matches("./")));
        }
    }
}

/// Two SVG files that link to each other, each shown by the imported SVG, each show the other
/// once, then the placeholder of the link back to themselves.
#[test]
fn svg_files_that_link_to_each_other_each_lead_back_to_themselves() {
    let dir = folder("nested-cycle-both");
    std::fs::write(dir.join("img").join("a.svg"), svg_file(r#"<image href="b.svg" width="10" height="10"/>"#)).unwrap();
    std::fs::write(dir.join("img").join("b.svg"), svg_file(r#"<image href="a.svg" width="10" height="10"/>"#)).unwrap();
    let (d, w) = open_in(&dir, r#"<image href="img/a.svg" width="20" height="20"/><image href="img/b.svg" x="50" width="20" height="20"/>"#);
    assert_eq!(w.len(), 2, "{w:?}");
    assert_eq!(rects(&d), 4, "each file's art once in each place");
    let [(_, a), (_, b)] = images(&d)[..] else { panic!("{:?}", art(&d)) };
    assert_eq!(Path::new(&a.link.as_ref().unwrap().path), dir.join("img").join("a.svg"));
    assert_eq!(Path::new(&b.link.as_ref().unwrap().path), dir.join("img").join("b.svg"));
}

/// An SVG file shown twice, and a raster that two SVG files link to, are read once.
#[test]
fn a_repeated_linked_svg_file_is_read_once() {
    let dir = folder("nested-repeat");
    std::fs::write(dir.join("img").join("photo.png"), png(4, 4)).unwrap();
    std::fs::write(dir.join("img").join("a.svg"), svg_file(r#"<image href="photo.png" width="10" height="10"/>"#)).unwrap();
    std::fs::write(
        dir.join("img").join("b.svg"),
        svg_file(r#"<image href="a.svg" width="10" height="10"/><image href="photo.png" width="10" height="10"/>"#),
    )
    .unwrap();
    let reads = std::sync::Mutex::new(vec![]);
    let counted = counting(&reads);
    let folder = dir.to_string_lossy();
    let body = r#"<image href="img/a.svg" width="20" height="20"/><image href="./img/a.svg" width="20" height="20"/><image href="img/b.svg" width="20" height="20"/>"#;
    let (d, w) = import_with(&svg(body), &ImportOptions { folder: Some(&folder), read: Some(&counted) }).unwrap();
    assert!(w.is_empty(), "{w:?}");
    let mut reads = reads.lock().unwrap().clone();
    reads.sort();
    assert_eq!(reads, ["a.svg", "b.svg", "photo.png"]);
    let found = images(&d);
    assert_eq!(found.len(), 4);
    assert!(found.iter().all(|(_, im)| Path::new(&im.link.as_ref().unwrap().path) == dir.join("img").join("photo.png")));
}

/// A chain of SVG files, each linking to the next, is read only as deep as SVG images may nest:
/// past it the images are left out, with a warning.
#[test]
fn a_chain_of_linked_svg_files_stops_at_the_depth_cap() {
    let dir = folder("nested-chain");
    let levels = vectorcraft_svg::MAX_ELEMENT_DEPTH;
    for i in 0..levels {
        std::fs::write(dir.join("img").join(format!("{i}.svg")), svg_file(&format!(r#"<image href="{}.svg" width="10" height="10"/>"#, i + 1)))
            .unwrap();
    }
    let reads = std::sync::Mutex::new(vec![]);
    let counted = counting(&reads);
    let folder = dir.to_string_lossy();
    let (d, w) =
        import_with(&svg(r#"<image href="img/0.svg" width="20" height="20"/>"#), &ImportOptions { folder: Some(&folder), read: Some(&counted) })
            .unwrap();
    let reads = reads.lock().unwrap().len();
    assert!(reads <= levels / 2 + 4, "{reads} reads");
    assert!(rects(&d) < levels / 2, "{}", rects(&d));
    assert!(w.iter().any(|w| w.contains("nest more than")), "{w:?}");
}

/// An SVG file reached first inside an SVG file where it nests too deep to be shown, then from the
/// imported SVG, shows the raster it links to where it fits.
#[test]
fn an_svg_file_too_deep_in_one_place_shows_its_links_in_another() {
    let dir = folder("nested-deep-then-shallow");
    let depth = vectorcraft_svg::MAX_ELEMENT_DEPTH;
    let nest = |n: usize, inner: &str| format!("{}{inner}{}", "<g>".repeat(n), "</g>".repeat(n));
    std::fs::write(dir.join("img").join("photo.png"), png(4, 4)).unwrap();
    std::fs::write(dir.join("img").join("deep.svg"), svg_file(&nest(depth * 2 / 3, r#"<image href="a.svg" width="10" height="10"/>"#))).unwrap();
    std::fs::write(dir.join("img").join("a.svg"), svg_file(&nest(depth / 2, r#"<image href="photo.png" width="10" height="10"/>"#))).unwrap();
    let (d, w) = open_in(&dir, r#"<image href="img/deep.svg" width="20" height="20"/><image href="img/a.svg" x="50" width="20" height="20"/>"#);
    assert!(w.iter().any(|w| w.contains("nest more than")), "{w:?}");
    let [(_, im)] = images(&d)[..] else { panic!("{:?} {w:?}", art(&d)) };
    assert_eq!(Path::new(&im.link.as_ref().unwrap().path), dir.join("img").join("photo.png"));
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

/// SVG 1.1 §14.3.5: a clip path clips to the union of its children's silhouettes, each under its
/// own `clip-rule` (https://www.w3.org/TR/SVG11/masking.html#EstablishingANewClippingPath).
#[test]
fn clip_path_children_unite_under_their_own_rules() {
    // Two squares wound in opposite directions overlap at 60–100: the overlap stays clipped in.
    let opposite = r##"<defs><clipPath id="c"><path d="M20 20H100V100H20Z"/><path d="M60 60V140H140V60Z"/></clipPath></defs>
        <rect width="200" height="200" fill="#f00" clip-path="url(#c)"/>"##;
    // A small non-zero square first, then an even-odd ring: the ring's hole stays out.
    let ring = r##"<defs><clipPath id="c"><path d="M0 0H10V10H0Z"/>
        <path clip-rule="evenodd" d="M40 20H160V140H40Z M70 50H130V110H70Z"/></clipPath></defs>
        <rect width="200" height="200" fill="#f00" clip-path="url(#c)"/>"##;
    let page = |body: &str| format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">{body}</svg>"#);
    for body in [opposite, ring] {
        let (d, w) = import_with_report(&page(body)).unwrap();
        assert!(w.is_empty(), "{w:?}");
        let mine = render_artboard(&d);
        assert_similar(&mine, &resvg_render(&page(body), 200, 200), 24.0, 0.003);
        // Written back out, the clip is still the union.
        let out = vectorcraft_svg::export(&d, &Default::default());
        assert_similar(&render_artboard(&vectorcraft_svg::import(&out).unwrap()), &mine, 24.0, 0.003);
    }
    let (d, _) = import_with_report(&page(opposite)).unwrap();
    let mine = render_artboard(&d);
    assert_eq!(mine.over_white(80, 80), [255, 0, 0], "the overlap is clipped in");
    assert_eq!(mine.over_white(120, 120), [255, 0, 0]);
    assert_eq!(mine.over_white(130, 30), [255, 255, 255]);
    let (d, _) = import_with_report(&page(ring)).unwrap();
    let mine = render_artboard(&d);
    assert_eq!(mine.over_white(100, 80), [255, 255, 255], "the even-odd hole stays out");
    assert_eq!(mine.over_white(50, 80), [255, 0, 0]);
    assert_eq!(mine.over_white(5, 5), [255, 0, 0]);
    // Each child keeps its own rule in the clipping group, still named after the clip path.
    let clip = &art(&d)[1];
    assert_eq!(clip.name.as_deref(), Some("c"));
    let NodeKind::Group { children, clip: false } = &clip.kind else { panic!("{clip:?}") };
    assert!(matches!(children[0].kind, NodeKind::Path { rule: vectorcraft_geom::FillRule::NonZero, .. }));
    assert!(matches!(children[1].kind, NodeKind::Compound { rule: vectorcraft_geom::FillRule::EvenOdd, .. }));
}

/// SVG 1.1 §14.3.5: a child of a clipPath element with its own clip-path is clipped by it before
/// contributing to the clip (https://www.w3.org/TR/SVG11/masking.html#EstablishingANewClippingPath).
#[test]
fn clip_path_child_clip_path_clips_child() {
    let page = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">
      <defs>
        <clipPath id="inner"><rect width="50" height="200"/></clipPath>
        <clipPath id="outer">
          <rect width="200" height="200" clip-path="url(#inner)"/>
        </clipPath>
      </defs>
      <rect width="200" height="200" fill="#f00" clip-path="url(#outer)"/>
    </svg>"##;
    let (d, w) = import_with_report(page).unwrap();
    assert!(w.is_empty(), "{w:?}");
    let mine = render_artboard(&d);
    assert_similar(&mine, &resvg_render(page, 200, 200), 24.0, 0.003);
    assert_eq!(mine.over_white(25, 25), [255, 0, 0], "the left strip is clipped in");
    assert_eq!(mine.over_white(100, 100), [255, 255, 255], "the right side is cut by inner clip");
    // Written back out, the clip stays identical.
    let out = vectorcraft_svg::export(&d, &Default::default());
    assert_similar(&render_artboard(&vectorcraft_svg::import(&out).unwrap()), &mine, 24.0, 0.003);
}

#[test]
fn clip_path_child_with_chained_clip_path_intersects() {
    let page = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">
      <defs>
        <clipPath id="base"><rect y="0" width="200" height="100"/></clipPath>
        <clipPath id="inner" clip-path="url(#base)"><rect x="0" width="50" height="200"/></clipPath>
        <clipPath id="outer">
          <rect width="200" height="200" clip-path="url(#inner)"/>
        </clipPath>
      </defs>
      <rect width="200" height="200" fill="#f00" clip-path="url(#outer)"/>
    </svg>"##;
    let (d, w) = import_with_report(page).unwrap();
    assert!(w.is_empty(), "{w:?}");
    let mine = render_artboard(&d);
    assert_similar(&mine, &resvg_render(page, 200, 200), 24.0, 0.003);
    assert_eq!(mine.over_white(25, 25), [255, 0, 0], "top left quadrant is inside both clips");
    assert_eq!(mine.over_white(25, 150), [255, 255, 255], "bottom left is cut by base clip");
    assert_eq!(mine.over_white(100, 25), [255, 255, 255], "top right is cut by inner clip");
}

/// Clip paths are shared, so children that each name the next one multiply at every level: ten
/// children on seven levels would take 10^7 reads unbounded. The import must still finish.
#[test]
fn clip_paths_whose_children_fan_out_still_import() {
    let mut defs = String::from(r#"<clipPath id="c0"><rect width="10" height="10"/></clipPath>"#);
    for n in 1..8 {
        let kids = format!(r#"<rect width="10" height="10" clip-path="url(#c{})"/>"#, n - 1).repeat(10);
        defs += &format!(r#"<clipPath id="c{n}">{kids}</clipPath>"#);
    }
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><defs>{defs}</defs><rect width="20" height="20" clip-path="url(#c7)"/></svg>"#
    );
    assert!(import_with_report(&svg).is_ok());
}

#[test]
fn clip_path_deep_chain_preserves_art() {
    let mut defs = String::from(r#"<clipPath id="c0"><rect width="100" height="100"/></clipPath>"#);
    for n in 1..=20 {
        defs += &format!(r#"<clipPath id="c{n}" clip-path="url(#c{})"><rect width="100" height="100"/></clipPath>"#, n - 1);
    }
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><defs>{defs}</defs><rect width="100" height="100" fill="#f00" clip-path="url(#c20)"/></svg>"##
    );
    let (d, w) = import_with_report(&svg).unwrap();
    assert!(!w.is_empty(), "expected warning about deep clip nesting");
    let mine = render_artboard(&d);
    assert_eq!(mine.over_white(50, 50), [255, 0, 0], "red rect should be preserved despite depth limit");
}

#[test]
fn clip_path_child_deep_chain_preserves_art() {
    let mut defs = String::from(r#"<clipPath id="c0"><rect width="100" height="100"/></clipPath>"#);
    for n in 1..=20 {
        defs += &format!(r#"<clipPath id="c{n}" clip-path="url(#c{})"><rect width="100" height="100"/></clipPath>"#, n - 1);
    }
    defs += r#"<clipPath id="outer"><rect width="100" height="100" clip-path="url(#c20)"/></clipPath>"#;
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><defs>{defs}</defs><rect width="100" height="100" fill="#f00" clip-path="url(#outer)"/></svg>"##
    );
    let (d, _w) = import_with_report(&svg).unwrap();
    let mine = render_artboard(&d);
    assert_eq!(mine.over_white(50, 50), [255, 0, 0], "red rect should be preserved despite depth limit in child clip");
}

#[test]
fn clip_path_preserves_union_when_read_budget_exhausted() {
    let mut defs = String::from(r#"<clipPath id="tile"><rect width="10" height="10"/></clipPath>"#);
    let mut kids = String::new();
    for i in 0..1100 {
        let x = (i % 20) * 10;
        let y = (i / 20) * 10;
        kids += &format!(r##"<rect x="{x}" y="{y}" width="10" height="10" clip-path="url(#tile)"/>"##);
    }
    defs += &format!(r#"<clipPath id="many">{kids}</clipPath>"#);
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="600"><defs>{defs}</defs><rect width="200" height="600" fill="#f00" clip-path="url(#many)"/></svg>"##
    );
    let (d, _w) = import_with_report(&svg).unwrap();
    let mine = render_artboard(&d);
    // Early tiles processed within the 1024 read budget must be preserved in the union.
    assert_eq!(mine.over_white(5, 5), [255, 0, 0], "early tile inside read budget is preserved");
}

#[test]
fn clip_path_empty_clips_everything() {
    let page = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
      <defs>
        <clipPath id="empty"/>
      </defs>
      <rect width="100" height="100" fill="#f00" clip-path="url(#empty)"/>
    </svg>"##;
    let (d, _w) = import_with_report(page).unwrap();
    let mine = render_artboard(&d);
    assert_eq!(mine.over_white(50, 50), [255, 255, 255], "genuinely empty clip path clips away all art");
}

#[test]
fn clip_path_child_with_empty_linked_clip_clips_everything() {
    let page = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
      <defs>
        <clipPath id="empty"/>
        <clipPath id="inner" clip-path="url(#empty)">
          <rect width="100" height="100"/>
        </clipPath>
        <clipPath id="outer">
          <rect width="100" height="100" clip-path="url(#inner)"/>
        </clipPath>
      </defs>
      <rect width="100" height="100" fill="#f00" clip-path="url(#outer)"/>
    </svg>"##;
    let (d, _w) = import_with_report(page).unwrap();
    let mine = render_artboard(&d);
    assert_eq!(mine.over_white(50, 50), [255, 255, 255], "empty linked clip path clips away all art");
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

/// #1598: the warning for a filter that neither the live effects nor an SVG filter effect can hold
/// (here, on turned objects) names the filter, its primitives and the object, and says it is drawn
/// without it (not "a group": usvg wraps the element in one). Such a filter on an object that
/// isn't turned comes in as an SVG filter effect holding the filter's XML.
#[test]
fn unsupported_filters_are_named_in_the_warning() {
    let body = r#"<filter id="f"><feColorMatrix type="saturate" values="0"/></filter><filter id="t"><feTurbulence baseFrequency="0.05"/></filter>
        <rect x="20" y="20" width="60" height="60" fill="red" transform="rotate(10 50 50)" filter="url(#f)"/>
        <rect id="r" x="100" y="20" width="60" height="60" fill="blue" transform="rotate(10 130 50)" filter="url(#t)"/>
        <rect x="20" y="90" width="20" height="20" fill="red" filter="url(#f)"/>"#;
    let (d, w) = import_with_report(&svg(body)).unwrap();
    assert!(w.iter().any(|w| w == "filter 'f' (feColorMatrix) on an object isn't supported; drawn without it"), "{w:?}");
    assert!(w.iter().any(|w| w == "filter 't' (feTurbulence) on 'r' isn't supported; drawn without it"), "{w:?}");
    assert_eq!(w.iter().filter(|w| w.contains("filter")).count(), 2, "{w:?}");
    let kept: Vec<_> = art(&d).into_iter().flat_map(|n| &n.appearance.effects).collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!((kept[0].id.as_str(), &kept[0].params["name"]), ("svg.filter", &serde_json::json!("f")));
    assert_eq!(kept[0].params["xml"], r#"<filter id="f"><feColorMatrix type="saturate" values="0"/></filter>"#, "as it is written");
    assert_eq!(art(&d).iter().filter(|n| n.path_data().is_some()).count(), 3, "all drawn");
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

/// An ordinary nested SVG image imports within budget, while `<use>`-of-nested-image fan-out
/// past the node budget is refused with `SvgError::TooLarge` (#3564).
#[test]
fn nested_image_within_budget_imports_and_use_fan_out_is_refused() {
    let inner = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><rect width="10" height="10" fill="red"/></svg>"#;
    let nested = format!("data:image/svg+xml;base64,{}", vectorcraft_svg::base64_encode(inner.as_bytes()));
    let single = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><image href="{nested}" width="20" height="20"/></svg>"#);
    // An ordinary nested image imports within a small budget (3 objects: Layer 1, Group, Rect).
    let (d, _) = import_within(&single, &ImportOptions::default(), 10).unwrap();
    assert!(!d.layers[0].children().unwrap().is_empty());

    // Fan-out duplicating the nested image via <use> exceeds the budget and is refused.
    let uses = "<use href=\"#item\"/>".repeat(20);
    let fanout = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <defs><g id="item"><image href="{nested}" width="20" height="20"/></g></defs>
            {uses}
        </svg>"#
    );
    let err = import_within(&fanout, &ImportOptions::default(), 5).unwrap_err();
    assert!(matches!(err, SvgError::TooLarge(5)), "{err:?}");
    assert_eq!(err.to_string(), "the SVG expands to more than 5 objects");

    // Highly amplified multi-level <use> tree is refused without exploding memory.
    let multi_level = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <defs>
                <g id="g0"><image href="{nested}" width="20" height="20"/></g>
                <g id="g1"><use href="#g0"/><use href="#g0"/></g>
                <g id="g2"><use href="#g1"/><use href="#g1"/></g>
                <g id="g3"><use href="#g2"/><use href="#g2"/></g>
                <g id="g4"><use href="#g3"/><use href="#g3"/></g>
            </defs>
            <use href="#g4"/>
        </svg>"##
    );
    let err = import_within(&multi_level, &ImportOptions::default(), 15).unwrap_err();
    assert!(matches!(err, SvgError::TooLarge(15)), "{err:?}");
}

/// Nested SVG images exceeding `MAX_NESTED_SVG_IMAGES` are left out with a warning note (#3564).
#[test]
fn nested_svg_images_past_count_cap_are_left_out() {
    let inner = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="5" height="5"/></svg>"#;
    let nested = format!("data:image/svg+xml;base64,{}", vectorcraft_svg::base64_encode(inner.as_bytes()));
    let images = format!(r#"<image href="{nested}" width="10" height="10"/>"#).repeat(MAX_NESTED_SVG_IMAGES + 10);
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="500" height="500">{images}</svg>"#);
    let (_d, warnings) = import_with(&svg, &ImportOptions::default()).unwrap();
    assert!(warnings.iter().any(|w| w.contains(&format!("more than {MAX_NESTED_SVG_IMAGES} nested SVG images"))));
}

/// Nested SVG images that can't be read: text that isn't XML, and an SVG without a size.
const BROKEN_SVG_IMAGES: [&str; 2] =
    [r#"<svg xmlns="http://www.w3.org/2000/svg" width="10"#, r#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"/>"#];
const SMALL_SVG_IMAGE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="5" height="5"/></svg>"#;

/// `broken` images showing the broken SVG images in turn from data URLs, then `good` images
/// showing a rectangle.
fn broken_then_good(broken: usize, good: usize) -> String {
    let broken: String = (0..broken).map(|i| image_of("image/svg+xml", BROKEN_SVG_IMAGES[i % 2])).collect();
    format!(r#"<svg {SVG_NS} width="500" height="500">{broken}{}</svg>"#, image_of("image/svg+xml", SMALL_SVG_IMAGE).repeat(good))
}

/// Only nested SVG images that decode count toward `MAX_NESTED_SVG_IMAGES`: an image after that
/// many broken ones, from data URLs or a linked file shown again and again, is imported, and the
/// images past that many decoded ones are still left out.
#[test]
fn broken_nested_svg_images_do_not_use_up_the_image_allowance() {
    let allowance = |w: &[String]| w.iter().any(|w| w.contains(&format!("more than {MAX_NESTED_SVG_IMAGES} nested SVG images")));
    let (d, w) = import_with(&broken_then_good(MAX_NESTED_SVG_IMAGES + 10, 1), &ImportOptions::default()).unwrap();
    assert_eq!(rects(&d), 1, "{w:?}");
    assert!(!w.iter().any(|w| w.contains("left out")), "{w:?}");
    let (d, w) = import_with(&broken_then_good(10, MAX_NESTED_SVG_IMAGES + 1), &ImportOptions::default()).unwrap();
    assert_eq!(rects(&d), MAX_NESTED_SVG_IMAGES, "{w:?}");
    assert!(allowance(&w), "{w:?}");

    let dir = folder("brokensvgimages");
    std::fs::write(dir.join("img").join("broken.svg"), BROKEN_SVG_IMAGES[0]).unwrap();
    std::fs::write(dir.join("img").join("good.svg"), SMALL_SVG_IMAGE).unwrap();
    let reads = std::sync::atomic::AtomicUsize::new(0);
    let counted = |path: &str| {
        reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        read(path)
    };
    let body = format!(
        r#"{}<image href="img/good.svg" width="10" height="10"/>"#,
        r#"<image href="img/broken.svg" width="10" height="10"/>"#.repeat(MAX_NESTED_SVG_IMAGES + 10)
    );
    let folder = dir.to_string_lossy();
    let (d, w) = import_with(&svg(&body), &ImportOptions { folder: Some(&folder), read: Some(&counted) }).unwrap();
    assert_eq!(rects(&d), 1, "{w:?}");
    assert!(!w.iter().any(|w| w.contains("left out")), "{w:?}");
    assert_eq!(reads.into_inner(), 2, "each file is read once");
}

/// Every nested SVG image read, decoded or not, counts toward `MAX_NESTED_SVG_READS`: past it,
/// images are left out with a warning, also when `<use>` shows one broken linked file many times.
#[test]
fn nested_svg_images_are_read_at_most_max_nested_svg_reads_times() {
    let past = |w: &[String]| w.iter().any(|w| w.contains(&format!("read at most {MAX_NESTED_SVG_READS} times")));
    let (d, w) = import_with(&broken_then_good(MAX_NESTED_SVG_READS - 1, 1), &ImportOptions::default()).unwrap();
    assert_eq!(rects(&d), 1, "{w:?}");
    assert!(!past(&w), "{w:?}");
    let (d, w) = import_with(&broken_then_good(MAX_NESTED_SVG_READS, 1), &ImportOptions::default()).unwrap();
    assert_eq!(rects(&d), 0, "{w:?}");
    assert!(past(&w), "{w:?}");
    assert!(!w.iter().any(|w| w.contains(&format!("more than {MAX_NESTED_SVG_IMAGES} nested SVG images"))), "{w:?}");

    let dir = folder("brokensvguses");
    std::fs::write(dir.join("img").join("broken.svg"), BROKEN_SVG_IMAGES[0]).unwrap();
    std::fs::write(dir.join("img").join("good.svg"), SMALL_SVG_IMAGE).unwrap();
    let uses = r##"<use href="#broken"/>"##.repeat(2 * MAX_NESTED_SVG_READS);
    let body = format!(
        r#"<defs><g id="broken"><image href="img/broken.svg" width="10" height="10"/></g></defs>{uses}<image href="img/good.svg" width="10" height="10"/>"#
    );
    let (d, w) = open_in(&dir, &body);
    assert_eq!(rects(&d), 0, "{w:?}");
    assert!(past(&w), "{w:?}");
}

/// A nested SVG image that can't be read and is left out gives no warning about its text.
#[test]
fn broken_nested_svg_images_do_not_warn_about_their_text() {
    let text = |svg: &str| {
        let file = format!(r#"<svg {SVG_NS} width="50" height="50">{}</svg>"#, image_of("image/svg+xml", svg));
        let (_, w) = import_with(&file, &ImportOptions::default()).unwrap();
        w.iter().any(|w| w.contains("text inside an SVG image"))
    };
    assert!(!text(r#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"><text>Hi</text></svg>"#));
    assert!(text(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text>Hi</text></svg>"#));
}

/// #3566: entity references in element content count toward the expansion limit, as in
/// attributes. A 10 KB entity, a second entity that refers to it 250 times, and eight references
/// to the second inside `<desc>` would expand to about 20 MB from a file of a few KB: the import
/// refuses it before the XML reader expands anything. The namespace entities drawing apps write
/// (`&ns_svg;`) still import.
#[test]
fn entities_repeated_in_element_content_are_refused_and_namespace_entities_import() {
    let bomb = format!(
        r#"<!DOCTYPE svg [<!ENTITY a "{}"><!ENTITY b "{}">]><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><desc>{}</desc><rect width="5" height="5"/></svg>"#,
        "x".repeat(10_000),
        "&a;".repeat(250),
        "&b;".repeat(8)
    );
    assert!(bomb.len() < 20_000, "{} bytes", bomb.len());
    match import_with(&bomb, &ImportOptions::default()) {
        Err(SvgError::Parse(e)) => assert!(e.contains("entities"), "{e}"),
        other => panic!("refused as a parse error, not {:?}", other.map(|(_, w)| w)),
    }
    let ns = r#"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY ns_svg "http://www.w3.org/2000/svg">]><svg xmlns="&ns_svg;" width="10" height="10"><rect width="5" height="5"/></svg>"#;
    let (d, _) = import_with(ns, &ImportOptions::default()).unwrap();
    assert_eq!(art(&d).len(), 1, "the rectangle");
}
