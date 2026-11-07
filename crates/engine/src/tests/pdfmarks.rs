//! `document.exportPdf` Marks and Bleeds: the document's bleed or custom values set the page
//! boxes, printer's marks print in Registration, non-printing layers are left out unless asked.

use serde_json::{Value, json};
use vectorcraft_pdf::CropTo;

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80, "bleed": 9})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
    s
}

/// The PDF `document.exportPdf {p}` writes, and its warnings.
fn export(s: &mut Session, p: Value) -> (Vec<u8>, Vec<String>) {
    let v = s.execute("document.exportPdf", &p).unwrap_or_else(|e| panic!("{p}: {e}"));
    let bytes = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
    (bytes, serde_json::from_value(v["warnings"].clone()).unwrap())
}

fn boxes(bytes: &[u8]) -> (kurbo::Rect, kurbo::Rect, kurbo::Rect) {
    let page = &vectorcraft_pdf::info(bytes, None).unwrap().pages[0];
    let get = |c: CropTo| page.boxes.iter().find(|(b, _)| *b == c).unwrap().1;
    (get(CropTo::Media), get(CropTo::Bleed), get(CropTo::Trim))
}

#[test]
fn the_document_bleed_or_custom_values_grow_each_page() {
    let mut s = session();
    let (bytes, warnings) = export(&mut s, json!({"bleed": {"useDocument": true}}));
    assert!(warnings.is_empty(), "{warnings:?}");
    let (media, bleed, trim) = boxes(&bytes);
    assert_eq!((media.width(), media.height()), (118.0, 98.0), "MediaBox = artboard + the document's 9 pt bleed");
    assert_eq!((bleed, trim), (media, kurbo::Rect::new(9.0, 9.0, 109.0, 89.0)));
    // Press Quality uses the document's bleed; the default preset and custom values don't.
    assert_eq!(boxes(&export(&mut s, json!({"preset": "Press Quality"})).0).0.width(), 118.0);
    assert_eq!(boxes(&export(&mut s, json!({})).0).0.width(), 100.0);
    assert_eq!(boxes(&export(&mut s, json!({"bleed": {"left": 4.5}})).0).0.width(), 104.5);
}

#[test]
fn printer_marks_print_in_registration_with_no_warning() {
    let mut s = session();
    let marks = json!({"trim": true, "registration": true, "colorBars": true, "pageInfo": true, "kind": "japanese"});
    let (bytes, warnings) = export(&mut s, json!({"marks": marks, "bleed": {"useDocument": true}}));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(String::from_utf8_lossy(&bytes).contains("/Separation/All"));
    let (media, bleed, _) = boxes(&bytes);
    // Japanese marks start at the 9 pt bleed and reach 18 pt (+ their weight) beyond it.
    assert_eq!((bleed.width(), media.width()), (118.0, 100.0 + 2.0 * (9.0 + 18.0 + 0.25)), "the sheet holds the marks: {media:?}");
    let v = s.execute("document.pdfSettings", &json!({"marks": marks, "bleed": {"top": 9}, "includeDocument": true})).unwrap();
    assert_eq!(v["warnings"], json!([]));
}

#[test]
fn non_printing_layers_are_left_out_unless_included() {
    let mut s = session();
    let layer = s.execute("layer.new", &json!({"name": "Notes"})).unwrap()["id"].as_u64().unwrap();
    s.execute("layer.setProps", &json!({"id": layer, "printable": false})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 60, "y": 40, "width": 20, "height": 20})).unwrap();
    let paths = |s: &mut Session, p| {
        let doc = vectorcraft_pdf::import(&export(s, p).0).unwrap();
        let mut n = 0;
        doc.walk(|x| n += usize::from(x.path_data().is_some()));
        n
    };
    let without = paths(&mut s, json!({}));
    assert_eq!(paths(&mut s, json!({"includeNonPrinting": true})), without + 1, "the ellipse on the non-printing layer");
    s.execute("layer.setProps", &json!({"id": layer, "printable": true})).unwrap();
    assert_eq!(paths(&mut s, json!({})), without + 1);
}
