//! The Import PDF dialog: opening and placing multi-page and password-protected PDFs.

use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::Session;
use vectorcraft_testkit::ai;
use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf, pdf_with};

use super::*;
use crate::io;
use crate::theme;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app
}

/// One headless frame of the dialog layer.
fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    let mut out = ctx.run_ui(Default::default(), |ui| show(app, ui.ctx()));
    out.textures_delta.clear();
}

fn set(app: &mut VectorcraftApp, key: &str, v: serde_json::Value) {
    app.ui.dialog.as_mut().unwrap().fields.insert(key.into(), v);
}

fn field(app: &VectorcraftApp, key: &str) -> serde_json::Value {
    app.ui.dialog.as_ref().unwrap().fields[key].clone()
}

/// Three pages, 100, 200 and 300 pt wide; the second has a 50 × 60 art box.
fn three_pages() -> Vec<u8> {
    let mut second = PdfPage::new(200.0, 200.0, "0 0 1 rg 20 20 40 40 re f");
    second.art = Some([10.0, 10.0, 60.0, 70.0]);
    pdf(&[PdfPage::new(100.0, 100.0, "0 g 0 0 10 10 re f"), second, PdfPage::new(300.0, 100.0, "1 0 0 rg 0 0 30 30 re f")], None)
}

fn artboard_widths(app: &VectorcraftApp) -> Vec<f64> {
    app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect.width()).collect()
}

#[test]
fn opening_a_multi_page_pdf_asks_for_the_pages() {
    let mut app = app();
    io::open_bytes(&mut app, "three.pdf", &three_pages(), Some("/tmp/three.pdf".into())).unwrap();
    assert_eq!(app.session.documents().len(), 1, "nothing opened yet");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.str("mode"), field(&app, "pageCount")), (import_pdf::KIND, "open".into(), json!(3)));
    assert_eq!(field(&app, "cropTo"), "crop");
    frame(&mut app);
    // Page navigation stays within the file.
    set(&mut app, "page", json!(9));
    frame(&mut app);
    assert_eq!(field(&app, "page"), 3);
    // A bad range keeps the dialog open with the message.
    set(&mut app, "allPages", json!(false));
    set(&mut app, "range", json!("2-7"));
    assert!(confirm(&mut app).unwrap_err().contains("1 to 3"));
    assert!(app.ui.dialog.is_some());
    set(&mut app, "range", json!("2-3"));
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(artboard_widths(&app), [200.0, 300.0]);
    assert_eq!(app.views.len(), 2, "views follow the documents");
    // The file is let go once no dialog is open.
    assert!(app.ui.dialog_file.is_some());
    frame(&mut app);
    assert!(app.ui.dialog_file.is_none());
}

#[test]
fn all_pages_and_a_crop_box() {
    let mut app = app();
    io::open_bytes(&mut app, "three.pdf", &three_pages(), None).unwrap();
    set(&mut app, "cropTo", json!("bounding"));
    confirm(&mut app).unwrap();
    assert_eq!(artboard_widths(&app), [10.0, 40.0, 30.0]);
}

#[test]
fn single_page_pdfs_open_directly() {
    let mut app = app();
    io::open_bytes(&mut app, "one.pdf", &pdf(&[PdfPage::new(80.0, 40.0, "")], None), None).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(artboard_widths(&app), [80.0]);
}

#[test]
fn a_password_protected_pdf_asks_for_its_password() {
    let mut app = app();
    let bytes = pdf(&[PdfPage::new(120.0, 60.0, "1 0 0 rg 0 0 20 20 re f")], Some("pw"));
    io::open_bytes(&mut app, "locked.pdf", &bytes, None).unwrap();
    assert_eq!(field(&app, "__locked"), true);
    frame(&mut app);
    set(&mut app, "password", json!("nope"));
    assert!(confirm(&mut app).unwrap_err().contains("wrong"));
    assert_eq!(field(&app, "__locked"), true, "still locked");
    set(&mut app, "password", json!("pw"));
    // One page: unlocking opens it.
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(artboard_widths(&app), [120.0]);
    // Several pages: unlocking shows the pages first.
    let two = pdf(&[PdfPage::new(10.0, 10.0, ""), PdfPage::new(20.0, 10.0, "")], Some("pw"));
    io::open_bytes(&mut app, "two.pdf", &two, None).unwrap();
    set(&mut app, "password", json!("pw"));
    confirm(&mut app).unwrap();
    assert_eq!((field(&app, "__locked"), field(&app, "pageCount")), (json!(false), json!(2)));
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert_eq!(artboard_widths(&app), [10.0, 20.0]);
}

