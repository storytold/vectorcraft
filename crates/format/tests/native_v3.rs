//! `.vectorcraft` v3: compression, pruning unused images, keys from newer versions, colour
//! profiles as a document field, saving for older versions and the embedded preview.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write as _;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{ColorProfiles, Composer, Document, ImageBlob, ImageObject, Node, NodeKind, OpacityMask, Symbol};
use vectorcraft_format::{FormatError, SaveOptions, VERSION, is_compressed, load, preview, save, save_with, sniff};
use vectorcraft_geom::Affine;
use vectorcraft_testkit::fixtures;
use vectorcraft_testkit::invariants::{check_native_roundtrip, doc_json, json_approx_eq};

fn rich_doc() -> Document {
    (*fixtures::rich_session().doc().unwrap().doc).clone()
}

fn json_of(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(bytes).unwrap();
    gz.finish().unwrap()
}

fn blob(bytes: &[u8]) -> ImageBlob {
    ImageBlob::new("image/png", bytes.to_vec())
}

fn image_node(d: &mut Document, key: &str) -> Node {
    let id = d.alloc_id();
    Node::new(
        id,
        NodeKind::Image(ImageObject { key: key.into(), width: 2, height: 2, xf: Affine::IDENTITY, link: None, placement: Default::default() }),
    )
}

fn opts(version: u32, compress: bool) -> SaveOptions {
    SaveOptions { version, compress, ..SaveOptions::default() }
}

