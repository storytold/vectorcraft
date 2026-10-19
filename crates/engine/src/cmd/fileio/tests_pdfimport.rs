//! `document.open` PDF options (pages, Crop To, password) and `document.pdfInfo`.

use serde_json::{Value, json};
use vectorcraft_testkit::pdf::{PdfPage, pdf};

use super::*;

fn b64(bytes: &[u8]) -> String {
    vectorcraft_format::base64_encode(bytes)
}

/// Three pages of widths 100, 200 and 300 pt, the second with every box set.
fn three_pages() -> Vec<u8> {
    let mut second = PdfPage::new(200.0, 400.0, "0 0 1 rg 20 30 40 50 re f");
    second.crop = Some([10.0, 10.0, 190.0, 390.0]);
    second.bleed = Some([5.0, 5.0, 195.0, 395.0]);
    second.trim = Some([15.0, 15.0, 185.0, 385.0]);
    second.art = Some([20.0, 20.0, 120.0, 220.0]);
    pdf(&[PdfPage::new(100.0, 100.0, "0 g 0 0 10 10 re f"), second, PdfPage::new(300.0, 100.0, "")], None)
}

fn open(s: &mut Session, bytes: &[u8], extra: Value) -> crate::Result<Value> {
    let mut p = json!({"name": "pages.pdf", "dataBase64": b64(bytes)});
    if let (Some(o), Value::Object(e)) = (p.as_object_mut(), extra) {
        o.extend(e);
    }
    s.execute("document.open", &p)
}

fn sizes(s: &Session) -> Vec<(f64, f64)> {
    s.active().unwrap().doc.artboards.iter().map(|a| (a.rect.width(), a.rect.height())).collect()
}

#[test]
fn document_open_imports_the_pages_named() {
    let bytes = three_pages();
    let mut s = Session::new();
    open(&mut s, &bytes, json!({"pages": "2-3"})).unwrap();
    assert_eq!(sizes(&s), [(180.0, 380.0), (300.0, 100.0)]);
    let layers: Vec<String> = s.active().unwrap().doc.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(layers, ["Page 2", "Page 3"]);
    open(&mut s, &bytes, json!({"page": 1})).unwrap();
    assert_eq!(sizes(&s), [(100.0, 100.0)]);
    open(&mut s, &bytes, json!({"pages": "all"})).unwrap();
    assert_eq!(sizes(&s).len(), 3);
    let e = open(&mut s, &bytes, json!({"pages": "4"})).unwrap_err().to_string();
    assert!(e.contains("pages") && e.contains("1 to 3"), "{e}");
    assert!(open(&mut s, &bytes, json!({"pages": true})).is_err());
}

#[test]
fn document_open_crops_each_page_to_the_box_asked_for() {
    let bytes = three_pages();
    let mut s = Session::new();
    for (crop, size) in [
        ("media", (200.0, 400.0)),
        ("crop", (180.0, 380.0)),
        ("bleed", (190.0, 390.0)),
        ("trim", (170.0, 370.0)),
        ("art", (100.0, 200.0)),
        ("bounding", (40.0, 50.0)),
    ] {
        open(&mut s, &bytes, json!({"page": 2, "cropTo": crop})).unwrap();
        assert_eq!(sizes(&s), [size], "{crop}");
    }
    // `crop` is an alias (the file.place name); case doesn't matter.
    open(&mut s, &bytes, json!({"page": 2, "crop": "Art"})).unwrap();
    assert_eq!(sizes(&s), [(100.0, 200.0)]);
    let e = open(&mut s, &bytes, json!({"cropTo": "page"})).unwrap_err().to_string();
    assert!(e.contains("bounding, art, crop, trim, bleed, media"), "{e}");
}

#[test]
fn an_encrypted_pdf_needs_its_password() {
    let bytes = pdf(&[PdfPage::new(100.0, 50.0, "1 0 0 rg 0 0 20 20 re f"), PdfPage::new(100.0, 50.0, "")], Some("open sesame"));
    let mut s = Session::new();
    let e = open(&mut s, &bytes, json!({})).unwrap_err().to_string();
    assert!(e.contains("password-protected"), "{e}");
    let e = open(&mut s, &bytes, json!({"password": "nope"})).unwrap_err().to_string();
    assert!(e.contains("password is wrong"), "{e}");
    assert!(s.active().is_none(), "nothing opened");
    let r = open(&mut s, &bytes, json!({"password": "open sesame", "pages": "1"})).unwrap();
    assert_eq!(r["format"], "pdf");
    assert_eq!(sizes(&s), [(100.0, 50.0)]);
    assert_eq!(s.active().unwrap().doc.node_count(), 2, "the layer and its decrypted rectangle");
}