/// The placed object: its kind (clip group or not) and bounds.
fn placed(app: &VectorcraftApp) -> (bool, vectorcraft_geom::Rect) {
    let st = app.session.active().unwrap();
    let n = st.doc.node(st.selection.objects[0]).unwrap();
    let NodeKind::Group { clip, .. } = n.kind else { panic!("{:?}", n.kind) };
    (clip, n.geometric_bounds().unwrap())
}

fn place(app: &mut VectorcraftApp, bytes: &[u8], extra: serde_json::Value) -> Result<serde_json::Value, String> {
    let mut p = json!({"name": "three.pdf", "dataBase64": vectorcraft_format::base64_encode(bytes)});
    if let (Some(o), serde_json::Value::Object(e)) = (p.as_object_mut(), extra) {
        o.extend(e);
    }
    crate::place::run(app, &p)
}

#[test]
fn placing_a_pdf_page_asks_for_the_page_and_crop_box() {
    let mut app = app();
    let bytes = three_pages();
    place(&mut app, &bytes, json!({"at": [100, 120], "link": false})).unwrap();
    assert_eq!((field(&app, "mode"), field(&app, "cropTo")), (json!("place"), json!("crop")));
    assert_eq!(field(&app, "__place"), json!({"at": [100, 120], "link": false}), "the other place params wait in the dialog");
    assert!(app.ui.dialog.as_ref().is_some_and(|d| (spec(&d.kind).heading)(d) == "Place PDF"));
    frame(&mut app);
    set(&mut app, "page", json!(2));
    set(&mut app, "cropTo", json!("art"));
    let clipboard = app.session.clipboard.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let (clip, b) = placed(&app);
    assert!(clip, "clipped to the art box");
    assert_eq!((b.width(), b.height()), (50.0, 60.0));
    assert_eq!(b.center(), vectorcraft_geom::Point::new(100.0, 120.0), "where `at` asked");
    assert_eq!(app.session.clipboard, clipboard, "the clipboard is untouched");
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    // With a page, no dialog; Bounding Box places the art itself.
    place(&mut app, &bytes, json!({"page": 3, "crop": "bounding"})).unwrap();
    assert!(app.ui.dialog.is_none());
    let (clip, b) = placed(&app);
    assert!(!clip);
    assert_eq!((b.width(), b.height()), (30.0, 30.0));
    assert!(place(&mut app, &bytes, json!({"page": 4})).is_err());
    // A one-page PDF places right away.
    place(&mut app, &pdf(&[PdfPage::new(80.0, 40.0, "0 g 0 0 10 10 re f")], None), json!({})).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(placed(&app).1.width(), 80.0);
}

/// A saved `.ai` with several artboards carries its VectorCraft document: opening it restores that
/// document (artboards, layers) instead of asking which pages to import.
#[test]
fn opening_a_saved_ai_with_several_artboards_restores_it_without_asking() {
    let mut app = app();
    app.run("artboard.new", json!({"width": 200, "height": 100, "name": "Second"})).unwrap();
    app.run("layer.new", json!({"name": "Front"})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    let saved = app.run("document.save", json!({"format": "ai"})).unwrap();
    let bytes = vectorcraft_format::base64_decode(saved["dataBase64"].as_str().unwrap()).unwrap();
    assert!(vectorcraft_pdf::info(&bytes, None).unwrap().pages.len() > 1, "a page per artboard");

    io::open_bytes(&mut app, "two.ai", &bytes, Some("/tmp/two.ai".into())).unwrap();
    assert!(app.ui.dialog.is_none(), "no Import PDF dialog");
    assert_eq!(app.session.documents().len(), 2);
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.artboards.len(), 2);
    assert_eq!(doc.artboards[1].name, "Second");
    assert!(doc.layers.iter().any(|l| l.name.as_deref() == Some("Front")), "layers come back");
}

/// A PDF without a VectorCraft document still asks, and placing a saved `.ai` still asks for the page.
#[test]
fn placing_a_saved_ai_still_asks_for_the_page() {
    let mut app = app();
    app.run("artboard.new", json!({"width": 200, "height": 100})).unwrap();
    let saved = app.run("document.save", json!({"format": "ai"})).unwrap();
    let bytes = vectorcraft_format::base64_decode(saved["dataBase64"].as_str().unwrap()).unwrap();
    assert!(import_pdf::offer(&mut app, "two.ai", &bytes, None, Some(&json!({}))));
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("mode"), "place");
}

