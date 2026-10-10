//! Illustrator EPS and `.ai` files open from the editing data they carry: layers, groups, names,
//! hidden objects, artboards, colours and images as they were; what the reader doesn't read yet
//! opens as before, with a warning saying why.

use std::io::Write as _;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_testkit::ai;

use super::*;

fn open(s: &mut Session, name: &str, bytes: &[u8], params: Value) -> Result<Value> {
    let mut p = json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)});
    if let (Some(p), Some(extra)) = (p.as_object_mut(), params.as_object()) {
        p.extend(extra.clone());
    }
    s.execute("document.open", &p)
}

/// The `.ai` private data of `data`, compressed as older `.ai` files compress it.
fn compressed(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    [&b"%AI12_CompressedData"[..], &e.finish().unwrap()].concat()
}

fn names(nodes: &[Arc<Node>]) -> Vec<String> {
    nodes.iter().map(|n| n.name.clone().unwrap_or_default()).collect()
}

/// The sample's structure, as [`ai::sample_data`] builds it.
fn check_sample(s: &Session) {
    let doc = &s.doc().unwrap().doc;
    assert_eq!(names(&doc.layers), ["Art", "Hidden"]);
    assert_eq!(doc.artboards.len(), 1);
    assert_eq!(doc.artboards[0].rect, vectorcraft_geom::Rect::new(0.0, 0.0, 200.0, 100.0));
    let art = doc.layers[0].children().unwrap();
    assert_eq!(names(art), ["Pair", "Ring", "Window"]);
    let pair = art[0].children().unwrap();
    assert_eq!(names(pair), ["Red", ""]);
    assert!(pair[0].visible && !pair[1].visible, "the hidden member stays hidden");
    assert!(matches!(art[1].kind, NodeKind::Compound { .. }));
    assert!(matches!(&art[2].kind, NodeKind::Group { clip: true, children } if matches!(children[0].kind, NodeKind::Path { clipping: true, .. })));
    let hidden = &doc.layers[1];
    assert!(!hidden.visible);
    let kids = hidden.children().unwrap();
    assert_eq!(names(kids), ["Shaded", "", "Sub", ""]);
    assert!(matches!(kids[0].appearance.fill().map(|f| &f.paint), Some(vectorcraft_color::Paint::Gradient(_))));
    assert_eq!((kids[1].opacity, kids[1].blend), (0.5, vectorcraft_color::BlendMode::Screen));
    assert!(kids[2].locked && matches!(kids[2].kind, NodeKind::Layer { .. }));
    let NodeKind::Image(im) = &kids[3].kind else { panic!("an image") };
    assert_eq!((im.width, im.height), (2, 1));
}

#[test]
fn an_illustrator_eps_opens_with_its_layers() {
    let mut s = Session::new();
    let r = open(&mut s, "art.eps", &ai::eps(&ai::sample_data(), ai::page_ps()), json!({})).unwrap();
    assert_eq!((&r["format"], &r["warnings"]), (&json!("eps"), &json!([])), "{r}");
    check_sample(&s);
    // Saved as EPS and opened again, it comes back the same.
    let eps = s.execute("document.exportEps", &json!({})).unwrap();
    let bytes = vectorcraft_format::base64_decode(eps["dataBase64"].as_str().unwrap()).unwrap();
    let r = open(&mut s, "again.eps", &bytes, json!({})).unwrap();
    assert_eq!(r["restored"], json!(true), "{r}");
    check_sample(&s);
}

#[test]
fn an_illustrator_ai_file_opens_from_its_editing_data() {
    let mut s = Session::new();
    let file = ai::ai(&compressed(&ai::sample_data()), ai::page_pdf());
    let r = open(&mut s, "art.ai", &file, json!({})).unwrap();
    assert_eq!(r["format"], json!("ai"), "{r}");
    assert!(r["warnings"].as_array().unwrap().iter().all(|w| !w.as_str().unwrap().contains("weren't read")), "{r}");
    check_sample(&s);
    // Pages picked: the PDF's.
    open(&mut s, "art.ai", &file, json!({"pages": "1"})).unwrap();
    assert_ne!(names(&s.doc().unwrap().doc.layers), ["Art", "Hidden"]);
}

#[test]
fn what_the_reader_doesnt_read_opens_as_before_with_a_warning() {
    let symbols = ai::editing_data(200.0, 100.0, &ai::layer("Art", "/SymbolInstance :\n(Dot) /SymbolRef ,\n;\n"));
    let mut s = Session::new();
    let r = open(&mut s, "symbols.eps", &ai::eps(symbols.as_bytes(), ai::page_ps()), json!({})).unwrap();
    assert!(r["warnings"].to_string().contains("layers weren't read") && r["warnings"].to_string().contains("symbols"), "{r}");
    assert_eq!(names(&s.doc().unwrap().doc.layers), ["Layer 1"]);
    let r = open(&mut s, "symbols.ai", &ai::ai(&compressed(symbols.as_bytes()), ai::page_pdf()), json!({})).unwrap();
    assert!(r["warnings"].to_string().contains("symbols") && r["warnings"].to_string().contains("PDF part"), "{r}");
}