#[test]
fn pdf_info_lists_pages_boxes_and_a_thumbnail() {
    let bytes = three_pages();
    let mut s = Session::new();
    let v = s.execute("document.pdfInfo", &json!({"dataBase64": b64(&bytes), "thumbnail": 2, "thumbnailSize": 100, "cropTo": "art"})).unwrap();
    assert_eq!(v["pages"], 3);
    assert_eq!(v["needsPassword"], false);
    let p2 = &v["pageInfo"][1];
    assert_eq!((p2["width"].as_f64(), p2["height"].as_f64()), (Some(180.0), Some(380.0)));
    assert_eq!(p2["boxes"]["trim"], json!([15.0, 15.0, 185.0, 385.0]));
    assert_eq!(p2["boxes"]["media"], json!([0.0, 0.0, 200.0, 400.0]));
    let png = vectorcraft_format::base64_decode(v["thumbnail"].as_str().unwrap()).unwrap();
    let img = image::load_from_memory(&png).unwrap();
    assert_eq!((img.width(), img.height()), (50, 100), "the art box at 100 px on its longest side");
    assert!(s.execute("document.pdfInfo", &json!({"dataBase64": b64(&bytes), "thumbnail": 4})).is_err());
    // Locked: no pages until the password is given.
    let locked = pdf(&[PdfPage::new(10.0, 10.0, "")], Some("pw"));
    let v = s.execute("document.pdfInfo", &json!({"dataBase64": b64(&locked)})).unwrap();
    assert_eq!((v["pages"].as_u64(), v["needsPassword"].as_bool(), v["wrongPassword"].as_bool()), (Some(0), Some(true), Some(false)));
    let v = s.execute("document.pdfInfo", &json!({"dataBase64": b64(&locked), "password": "x"})).unwrap();
    assert_eq!(v["wrongPassword"], true);
    let v = s.execute("document.pdfInfo", &json!({"dataBase64": b64(&locked), "password": "pw"})).unwrap();
    assert_eq!((v["pages"].as_u64(), v["needsPassword"].as_bool()), (Some(1), Some(false)));
    assert!(s.execute("document.pdfInfo", &json!({"dataBase64": b64(b"not a pdf")})).is_err());
    assert!(s.execute("document.pdfInfo", &json!({})).is_err());
}

#[test]
fn file_place_crops_a_pdf_page_to_any_box_and_takes_its_password() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let place = |s: &mut Session, bytes: &[u8], extra: Value| {
        let mut p = json!({"name": "pages.pdf", "dataBase64": b64(bytes)});
        if let (Some(o), Value::Object(e)) = (p.as_object_mut(), extra) {
            o.extend(e);
        }
        s.execute("file.place", &p)
    };
    let bytes = three_pages();
    for (crop, size) in
        [("trim", (170.0, 370.0)), ("art", (100.0, 200.0)), ("media", (200.0, 400.0)), ("crop", (180.0, 380.0)), ("bounding", (40.0, 50.0))]
    {
        let r = place(&mut s, &bytes, json!({"page": 2, "crop": crop})).unwrap();
        assert_eq!((r["width"].as_f64(), r["height"].as_f64()), (Some(size.0), Some(size.1)), "{crop}");
    }
    let info = s.execute("file.place.info", &json!({"name": "pages.pdf", "dataBase64": b64(&bytes), "page": 2, "crop": "bleed"})).unwrap();
    assert_eq!((info["width"].as_f64(), info["height"].as_f64()), (Some(190.0), Some(390.0)));
    let locked = pdf(&[PdfPage::new(60.0, 30.0, "0 g 0 0 10 10 re f")], Some("pw"));
    assert!(place(&mut s, &locked, json!({})).unwrap_err().to_string().contains("password-protected"));
    assert_eq!(place(&mut s, &locked, json!({"password": "pw"})).unwrap()["width"].as_f64(), Some(60.0));
}

#[test]
fn load_options_read_pages_crop_and_password() {
    let o = LoadOptions::from_params("x", &json!({"page": 3, "crop": "trim", "password": "p"})).unwrap();
    assert_eq!(o, LoadOptions { pages: Some("3".into()), crop: vectorcraft_pdf::CropTo::Trim, password: Some("p".into()), ..Default::default() });
    // `pages` wins over `page`; an empty password is none.
    let o = LoadOptions::from_params("x", &json!({"pages": "1-2", "page": 3, "password": ""})).unwrap();
    assert_eq!((o.pages.as_deref(), o.password), (Some("1-2"), None));
    assert_eq!(LoadOptions::from_params("x", &Value::Null).unwrap(), LoadOptions::default());
}

