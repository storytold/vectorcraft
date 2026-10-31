//! The native save options of the file itself: Include Linked Files, embedded ICC profiles and the
//! PDF a PDF-compatible file carries, each round-tripping (and ignored by readers that don't
//! need them).

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::links::LinkInfo;
use vectorcraft_doc::{Appearance, BlendSpec, Document, EnvelopeKind, ImageBlob, ImageObject, LiveShape, Node, NodeKind, OpacityMask, Symbol};
use vectorcraft_format::{
    ICC_MIME, MAX_PRECISION, PRETTY_MAX_BYTES, PRETTY_MAX_OBJECTS, SaveOptions, load, load_file, pdf_content, save_file, save_with,
};
use vectorcraft_geom::path::{Anchor, PathData, SubPath};
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

/// One path at `x, y`, its out handle at `x + 6.1234567, y - 0.00001` (`-0.0` at 4 decimals when
/// `y` is 0), filled with a color whose components have many digits.
fn precise_path(d: &mut Document, x: f64, y: f64) -> Node {
    let p = Point::new(x, y);
    let a = Anchor { p, h_in: p, h_out: Point::new(x + 6.1234567, y - 0.00001), kind: Default::default() };
    let sp = SubPath::new(vec![a, Anchor::corner(Point::new(x + 20.0, y))], false);
    let fill = Paint::solid(Color::rgb(0.02745098, 0.33333334, 0.9019608));
    Node::path(d.alloc_id(), PathData::single(sp), Appearance::basic(fill, Paint::None, 0.0))
}

/// A document with a precise path in the layer, in an opacity mask, in a symbol (placed with a
/// precise transform) and in a brush kept as foreign data.
fn precise_doc() -> Document {
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    let mut n = precise_path(&mut d, 3.4876474632797176, 3.499999999999999);
    n.extras_mut().mask = Some(Box::new(OpacityMask::new(precise_path(&mut d, 7.77777777, 1.0), false)));
    d.insert(Some(layer), 0, n).unwrap();
    let art = precise_path(&mut d, 1.23456789, 0.0);
    d.symbols.push(Symbol { name: "S".into(), art: Arc::new(art) });
    let xf = Affine::new([1.0, 0.0, 0.0, 1.0, 0.123456789, 0.0]);
    let id = d.alloc_id();
    d.insert(Some(layer), 1, Node::new(id, NodeKind::SymbolInstance { symbol: "S".into(), xf })).unwrap();
    let brush = serde_json::to_value(precise_path(&mut d, 5.55555555, 4.0)).unwrap();
    d.unknown.insert("brushes".into(), serde_json::json!([{"name": "B", "type": "art", "art": brush}]));
    d
}

fn first_anchor(n: &Node) -> Anchor {
    let NodeKind::Path { path, .. } = &n.kind else { panic!("not a path: {n:?}") };
    path.subpaths[0].anchors[0]
}

#[test]
fn precision_rounds_path_anchors_and_handles_only() {
    let d = precise_doc();
    let bytes = save_with(&d, &SaveOptions { precision: Some(4), ..SaveOptions::default() }).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(!text.contains("-0.0"), "a handle rounded to zero is written as 0.0");
    let back = load(&bytes).unwrap();
    let path = &back.layers[0].children().unwrap()[0];
    let a = first_anchor(path);
    assert_eq!((a.p, a.h_in, a.h_out), (Point::new(3.4876, 3.5), Point::new(3.4876, 3.5), Point::new(9.6111, 3.5)));
    let mask = path.extras().mask.as_ref().unwrap();
    assert_eq!(first_anchor(&mask.art).p, Point::new(7.7778, 1.0));
    let a = first_anchor(&back.symbols[0].art);
    assert_eq!((a.p, a.h_out), (Point::new(1.2346, 0.0), Point::new(7.358, 0.0)));
    let brush = &back.unknown["brushes"][0]["art"]["kind"]["path"]["subpaths"][0]["anchors"][0];
    assert_eq!((brush["p"][0].as_f64(), brush["out"][0].as_f64()), (Some(5.5556), Some(11.679)));
    // Colors and transforms keep every digit.
    let original = &d.layers[0].children().unwrap()[0];
    assert_eq!(path.appearance, original.appearance);
    assert_eq!(mask.art.appearance, original.extras().mask.as_ref().unwrap().art.appearance);
    let NodeKind::SymbolInstance { xf, .. } = &back.layers[0].children().unwrap()[1].kind else { panic!("not an instance") };
    assert_eq!(xf.as_coeffs()[4], 0.123456789);
    // The document saved is left as it was.
    assert_eq!(first_anchor(&d.layers[0].children().unwrap()[0]).p.x, 3.4876474632797176);
}