#[test]
fn area_type_vertical_alignment_round_trips() {
    let d = rich_doc();
    let text = String::from_utf8(save(&d, false)).unwrap();
    assert!(text.contains("\"verticalAlign\":\"center\""), "the fixture's area type is centred");
    let back = load(text.as_bytes()).unwrap();
    assert_eq!(doc_json(&back), doc_json(&d));
    let area = |d: &Document| {
        let mut found = None;
        d.walk(|n| {
            // The fixture's centred area type, by its text (the fixture has other area type).
            if let NodeKind::Text(t) = &n.kind
                && matches!(t.kind, vectorcraft_doc::TextKind::Area { .. })
                && t.runs.iter().map(|r| r.text.as_str()).collect::<String>() == "Centred area type"
            {
                found = Some(t.area.vertical_align);
            }
        });
        found.unwrap()
    };
    assert_eq!(area(&back), vectorcraft_doc::VerticalAlign::Center);
    // Files from before the option open top-aligned.
    // (Every area type the fixture has writes its options, so the key goes from all of them.)
    fn strip(v: &mut Value) {
        match v {
            Value::Object(m) => {
                m.remove("verticalAlign");
                m.values_mut().for_each(strip);
            }
            Value::Array(a) => a.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut v = json_of(text.as_bytes());
    strip(&mut v);
    let old = serde_json::to_string(&v).unwrap();
    assert!(!old.contains("verticalAlign"));
    assert_eq!(area(&load(old.as_bytes()).unwrap()), vectorcraft_doc::VerticalAlign::Top);
}

#[test]
fn compressed_files_load() {
    let d = rich_doc();
    let packed = save_with(&d, &SaveOptions { compress: true, pretty: true, ..SaveOptions::default() }).unwrap();
    assert!(is_compressed(&packed) && sniff(&packed));
    let plain = save(&d, false);
    assert!(packed.len() * 3 < plain.len(), "compressed {} vs {} bytes", packed.len(), plain.len());
    assert_eq!(doc_json(&load(&packed).unwrap()), doc_json(&load(&plain).unwrap()));
    // Compressed files are compact even when pretty was asked for.
    assert!(!flate2_unpack(&packed).contains(&b'\n'));
    // A legacy (v2, pre-rename) file compressed by hand opens too.
    let text = String::from_utf8(plain.clone()).unwrap();
    let legacy = text.replacen(&format!("\"format\":\"vectorcraft\",\"version\":{VERSION}"), "\"format\":\"drawcraft\",\"version\":2", 1);
    assert_ne!(legacy, text);
    let legacy = gzip(legacy.as_bytes());
    assert!(sniff(&legacy));
    assert_eq!(doc_json(&load(&legacy).unwrap()), doc_json(&load(&plain).unwrap()));
    // Other gzip data (an SVGZ) is not a native file.
    assert!(!sniff(&gzip(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>")));
    assert!(load(&packed[..packed.len() / 2]).is_err(), "a truncated file is an error");
}

fn flate2_unpack(bytes: &[u8]) -> Vec<u8> {
    use std::io::Read as _;
    let mut out = vec![];
    flate2::read::GzDecoder::new(bytes).read_to_end(&mut out).unwrap();
    out
}

#[test]
fn unused_images_are_pruned() {
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    for (k, b) in [("used", &[1u8, 2][..]), ("unused", &[3, 4]), ("in symbol", &[5]), ("in brush", &[6]), ("in mask", &[7])] {
        d.images.insert(k.into(), blob(b));
    }
    let mut n = image_node(&mut d, "used");
    n.mask = Some(Box::new(OpacityMask::new(image_node(&mut d, "in mask"), false)));
    d.insert(Some(layer), 0, n).unwrap();
    let art = image_node(&mut d, "in symbol");
    d.symbols.push(Symbol { name: "S".into(), art: Arc::new(art) });
    // Image objects inside foreign data (a brush library kept as JSON) keep theirs too.
    d.unknown.insert("brushes".into(), json!([{"art": {"type": "image", "key": "in brush"}}]));
    let back = load(&save(&d, true)).unwrap();
    let mut keys: Vec<&str> = back.images.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["in brush", "in mask", "in symbol", "used"]);
    assert_eq!(back.images["used"].bytes.as_slice(), &[1, 2]);
    let v = json_of(&save(&d, false));
    assert!(v["images"].get("unused").is_none() && v["document"]["images"].as_object().is_some_and(|m| m.is_empty()));
}

#[test]
fn keys_from_newer_versions_round_trip() {
    let mut v = json_of(&save(&rich_doc(), false));
    v["document"]["futureFeature"] = json!({"a": [1, 2, 3], "b": "x"});
    v["document"]["futureFlag"] = json!(true);
    let d = load(&serde_json::to_vec(&v).unwrap()).unwrap();
    assert_eq!(d.extra["futureFeature"]["a"], json!([1, 2, 3]));
    assert!(!d.unknown.contains_key("futureFeature"), "kept apart from `unknown`");
    for pretty in [false, true] {
        let again = json_of(&save(&d, pretty));
        assert_eq!(again["document"]["futureFeature"], v["document"]["futureFeature"]);
        assert_eq!(again["document"]["futureFlag"], json!(true));
    }
    check_native_roundtrip(&d).unwrap();
}

/// The paragraph composer is saved only when it isn't the default (Every-line) and reads back.
#[test]
fn text_composer_round_trips() {
    let d = rich_doc();
    let composers = |d: &Document| {
        let mut v = vec![];
        d.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                v.push(t.para.composer);
            }
        });
        v
    };
    // The point type set to Single-line comes first; the fixture's other type keeps the default.
    let single_then_default = |c: &[Composer]| c.len() > 1 && c[0] == Composer::SingleLine && c[1..].iter().all(|c| *c == Composer::EveryLine);
    assert!(single_then_default(&composers(&d)), "{:?}", composers(&d));
    let bytes = save(&d, false);
    assert!(String::from_utf8_lossy(&bytes).contains("\"composer\":\"singleLine\""));
    assert_eq!(composers(&load(&bytes).unwrap()), composers(&d));
    // Files without the key (older files, Every-line text) read as Every-line.
    fn strip(v: &mut Value) {
        match v {
            Value::Object(m) => {
                m.remove("composer");
                m.values_mut().for_each(strip);
            }
            Value::Array(a) => a.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut v = json_of(&bytes);
    strip(&mut v);
    let d2 = load(&serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(composers(&d2).iter().all(|c| *c == Composer::EveryLine), "{:?}", composers(&d2));
    assert!(!String::from_utf8_lossy(&save(&d2, false)).contains("composer"));
}

/// A document as the first version wrote it (format name `drawcraft`, anchors as maps).
const V1_FILE: &str = r#"{"format":"drawcraft","version":1,"generator":"DrawCraft 0.1.0","document":{"version":1,"title":"Old","artboards":[{"id":1,"name":"Artboard 1","rect":{"x0":0.0,"y0":0.0,"x1":100.0,"y1":80.0}}],
"layers":[{"id":1,"name":"Layer 1","visible":true,"locked":false,"opacity":1.0,"kind":{"type":"layer","children":[
{"id":2,"visible":true,"locked":false,"opacity":1.0,"kind":{"type":"path","path":{"subpaths":[{"anchors":[
{"p":{"x":10.0,"y":10.0},"in":{"x":10.0,"y":10.0},"out":{"x":30.0,"y":0.0},"kind":"Smooth"},
{"p":{"x":50.0,"y":40.0},"in":{"x":50.0,"y":40.0},"out":{"x":50.0,"y":40.0},"kind":"Corner"}],"closed":false}]},"rule":"NonZero","clipping":false,"guide":false}}],"color":{"Preset":0}}}],
"next_id":3,"unknown":{"colorProfiles":{"rgb":"sRGB IEC61966-2.1","cmyk":null}}},"images":{}}"#;

#[test]
fn version_1_files_load() {
    let d = load(V1_FILE.as_bytes()).unwrap();
    assert_eq!(d.title, "Old");
    let NodeKind::Path { path, .. } = &d.layers[0].children().unwrap()[0].kind else { panic!("not a path") };
    let a = &path.subpaths[0].anchors;
    assert_eq!((a[0].h_out.x, a[0].h_out.y, a[1].p.x), (30.0, 0.0, 50.0));
    // Profiles migrate out of `unknown`.
    assert_eq!(d.color_profiles, ColorProfiles { rgb: Some("sRGB IEC61966-2.1".into()), cmyk: None });
    assert!(d.unknown.is_empty());
}

#[test]
fn saves_down_to_older_versions() {
    let mut d = rich_doc();
    d.color_profiles = ColorProfiles { rgb: None, cmyk: Some("Coated".into()) };
    // What versions before compound shapes can hold: the document as v4 writes it (compound
    // shapes as plain art).
    assert!(d.has_compound_shapes(), "the fixture has a compound shape");
    let current = doc_json(&load(&save_with(&d, &opts(4, false)).unwrap()).unwrap());
    for version in [1, 2] {
        let bytes = save_with(&d, &opts(version, false)).unwrap();
        let v = json_of(&bytes);
        assert_eq!((v["format"].as_str(), v["version"].as_u64()), (Some("drawcraft"), Some(version as u64)), "older apps know the old name");
        // Profiles where those versions keep them.
        assert_eq!(v["document"]["unknown"]["colorProfiles"], json!({"rgb": null, "cmyk": "Coated"}));
        assert!(v["document"].get("color_profiles").is_none());
        let back = load(&bytes).unwrap();
        assert!(json_approx_eq(&doc_json(&back), &current, 1e-12), "v{version} loses nothing it can hold");
        let text = String::from_utf8(bytes).unwrap();
        // v1 anchors are maps with every member; v2 anchors are compact.
        assert_eq!(text.contains("\"p\":{\"x\""), version == 1);
        assert_eq!(text.contains("\"p\":["), version == 2);
    }
    let v3 = json_of(&save(&d, false));
    assert_eq!((v3["format"].as_str(), v3["version"].as_u64()), (Some("vectorcraft"), Some(VERSION as u64)));
    assert_eq!(v3["document"]["color_profiles"], json!({"cmyk": "Coated"}));
    assert!(v3["document"]["unknown"].get("colorProfiles").is_none());
    // Out of range, and compression older apps can't read.
    assert!(matches!(save_with(&d, &opts(0, false)), Err(FormatError::BadVersion(0))));
    assert!(matches!(save_with(&d, &opts(VERSION + 1, false)), Err(FormatError::BadVersion(_))));
    assert!(matches!(save_with(&d, &opts(2, true)), Err(FormatError::CompressedTooOld(2))));
}

#[test]
fn preview_is_embedded_and_read_back() {
    let d = Document::new(10.0, 10.0);
    assert_eq!(preview(&save(&d, false)), None);
    let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3];
    for compress in [false, true] {
        let bytes = save_with(&d, &SaveOptions { compress, preview: Some(png.clone()), ..SaveOptions::default() }).unwrap();
        assert_eq!(preview(&bytes), Some(png.clone()));
        assert!(load(&bytes).is_ok());
    }
    assert_eq!(preview(b"not a file"), None);
}

#[test]
fn pretty_files_are_indented_like_serde_json() {
    let d = rich_doc();
    let bytes = save(&d, true);
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.starts_with("{\n  \"format\": \"vectorcraft\",\n  \"version\": "));
    assert!(text.contains("\n  \"document\": {\n    \""));
    assert!(text.ends_with("\n}"));
    // The same JSON as the compact file.
    assert_eq!(json_of(&bytes), json_of(&save(&d, false)));
}