#[test]
fn a_postscript_ai_file_says_it_cannot_be_opened_yet() {
    // PostScript opens through the EPS reader now (tests_epsimport.rs): a program it can't read,
    // with no preview to fall back on, still says so.
    let ps = b"%!PS-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n0 0 moveto 100 100 frobnicate stroke\nshowpage\n%%EOF\n";
    let mut s = Session::new();
    for name in ["legacy.ai", "art.eps", "named.pdf", "template.ait"] {
        let e = s.execute("document.open", &json!({"name": name, "dataBase64": b64(ps)})).unwrap_err().to_string();
        assert!(e.contains(name) && e.contains("PostScript") && e.contains("save it as PDF"), "{name}: {e}");
    }
    // An EPS with a binary header too, and the PDF info of each. The plain program's PDF info gives
    // its page, and its thumbnail gives the reader's error; the binary file's header points at no
    // PostScript, so its PDF info is an error.
    let mut binary = vec![0xC5, 0xD0, 0xD3, 0xC6];
    binary.extend_from_slice(&[0; 28]);
    binary.extend_from_slice(ps);
    assert!(s.execute("document.open", &json!({"name": "x.eps", "dataBase64": b64(&binary)})).unwrap_err().to_string().contains("PostScript"));
    assert!(s.execute("document.pdfInfo", &json!({"dataBase64": b64(&binary)})).unwrap_err().to_string().contains("PostScript"));
    let v = s.execute("document.pdfInfo", &json!({"dataBase64": b64(ps)})).unwrap();
    assert_eq!((v["pages"].as_u64(), v["pageInfo"][0]["width"].as_f64()), (Some(1), Some(100.0)), "{v}");
    let e = s.execute("document.pdfInfo", &json!({"dataBase64": b64(ps), "thumbnail": 1})).unwrap_err().to_string();
    assert!(e.contains("PostScript") && e.contains("frobnicate"), "{e}");
    assert!(s.active().is_none());
}

/// PostScript given where a PDF is read (`clipboard.importPdf`) is refused with an error that
/// names File › Open and File › Place, and both of those read the file.
#[test]
fn postscript_read_as_a_pdf_names_open_and_place() {
    let ps = b"%!PS-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n1 0 0 setrgbcolor 10 10 50 50 rectfill\nshowpage\n%%EOF\n";
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let e = s.execute("clipboard.importPdf", &json!({"dataBase64": b64(ps)})).unwrap_err().to_string();
    assert!(e.contains("PostScript") && e.contains("not a PDF: File › Open and File › Place read it as EPS"), "{e}");
    assert!(!e.contains("can't be opened"), "{e}");
    for name in ["legacy.ai", "art.eps"] {
        let r = s.execute("document.open", &json!({"name": name, "dataBase64": b64(ps)})).unwrap();
        assert_eq!(r["warnings"], json!([]), "{name}: {r}");
        let r = s.execute("file.place", &json!({"name": name, "dataBase64": b64(ps)})).unwrap();
        assert_eq!(r["ids"].as_array().map(Vec::len), Some(1), "{name}: {r}");
    }
}