#[test]
fn precision_rounds_the_paths_objects_keep_besides_their_own() {
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    let path = || PathData::single(SubPath::polyline(&[Point::new(1.23456789, 0.0), Point::new(9.87654321, 5.0)], false));
    // Live Corners' uncut outline, and an Intertwine area.
    let mut n = precise_path(&mut d, 1.0, 1.0);
    if let NodeKind::Path { live, .. } = &mut n.kind {
        *live = Some(Box::new(LiveShape::Path { base: path(), radii: vec![2.0], kinds: vec![], radii_relative: false }));
    }
    n.extras_mut().intertwine = Some(Box::new(path()));
    d.insert(Some(layer), 0, n).unwrap();
    // A blend's replaced spine and an envelope's top-object outline.
    let keys = vec![Arc::new(precise_path(&mut d, 1.0, 1.0)), Arc::new(precise_path(&mut d, 50.0, 1.0))];
    let blend = BlendSpec { spine: Some(path()), ..BlendSpec::default() };
    let id = d.alloc_id();
    d.insert(Some(layer), 1, Node::new(id, NodeKind::Blend { children: keys, spec: Box::new(blend) })).unwrap();
    let envelope = NodeKind::Envelope {
        content: vec![Arc::new(precise_path(&mut d, 1.0, 1.0))],
        kind: Box::new(EnvelopeKind::TopObject { path: path() }),
        fidelity: 50.0,
        editing: false,
        options: Default::default(),
        frame: Default::default(),
    };
    let id = d.alloc_id();
    d.insert(Some(layer), 2, Node::new(id, envelope)).unwrap();
    // The same three objects as brushes' art, and type, whose frame stays exact.
    let mut brushes: Vec<Value> = d.layers[0].children().unwrap().iter().map(|n| serde_json::json!({"art": n})).collect();
    let frame = serde_json::json!({"subpaths": [{"anchors": [{"p": [1.23456789, 0.0]}], "closed": true}]});
    brushes.push(serde_json::json!({"art": {"id": 99, "kind": {"type": "text", "kind": "area", "frame": frame}}}));
    d.unknown.insert("brushes".into(), Value::Array(brushes));
    let text = |precision| String::from_utf8(save_with(&d, &SaveOptions { precision, ..SaveOptions::default() }).unwrap()).unwrap();
    let exact = text(None);
    assert_eq!((exact.matches("1.23456789").count(), exact.matches("9.87654321").count()), (9, 8), "{exact}");
    let rounded = text(Some(4));
    assert_eq!((rounded.matches("1.23456789").count(), rounded.matches("9.87654321").count()), (1, 0), "{rounded}");
    assert_eq!((rounded.matches("1.2346").count(), rounded.matches("9.8765").count()), (8, 8), "{rounded}");
}

#[test]
fn without_precision_coordinates_are_written_exactly() {
    let d = precise_doc();
    let exact = save_with(&d, &SaveOptions::default()).unwrap();
    let text = String::from_utf8(exact.clone()).unwrap();
    for x in ["3.4876474632797176", "3.499999999999999", "7.77777777", "1.23456789", "5.55555555"] {
        assert!(text.contains(x), "{x}");
    }
    assert_eq!(first_anchor(&load(&exact).unwrap().layers[0].children().unwrap()[0]).p.x, 3.4876474632797176);
    // A precision over MAX_PRECISION rounds to MAX_PRECISION decimals.
    let at = |p| save_with(&d, &SaveOptions { precision: Some(p), ..SaveOptions::default() }).unwrap();
    assert_eq!(at(200), at(MAX_PRECISION));
    assert!(at(0).len() < at(MAX_PRECISION).len());
}
