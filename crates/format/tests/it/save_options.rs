//! The native save options of the file itself: Include Linked Files, embedded ICC profiles and the
//! PDF a PDF-compatible file carries, each round-tripping (and ignored by readers that don't
//! need them).

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::links::LinkInfo;
use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind, Symbol};
use vectorcraft_format::{ICC_MIME, PRETTY_MAX_BYTES, PRETTY_MAX_OBJECTS, SaveOptions, load, load_file, pdf_content, save_file, save_with};
use vectorcraft_geom::path::{PathData, SubPath};
use vectorcraft_geom::{Affine, Point};
use vectorcraft_testkit::fixtures::DocBuilder;

fn json_of(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}

/// A document showing image `k` linked to a file; the blob holds the file's bytes and a preview.
fn linked_doc() -> Document {
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    let id = d.alloc_id();
    let im = ImageObject {
        key: "k".into(),
        width: 2,
        height: 2,
        xf: Affine::IDENTITY,
        link: Some(LinkInfo::new("/art/photo.png")),
        placement: Default::default(),
        pixels: None,
    };
    d.insert(Some(layer), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
    let mut blob = ImageBlob::new("image/png", vec![1, 2, 3, 4, 5, 6]);
    blob.proxy = Some(Arc::new(vec![9, 9]));
    d.images.insert("k".into(), blob);
    d
}

#[test]
fn include_linked_files_keeps_the_files_bytes() {
    let d = linked_doc();
    let plain = save_with(&d, &SaveOptions::default()).unwrap();
    let image = &json_of(&plain)["images"]["k"];
    assert_eq!(image["proxy"], true, "only the preview by default");
    let full = save_with(&d, &SaveOptions { include_linked: true, ..SaveOptions::default() }).unwrap();
    let image = &json_of(&full)["images"]["k"];
    assert!(image.get("proxy").is_none());
    let back = load(&full).unwrap();
    assert_eq!(back.images["k"].bytes.as_slice(), &[1, 2, 3, 4, 5, 6]);
    assert!(!back.images["k"].is_proxy());
    // Only the preview loaded (the file was never found): the preview is all there is to write.
    let mut missing = linked_doc();
    let preview = Arc::new(vec![9, 9]);
    missing.images.insert("k".into(), ImageBlob { mime: "image/png".into(), bytes: preview.clone(), proxy: Some(preview) });
    let out = save_with(&missing, &SaveOptions { include_linked: true, ..SaveOptions::default() }).unwrap();
    assert_eq!(json_of(&out)["images"]["k"]["proxy"], true);
}

#[test]
fn embedded_profiles_and_pdf_round_trip() {
    let mut d = Document::new(100.0, 100.0);
    d.color_profiles.rgb = Some("Studio RGB".into());
    let o = SaveOptions {
        profiles: [("Studio RGB".to_string(), vec![0, 1, 2, 3, 250])].into(),
        pdf: Some(b"%PDF-1.7 fake".to_vec()),
        compress: true,
        ..SaveOptions::default()
    };
    let bytes = save_with(&d, &o).unwrap();
    let f = load_file(&bytes).unwrap();
    assert_eq!(f.profiles["Studio RGB"], vec![0, 1, 2, 3, 250]);
    assert_eq!(f.doc.color_profiles.rgb.as_deref(), Some("Studio RGB"));
    assert_eq!(pdf_content(&bytes).unwrap(), b"%PDF-1.7 fake");
    // The plain file says what they are; a file without them has neither member.
    let plain = save_with(&d, &SaveOptions { compress: false, ..o }).unwrap();
    let v = json_of(&plain);
    assert_eq!((v["profiles"]["Studio RGB"]["mime"].as_str(), v["pdf"]["mime"].as_str()), (Some(ICC_MIME), Some("application/pdf")));
    let none = save_with(&d, &SaveOptions::default()).unwrap();
    assert!(json_of(&none).get("profiles").is_none() && pdf_content(&none).is_none());
    assert!(load_file(&none).unwrap().profiles.is_empty());
    // Damaged profile data is left out rather than failing the open.
    let damaged = String::from_utf8(plain).unwrap().replacen("AAECA/o=", "not base64!", 1);
    assert!(load_file(damaged.as_bytes()).unwrap().profiles.is_empty());
}

/// A document of `paths` open paths with `anchors` anchors each.
fn paths_doc(paths: usize, anchors: usize) -> Document {
    let mut b = DocBuilder::new(1000.0, 1000.0);
    for i in 0..paths {
        let points: Vec<Point> = (0..anchors).map(|j| Point::new(j as f64, i as f64 + (j % 2) as f64)).collect();
        let node = b.path_node(PathData::single(SubPath::polyline(&points, false)), Default::default());
        b.add(node);
    }
    b.doc
}

fn is_pretty(bytes: &[u8]) -> bool {
    bytes.windows(3).any(|w| w == b"\n  ")
}

#[test]
fn small_documents_are_saved_pretty() {
    let d = paths_doc(10, 10);
    assert!(SaveOptions::for_doc(&d).pretty);
    assert!(is_pretty(&save_file(&d)));
}

#[test]
fn many_objects_are_saved_compact() {
    let d = paths_doc(PRETTY_MAX_OBJECTS + 1, 2);
    assert!(!SaveOptions::for_doc(&d).pretty);
}

/// Anchors per path that make a document of `paths` paths about `bytes` of compact JSON.
fn anchors_for(paths: usize, bytes: usize) -> usize {
    let per_anchor = serde_json::to_vec(&paths_doc(1, 1000)).unwrap().len() / 1000;
    bytes / per_anchor / paths
}

#[test]
fn few_objects_with_many_anchors_are_saved_compact() {
    let d = paths_doc(3, anchors_for(3, 2 * PRETTY_MAX_BYTES));
    assert!(!SaveOptions::for_doc(&d).pretty, "3 objects, but about {} bytes", 2 * PRETTY_MAX_BYTES);
    let bytes = save_file(&d);
    assert!(!is_pretty(&bytes));
    assert_eq!(load(&bytes).unwrap().layers[0].count(), d.layers[0].count());
}

/// Anchors per path that make a document of `paths` paths about `bytes` of pretty-printed JSON.
fn pretty_anchors_for(paths: usize, bytes: usize) -> usize {
    let per_anchor = serde_json::to_vec_pretty(&paths_doc(1, 1000)).unwrap().len() / 1000;
    bytes / per_anchor / paths
}

#[test]
fn documents_under_the_size_limit_stay_pretty() {
    let d = paths_doc(2, pretty_anchors_for(2, PRETTY_MAX_BYTES / 2));
    assert!(SaveOptions::for_doc(&d).pretty);
    assert!(is_pretty(&save_file(&d)));
}

#[test]
fn the_limit_is_on_the_pretty_printed_size() {
    // One traced-sized path: well under the limit as compact JSON, over it pretty-printed.
    let d = paths_doc(1, 20_000);
    let compact = serde_json::to_vec(&d).unwrap().len();
    let pretty = serde_json::to_vec_pretty(&d).unwrap().len();
    assert!(compact < PRETTY_MAX_BYTES && pretty > PRETTY_MAX_BYTES, "compact {compact} bytes, pretty {pretty} bytes");
    assert!(!SaveOptions::for_doc(&d).pretty);
    let bytes = save_file(&d);
    assert!(!is_pretty(&bytes));
    assert_eq!(load(&bytes).unwrap().layers[0].count(), d.layers[0].count());
}

#[test]
fn the_files_indentation_counts_toward_the_limit() {
    // The pretty JSON's size and line breaks; the file indents every line after the first by two spaces more.
    let measure = |anchors: usize| {
        let json = serde_json::to_vec_pretty(&paths_doc(1, anchors)).unwrap();
        (json.len(), json.iter().filter(|&&b| b == b'\n').count())
    };
    // The most anchors for which `fits` holds (it holds for 1 and not past twice the estimate).
    let most = |fits: &dyn Fn((usize, usize)) -> bool| {
        let (mut lo, mut hi) = (1, 2 * pretty_anchors_for(1, PRETTY_MAX_BYTES));
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if fits(measure(mid)) { lo = mid } else { hi = mid - 1 }
        }
        lo
    };
    // Without the file's indentation this many anchors fit the limit, with it they do not.
    let bare = most(&|(len, _)| len <= PRETTY_MAX_BYTES);
    let (len, newlines) = measure(bare);
    assert!(len + 2 * newlines > PRETTY_MAX_BYTES, "{len} bytes, {newlines} line breaks");
    assert!(!SaveOptions::for_doc(&paths_doc(1, bare)).pretty);
    // Indented, this many fit.
    let indented = most(&|(len, newlines)| len + 2 * newlines <= PRETTY_MAX_BYTES);
    assert!(indented < bare);
    assert!(SaveOptions::for_doc(&paths_doc(1, indented)).pretty);
    assert!(!SaveOptions::for_doc(&paths_doc(1, indented + 1)).pretty);
}