/// Two pages, 100 and 200 pt wide; with `editing_data` the first page has the keys of an editor's
/// copy of the art.
fn two_pages(editing_data: bool, password: Option<&str>) -> Vec<u8> {
    let mut first = PdfPage::new(100.0, 100.0, "0 g 0 0 10 10 re f");
    if editing_data {
        first.entries = "/PieceInfo << /Illustrator << /Private << /AIPrivateData1 7 /NumBlock 1 >> >> >>".into();
    }
    pdf(&[first, PdfPage::new(200.0, 100.0, "1 0 0 rg 0 0 30 30 re f")], password)
}

/// Two 100 pt pages and, on the first, editing data with two artboards to match and a layer of its
/// own, which draws the square the first page draws.
fn two_pages_with_a_layer_in_the_editing_data(layer: &str) -> Vec<u8> {
    let point = |x: i32, y: i32, n: u8| format!("%_{x} {y} /RealPointRelToROrigin\n%_ (PositionPoint{n}) ,\n");
    let board = |top: i32| format!("%_/Dictionary :\n{}{}%_; ,\n", point(0, top, 1), point(100, top - 100, 2));
    let boards = format!("%_/Document :\n%_/Dictionary :\n%_/Array :\n{}{}%_; (ArtboardArray) ,\n%_; /NotRecorded ,\n%_;\n", board(100), board(-50));
    let art = format!("0 0 0 1 k\n{}f\n", ai::rect(10.0, 10.0, 30.0, 30.0));
    let data = ai::editing_data(100.0, 100.0, &format!("{boards}{}", ai::layer(layer, &art)));
    let mut first = PdfPage::new(100.0, 100.0, "0 0 0 1 k 10 10 20 20 re f");
    first.entries = format!("/PieceInfo << /Illustrator << /Private << /AIPrivateData1 {} 0 R /NumBlock 1 >> >> >>", first_extra(2));
    let stream = format!("<< /Length {} >>\nstream\n{data}\nendstream", data.len());
    pdf_with(&[first, PdfPage::new(100.0, 100.0, "")], &[&stream], None)
}

/// #6135: the editing data of a `.ai` holds every artboard, so opening one asks nothing, opens
/// them all and reads the art from that data: the layer is in nothing else.
#[test]
fn opening_an_ai_with_editing_data_reads_it_without_asking() {
    let mut app = app();
    let bytes = two_pages_with_a_layer_in_the_editing_data("Only in the editing data");
    io::open_bytes(&mut app, "two.ai", &bytes, Some("/tmp/two.ai".into())).unwrap();
    assert!(app.ui.dialog.is_none(), "no Import PDF dialog");
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(artboard_widths(&app), [100.0, 100.0]);
    let layers = &app.session.active().unwrap().doc.layers;
    assert!(layers.iter().any(|l| l.name.as_deref() == Some("Only in the editing data")), "read from the editing data");
    assert!(!import_pdf::offer(&mut app, "two.ait", &bytes, None, None), "a template too");
    assert!(app.ui.dialog.is_none());
}

/// A `.ai` whose editing data can't be read asks nothing either, and every page opens from the PDF
/// part. Asking at that point is #6134.
#[test]
fn opening_an_ai_with_unreadable_editing_data_opens_every_page_without_asking() {
    let mut app = app();
    io::open_bytes(&mut app, "two.ai", &two_pages(true, None), Some("/tmp/two.ai".into())).unwrap();
    assert!(app.ui.dialog.is_none(), "no Import PDF dialog");
    assert_eq!(artboard_widths(&app), [100.0, 200.0]);
}

/// #6135: the dialog stays for what still has something to ask: a PDF, even one with editing data
/// keys, a `.ai` that is only a PDF, a password, and Place.
#[test]
fn the_import_dialog_stays_where_there_is_something_to_ask() {
    for (name, bytes) in [("two.pdf", two_pages(true, None)), ("two.ai", two_pages(false, None))] {
        let mut app = app();
        assert!(import_pdf::offer(&mut app, name, &bytes, None, None), "{name}");
        assert_eq!((app.ui.dialog.as_ref().unwrap().str("mode"), field(&app, "pageCount")), ("open".into(), json!(2)), "{name}");
    }

    let mut app = app();
    assert!(import_pdf::offer(&mut app, "locked.ai", &two_pages(true, Some("pw")), None, None));
    assert_eq!(field(&app, "__locked"), true);

    let mut app = self::app();
    assert!(import_pdf::offer(&mut app, "two.ai", &two_pages(true, None), None, Some(&json!({}))));
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("mode"), "place");
}