/// #451: a guide's artboard is written only for artboard guides, so canvas guides save as they
/// did, and a file without it (an older version's) opens with canvas guides.
#[test]
fn artboard_guides_round_trip_and_canvas_guides_save_as_before() {
    use vectorcraft_doc::Guide;
    let mut d = Document::new(100.0, 100.0);
    let id = d.artboards[0].id;
    d.guides = vec![Guide::new(true, 10.0), Guide { artboard: Some(id), ..Guide::new(false, 20.0) }];
    let mut v = json_of(&save(&d, false));
    assert_eq!(v["document"]["guides"], json!([{"vertical": true, "pos": 10.0}, {"vertical": false, "pos": 20.0, "artboard": id}]));
    assert_eq!(load(&save(&d, false)).unwrap().guides, d.guides);
    v["document"]["guides"] = json!([{"vertical": false, "pos": 20.0}]);
    let old = load(&serde_json::to_vec(&v).unwrap()).unwrap();
    assert_eq!(old.guides, [Guide::new(false, 20.0)]);
}

#[test]
fn inline_graphics_in_text_round_trip() {
    let d = rich_doc();
    let inline = |d: &Document| {
        let mut found = vec![];
        d.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                found.extend(t.runs.iter().filter_map(|r| r.inline.clone().map(|a| (r.text.clone(), a.symbol, a.scale, a.baseline_shift))));
            }
        });
        found
    };
    let want = inline(&d);
    assert_eq!(want, vec![("\u{FFFC}".to_string(), "Dot".to_string(), 1.0, 0.0)]);
    let bytes = save(&d, false);
    let saved = json_of(&bytes).to_string();
    assert!(saved.contains("\"inline\":{") && saved.contains("\"symbol\":\"Dot\""), "{saved}");
    let back = load(&bytes).unwrap();
    assert_eq!(inline(&back), want);
    assert_eq!(doc_json(&back), doc_json(&d));
    check_native_roundtrip(&d).unwrap();
}