/// A page showing only a placeholder line, as a `.ai` saved without PDF compatibility has.
const PLACEHOLDER: &str = "BT /F1 12 Tf 10 50 Td (Saved without its PDF part) Tj ET";

#[test]
fn an_ai_file_saved_without_its_pdf_part_opens_from_its_editing_data() {
    let mut s = Session::new();
    let r = open(&mut s, "art.ai", &ai::ai(&compressed(&ai::sample_data()), PLACEHOLDER), json!({})).unwrap();
    assert_eq!(r["format"], json!("ai"), "{r}");
    check_sample(&s);
    // Its editing data damaged: it says so (the placeholder isn't the art).
    let e = open(&mut s, "damaged.ai", &ai::ai(b"%AI12_CompressedData not zlib", PLACEHOLDER), json!({})).unwrap_err().to_string();
    assert!(e.contains("without PDF compatibility") && e.contains("zlib"), "{e}");
    // Without editing data, the placeholder still isn't opened as the art.
    let e = open(&mut s, "plain.ai", &ai::ai(b"", PLACEHOLDER), json!({})).unwrap_err().to_string();
    assert!(e.contains("placeholder"), "{e}");
}

#[test]
fn editing_data_false_opens_only_what_the_page_prints() {
    let mut s = Session::new();
    for (name, file) in [("art.eps", ai::eps(&ai::sample_data(), ai::page_ps())), ("art.ai", ai::ai(&compressed(&ai::sample_data()), ai::page_pdf()))]
    {
        let r = open(&mut s, name, &file, json!({"editingData": false})).unwrap();
        let layers = &s.doc().unwrap().doc.layers;
        assert_eq!(layers.len(), 1, "{name}: {r}");
        assert_ne!(names(layers), ["Art"], "{name}: the page's layer");
    }
    let e = open(&mut s, "art.eps", &ai::eps(&ai::sample_data(), ai::page_ps()), json!({"editingData": "no"})).unwrap_err().to_string();
    assert!(e.contains("editingData must be true or false"), "{e}");
    // A file that is only its editing data can't open without it.
    let e = open(&mut s, "art.ai", &ai::ai(&compressed(&ai::sample_data()), PLACEHOLDER), json!({"editingData": false})).unwrap_err().to_string();
    assert!(e.contains("placeholder"), "{e}");
}

#[test]
fn fit_to_artwork_bounds_leaves_out_the_hidden_art_and_guides_the_editing_data_has() {
    // A shown square; a hidden one and a long guide far from it, which the page doesn't draw.
    let art =
        format!("0 g\n{}f\n1 Xw\n{}f\n0 Xw\n-500 20 m\n700 20 L\n(N) *\n", ai::rect(10.0, 10.0, 50.0, 50.0), ai::rect(150.0, 60.0, 190.0, 95.0));
    let data = ai::editing_data(200.0, 100.0, &ai::layer("Art", &art));
    let mut s = Session::new();
    let r = open(&mut s, "fit.eps", &ai::eps(data.as_bytes(), "0 setgray 10 10 40 40 rectfill\n"), json!({})).unwrap();
    let layer = &s.doc().unwrap().doc.layers[0];
    let kids = layer.children().unwrap();
    assert_eq!(kids.len(), 3, "{r}");
    assert!(!kids[1].visible && matches!(kids[2].kind, NodeKind::Path { guide: true, .. }));
    s.execute("artboard.fitToArt", &json!({})).unwrap();
    // The square, 50 to 90 down the 100-point page.
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect, vectorcraft_geom::Rect::new(10.0, 50.0, 50.0, 90.0));
}

#[test]
fn type_asked_for_as_outlines_opens_a_file_whose_type_shows_from_its_pdf_part() {
    let text = "/AI11Text :\n0 /FreeUndo ,\n0 /FrameIndex ,\n0 /StoryIndex ,\n;\n";
    let data = ai::editing_data(200.0, 100.0, &ai::layer("Art", &format!("0 g\n{}f\n{text}", ai::rect(10.0, 10.0, 50.0, 50.0))));
    let file = ai::ai(&compressed(data.as_bytes()), "0 g 10 10 40 40 re f BT /F1 12 Tf 100 50 Td (Hi) Tj ET");
    let mut s = Session::new();
    // As type, the page's type goes into the text object of the layers.
    let r = open(&mut s, "type.ai", &file, json!({})).unwrap();
    assert_eq!(names(&s.doc().unwrap().doc.layers), ["Art"], "{r}");
    // As outlines, only the PDF part has them.
    let r = open(&mut s, "type.ai", &file, json!({"textAs": "outlines"})).unwrap();
    assert!(r["warnings"].to_string().contains("outlines"), "{r}");
    assert_ne!(names(&s.doc().unwrap().doc.layers), ["Art"], "{r}");
}