#[test]
fn the_limit_is_on_the_version_written() {
    // Version 1 writes every anchor with both handles as `{x, y}` maps: this document fits in the
    // current version and not in version 1.
    let d = paths_doc(1, pretty_anchors_for(1, PRETTY_MAX_BYTES / 2));
    let v1 = SaveOptions { version: 1, ..SaveOptions::default() };
    assert!(SaveOptions::default().pretty_fits(&d));
    assert!(!v1.pretty_fits(&d));
    let pretty = save_with(&d, &SaveOptions { pretty: true, ..v1.clone() }).unwrap();
    assert!(pretty.len() > PRETTY_MAX_BYTES, "{} bytes", pretty.len());
}

#[test]
fn art_outside_the_layers_counts_too() {
    // Generated art kept in a symbol: one instance-sized document, a symbol full of anchors.
    let mut d = paths_doc(1, 4);
    let mut art = paths_doc(1, anchors_for(1, 2 * PRETTY_MAX_BYTES));
    let path = Arc::clone(&art.layers[0]);
    art.layers.clear();
    d.symbols.push(Symbol { name: "Leaves".into(), art: path });
    assert!(!SaveOptions::for_doc(&d).pretty);
    let bytes = save_file(&d);
    assert!(!is_pretty(&bytes));
    assert_eq!(load(&bytes).unwrap().symbols.len(), 1);
}