/// `document.pdfInfo` reads PostScript (an EPS, a `.ai` in PostScript form, a `.ps` file) as one
/// page, its bounding box or a letter page without one, and its thumbnail shows that page. Each
/// file here opens on that page with `editingData: false`.
#[test]
fn pdf_info_reads_postscript_as_one_page() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    let data = |r: Value| vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
    let eps = data(s.execute("document.exportEps", &json!({"useArtboards": true})).unwrap());
    let ai = data(s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7"})).unwrap());
    assert!(eps.starts_with(&[0xC5, 0xD0, 0xD3, 0xC6]) && ai.starts_with(b"%!PS"));
    let hires = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 101 41\n%%HiResBoundingBox: 0 0 100.5 40.25\n%%EndComments\n0 0 1 setrgbcolor 0 0 50 20 rectfill\n";
    let letter = b"%!PS\n1 0 0 setrgbcolor 72 72 144 72 rectfill\nshowpage\n";
    let bare = b"%!\n0 0 1 setrgbcolor 100 100 50 50 rectfill\nshowpage\n";
    for (name, bytes, size) in [
        ("art.eps", &eps[..], (300.0, 200.0)),
        ("legacy.ai", &ai, (101.0, 51.0)),
        ("hires.eps", hires, (100.5, 40.25)),
        ("page.ps", letter, (612.0, 792.0)),
        ("bare.ps", bare, (612.0, 792.0)),
    ] {
        let v = s.execute("document.pdfInfo", &json!({"dataBase64": b64(bytes), "thumbnail": 1, "thumbnailSize": 100})).unwrap();
        let png = vectorcraft_format::base64_decode(v["thumbnail"].as_str().unwrap()).unwrap();
        let img = image::load_from_memory(&png).unwrap();
        assert_eq!(img.width().max(img.height()), 100, "{name}");
        let mut v = v;
        v["thumbnail"] = json!("png");
        let page = json!({"width": size.0, "height": size.1, "rotation": 0, "boxes": {}});
        assert_eq!(v, json!({"pages": 1, "needsPassword": false, "pageInfo": [page], "thumbnail": "png"}), "{name}");
        s.execute("document.open", &json!({"name": name, "dataBase64": b64(bytes), "editingData": false})).unwrap();
        let board = s.doc().unwrap().doc.artboards[0].rect;
        assert_eq!((board.width(), board.height()), size, "{name}: the page document.open reads");
    }
    let e = s.execute("document.pdfInfo", &json!({"dataBase64": b64(&eps), "thumbnail": 2})).unwrap_err().to_string();
    assert!(e.contains("no page 2"), "{e}");
}

/// `document.pdfInfo`'s `thumbnail` is a page number from 1, for a PDF and for PostScript alike:
/// any other value is an error that names the pages, and an absent or null one gives no thumbnail.
#[test]
fn pdf_info_refuses_a_thumbnail_that_is_not_a_page_number() {
    let ps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 50\n%%EndComments\n0 0 1 setrgbcolor 0 0 50 20 rectfill\n";
    let mut s = Session::new();
    for (bytes, pages) in [(&three_pages()[..], "(the PDF has 3)"), (ps, "(the file has 1)")] {
        for n in [json!(-1), json!(0), json!(1.5), json!("1"), json!(true)] {
            let e = s.execute("document.pdfInfo", &json!({"dataBase64": b64(bytes), "thumbnail": n})).unwrap_err().to_string();
            assert!(e.contains(&format!("thumbnail: no page {n} {pages}")), "{n}: {e}");
        }
        for none in [json!({"dataBase64": b64(bytes)}), json!({"dataBase64": b64(bytes), "thumbnail": null})] {
            assert!(s.execute("document.pdfInfo", &none).unwrap().get("thumbnail").is_none(), "{pages}");
        }
        let v = s.execute("document.pdfInfo", &json!({"dataBase64": b64(bytes), "thumbnail": 1, "thumbnailSize": 64})).unwrap();
        let png = vectorcraft_format::base64_decode(v["thumbnail"].as_str().unwrap()).unwrap();
        let img = image::load_from_memory(&png).unwrap();
        assert_eq!(img.width().max(img.height()), 64, "{pages}");
    }
}

/// A page showing only text, with an editor's private data when `private`.
fn text_page(private: bool, also: &str) -> PdfPage {
    PdfPage {
        resources: "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >>".into(),
        entries: if private { "/PieceInfo << /Editor << /Private << /AIPrivateData1 7 /RoundTripVersion 24 >> >> >>".into() } else { String::new() },
        ..PdfPage::new(300.0, 100.0, &format!("BT /F1 12 Tf 10 50 Td (Saved without PDF content) Tj ET {also}"))
    }
}

#[test]
fn a_placeholder_only_pdf_part_is_detected() {
    let mut s = Session::new();
    let e = open(&mut s, &pdf(&[text_page(true, "")], None), json!({})).unwrap_err().to_string();
    assert!(e.contains("placeholder page") && e.contains("PDF compatibility"), "{e}");
    assert!(s.active().is_none());
    // Text without private data, and private data with art, are ordinary files.
    open(&mut s, &pdf(&[text_page(false, "")], None), json!({})).unwrap();
    open(&mut s, &pdf(&[text_page(true, "0 g 0 0 10 10 re f")], None), json!({})).unwrap();
    // One real page among the pages picked is enough.
    let two = pdf(&[text_page(true, ""), text_page(true, "0 g 0 0 10 10 re f")], None);
    assert!(open(&mut s, &two, json!({"pages": "1"})).is_err());
    open(&mut s, &two, json!({})).unwrap();
}
