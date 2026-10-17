//! Clipboard flavours (M4.57): PNG, PDF and plain text out; bitmaps, plain text and PDF in; the
//! Clipboard Handling preferences.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeKind, TextObject};
use vectorcraft_geom::Point;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use super::*;
use crate::cmd::clipboard::{AI, BITMAP, EPS, FILE_HEAD, PASTE_ORDER, PDF, PNG, SVG, TEXT, file_flavour};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A red 40 × 40 pt square, copied.
fn copy_red_square(s: &mut Session) {
    let id = run(s, "shape.rectangle", json!({"x": 10, "y": 20, "width": 40, "height": 40}))["id"].clone();
    run(s, "paint.setFill", json!({"color": "#ff0000"}));
    run(s, "paint.setStroke", json!({"none": true}));
    run(s, "select.set", json!({"ids": [id]}));
    run(s, "edit.copy", json!({}));
}

fn copy_text(s: &mut Session, texts: &[&str]) {
    let ids: Vec<Value> =
        texts.iter().enumerate().map(|(i, t)| run(s, "text.create", json!({"x": 20, "y": 40 + 30 * i, "text": t}))["id"].clone()).collect();
    run(s, "select.set", json!({ "ids": ids }));
    run(s, "edit.copy", json!({}));
}

fn decode(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()
}

/// The pasted objects (the selection after a paste).
fn pasted(s: &Session) -> Vec<vectorcraft_doc::Node> {
    let st = s.doc().unwrap();
    st.selection.objects.iter().map(|id| st.doc.node(*id).unwrap().clone()).collect()
}

fn text_of(n: &vectorcraft_doc::Node) -> &TextObject {
    match &n.kind {
        NodeKind::Text(t) => t,
        k => panic!("not type: {k:?}"),
    }
}

#[test]
fn exported_png_of_a_red_square_is_red() {
    let mut s = session();
    assert!(run(&mut s, "clipboard.exportPng", json!({}))["dataBase64"].is_null(), "nothing copied");
    copy_red_square(&mut s);
    let r = run(&mut s, "clipboard.exportPng", json!({"scale": 2}));
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(80), Some(80)));
    let img = image::load_from_memory(&decode(&r)).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (80, 80));
    assert_eq!(img.get_pixel(40, 40).0, [255, 0, 0, 255]);
    assert_eq!(img.get_pixel(2, 77).0, [255, 0, 0, 255], "cropped to the square");
    for bad in [json!({"scale": 0}), json!({"scale": -1}), json!({"scale": 65})] {
        assert!(s.execute("clipboard.exportPng", &bad).is_err(), "{bad}");
    }
}

#[test]
fn exported_pdf_pastes_back_as_vectors() {
    let mut s = session();
    assert!(run(&mut s, "clipboard.exportPdf", json!({}))["dataBase64"].is_null());
    copy_red_square(&mut s);
    let pdf = run(&mut s, "clipboard.exportPdf", json!({}));
    assert!(decode(&pdf).starts_with(b"%PDF"));
    // Plain PDF: no native document inside (the default PDF preset embeds one).
    let has_native = |b: &[u8]| b.windows(13).any(|w| w == b"/EmbeddedFile");
    assert!(!has_native(&decode(&pdf)));
    let saved = run(&mut s, "document.exportPdf", json!({}));
    assert!(has_native(&decode(&saved)), "the check finds the native document");
    let mut t = session();
    let r = run(&mut t, "clipboard.importPdf", json!({"dataBase64": pdf["dataBase64"], "center": [200, 150]}));
    assert!(r["count"].as_u64().unwrap() >= 1, "{r}");
    run(&mut t, "edit.pasteInPlace", json!({}));
    let st = t.doc().unwrap();
    let b = st.doc.bounds_of(&st.selection.in_paint_order(&st.doc), false).unwrap();
    assert!((b.center().x - 200.0).abs() < 0.5 && (b.center().y - 150.0).abs() < 0.5, "{b:?}");
    assert!((b.width() - 40.0).abs() < 0.5 && (b.height() - 40.0).abs() < 0.5, "{b:?}");
    let mut red = false;
    for n in pasted(&t) {
        n.walk(&mut |c| red |= c.appearance.fill_paint().color().is_some_and(|c| c.to_hex() == "#ff0000"));
    }
    assert!(red, "the square keeps its red fill");
    assert!(t.execute("clipboard.importPdf", &json!({"dataBase64": vectorcraft_format::base64_encode(b"not a pdf")})).is_err());
    assert!(t.execute("clipboard.importPdf", &json!({})).is_err());
}

#[test]
fn import_image_then_paste_gives_an_image() {
    let mut s = session();
    copy_red_square(&mut s);
    let png = run(&mut s, "clipboard.exportPng", json!({}));
    let r = run(&mut s, "clipboard.importImage", json!({"dataBase64": png["dataBase64"], "mime": "image/png", "center": [100, 100]}));
    assert_eq!((r["count"].as_u64(), r["pixelWidth"].as_u64(), r["width"].as_f64()), (Some(1), Some(40), Some(40.0)));
    let r = run(&mut s, "edit.paste", json!({"center": [300, 200]}));
    assert_eq!(r["ids"].as_array().unwrap().len(), 1);
    let n = &pasted(&s)[0];
    let NodeKind::Image(im) = &n.kind else { panic!("not an image: {:?}", n.kind) };
    assert!(im.link.is_none(), "embedded");
    assert!(s.doc().unwrap().doc.images.contains_key(&im.key), "the blob came along");
    let b = n.geometric_bounds().unwrap();
    assert!((b.center().x - 300.0).abs() < 1e-6 && (b.center().y - 200.0).abs() < 1e-6 && (b.width() - 40.0).abs() < 1e-6, "{b:?}");
    // Not a bitmap.
    assert!(s.execute("clipboard.importImage", &json!({"dataBase64": png["dataBase64"], "mime": "image/svg+xml"})).is_err());
    assert!(s.execute("clipboard.importImage", &json!({"dataBase64": vectorcraft_format::base64_encode(b"hello")})).is_err());
    assert!(s.execute("clipboard.importImage", &json!({"dataBase64": "!!"})).is_err());
}

#[test]
fn copied_svgz_files_paste_as_svg() {
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40\" height=\"40\"><rect width=\"40\" height=\"40\" fill=\"#ff0000\"/></svg>";
    let svgz = vectorcraft_svg::compress(svg);
    let of = |name: &str, bytes: &[u8]| file_flavour(name, &bytes[..bytes.len().min(FILE_HEAD)], &PASTE_ORDER);
    assert_eq!(of("art.svgz", &svgz), Some(SVG));
    assert_eq!(of("art.gz", &svgz), Some(SVG), "the content tells");
    assert_eq!(file_flavour("art.svgz", &svgz, &[BITMAP]), None);
    // A file larger than FILE_HEAD: the head is a gzip stream cut short.
    let mut seed = 1u32;
    let mut big = String::from("<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\">");
    for _ in 0..2000 {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        big += &format!("<circle cx=\"{}\" cy=\"{}\" r=\"{}\"/>", seed >> 20, (seed >> 8) & 0xfff, seed & 0xff);
    }
    big += "</svg>";
    // UTF-16 after a byte order mark, as SVG Options' UTF-16 encoding writes it.
    let utf16 =
        |text: &str, unit: fn(u16) -> [u8; 2], bom: [u8; 2]| -> Vec<u8> { bom.into_iter().chain(text.encode_utf16().flat_map(unit)).collect() };
    assert_eq!(of("be.svgz", &vectorcraft_svg::compress_bytes(&utf16(svg, u16::to_be_bytes, [0xfe, 0xff]))), Some(SVG));
    assert_eq!(of("le.svgz", &vectorcraft_svg::compress_bytes(&utf16(svg, u16::to_le_bytes, [0xff, 0xfe]))), Some(SVG));
    // Stored without compression, the head unpacks to fewer than FILE_HEAD bytes (an odd number)
    // before it ends.
    let stored = |bytes: &[u8]| {
        let mut gz = flate2::write::GzEncoder::new(vec![], flate2::Compression::none());
        std::io::Write::write_all(&mut gz, bytes).unwrap();
        gz.finish().unwrap()
    };
    assert_eq!(of("stored.svgz", &stored(big.as_bytes())), Some(SVG));
    assert_eq!(of("stored16.svgz", &stored(&utf16(&big, u16::to_be_bytes, [0xfe, 0xff]))), Some(SVG));
    let big = vectorcraft_svg::compress(&big);
    assert!(big.len() > 2 * FILE_HEAD, "{}", big.len());
    assert_eq!(of("big.svgz", &big), Some(SVG));
    // Other gzip data, whole or cut short, pastes as nothing.
    let notes = vectorcraft_svg::compress("plain text, not SVG");
    assert_eq!(of("notes.txt.gz", &notes), None);
    assert_eq!(of("notes.svgz", &notes), None);
    let mut text = String::new();
    for i in 0..3000 {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        text += &format!("line {i}: {seed}\n");
    }
    let text = vectorcraft_svg::compress(&text);
    assert!(text.len() > FILE_HEAD, "{}", text.len());
    assert_eq!(of("log.gz", &text), None);
    // Cut within the gzip header (10 bytes) or at its end: nothing unpacks.
    for cut in [2, 5, 10] {
        assert_eq!(of("art.svgz", &svgz[..cut]), None, "{cut} bytes");
    }
    // The SVG flavor's command reads the SVGZ bytes: the red square pastes.
    let mut s = session();
    let r = run(&mut s, "clipboard.importSvg", json!({"dataBase64": vectorcraft_format::base64_encode(&svgz), "center": [200, 150]}));
    assert_eq!(r["count"], 1, "{r}");
    run(&mut s, "edit.paste", json!({}));
    let n = &pasted(&s)[0];
    assert_eq!(n.appearance.fill_paint().color().map(|c| c.to_hex()).as_deref(), Some("#ff0000"));
    let b = n.geometric_bounds().unwrap();
    assert!((b.width() - 40.0).abs() < 1e-6 && (b.height() - 40.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn copied_files_paste_by_their_content() {
    let mut s = session();
    copy_red_square(&mut s);
    let png = decode(&run(&mut s, "clipboard.exportPng", json!({})));
    let pdf = decode(&run(&mut s, "clipboard.exportPdf", json!({})));
    let mut jpeg = vec![];
    image::RgbImage::new(2, 2).write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect width=\"4\" height=\"4\"/></svg>";
    let of = |name: &str, bytes: &[u8]| file_flavour(name, &bytes[..bytes.len().min(FILE_HEAD)], &PASTE_ORDER);
    assert_eq!(of("red.png", &png), Some(PNG));
    // The content tells, whatever the name says; bitmaps keep their own type.
    assert_eq!(of("photo.txt", &jpeg), Some("image/jpeg"));
    assert_eq!(of("art.svg", svg), Some(SVG));
    assert_eq!(of("doc.pdf", &pdf), Some(PDF));
    // Files that aren't art paste as nothing (not as their path).
    assert_eq!(of("notes.txt", b"hello"), None);
    assert_eq!(of("archive.zip", b"PK\x03\x04"), None);
    // Only what was asked for.
    assert_eq!(file_flavour("red.png", &png, &[TEXT, SVG]), None);
    assert_eq!(file_flavour("art.svg", svg, &[BITMAP]), None);
}

#[test]
fn copied_ai_files_paste_as_the_pdf_or_postscript_inside() {
    let mut s = session();
    copy_red_square(&mut s);
    let pdf = decode(&run(&mut s, "clipboard.exportPdf", json!({})));
    let ps = b"%!PS-Adobe-3.0\n%%BoundingBox: 0 0 40 40\n%%EndComments\n1 0 0 setrgbcolor\nnewpath 0 0 moveto 40 0 lineto 40 40 lineto closepath fill\nshowpage\n";
    let of = |name: &str, bytes: &[u8]| file_flavour(name, &bytes[..bytes.len().min(FILE_HEAD)], &PASTE_ORDER);
    for name in ["art.ai", "art.ait", "ART.AI"] {
        // Asked for as PDF; Paste reads it from its editing data (tests_aiimport).
        assert_eq!(of(name, &pdf), Some(AI), "{name}");
        assert_eq!(of(name, ps), Some(EPS), "{name}");
    }
    // Neither PDF nor PostScript: nothing to paste, whatever the name says.
    assert_eq!(of("art.ai", b"hello"), None);
    // Only what was asked for.
    assert_eq!(file_flavour("art.ai", &pdf, &[EPS]), None);
    assert_eq!(file_flavour("art.ait", ps, &[PDF]), None);
    // clipboard.importEps, the EPS flavor's command, reads the PostScript.
    let r = run(&mut s, "clipboard.importEps", json!({"dataBase64": vectorcraft_format::base64_encode(ps)}));
    assert_eq!(r["count"], 1, "{r}");
}

#[test]
fn import_text_then_paste_gives_point_text() {
    let mut s = session();
    let r = run(&mut s, "clipboard.importText", json!({"text": "Hello\r\nworld\r\n"}));
    assert_eq!(r["count"], 1);
    run(&mut s, "edit.paste", json!({"center": [200, 150]}));
    let n = &pasted(&s)[0];
    let t = text_of(n);
    assert!(matches!(t.kind, vectorcraft_doc::TextKind::Point));
    assert_eq!(t.plain_text(), "Hello\nworld");
    let b = n.geometric_bounds().unwrap();
    assert!((b.center().x - 200.0).abs() < 1.0 && (b.center().y - 150.0).abs() < 1.0, "{b:?}");
    for bad in [json!({}), json!({"text": " \n\n"}), json!({"text": "x".repeat((1 << 20) + 1)})] {
        assert!(s.execute("clipboard.importText", &bad).is_err());
    }
}

#[test]
fn type_only_copies_offer_their_text() {
    let mut s = session();
    assert!(run(&mut s, "clipboard.exportText", json!({}))["text"].is_null());
    copy_text(&mut s, &["One", "Two"]);
    assert_eq!(run(&mut s, "clipboard.exportText", json!({}))["text"], "One\nTwo");
    // A group of type is type.
    run(&mut s, "object.group", json!({}));
    run(&mut s, "edit.copy", json!({}));
    assert_eq!(run(&mut s, "clipboard.exportText", json!({}))["text"], "One\nTwo");
    // Type with a path is not.
    copy_red_square(&mut s);
    run(&mut s, "select.all", json!({}));
    run(&mut s, "edit.copy", json!({}));
    assert!(run(&mut s, "clipboard.exportText", json!({}))["text"].is_null());
}

#[test]
fn copy_offers_flavours_by_the_preferences() {
    let mut s = session();
    assert!(s.clipboard_flavours().is_empty());
    assert_eq!(run(&mut s, "clipboard.flavours", json!({}))["flavours"], json!([]));
    copy_red_square(&mut s);
    let mimes = |s: &Session| s.clipboard_flavours().iter().map(|f| f.mime).collect::<Vec<_>>();
    // Defaults: Include SVG Code on, PDF off.
    assert_eq!(mimes(&s), [TEXT, SVG, PNG]);
    assert_eq!(run(&mut s, "clipboard.flavours", json!({}))["flavours"], json!([TEXT, SVG, PNG]));
    let f = s.clipboard_flavours();
    assert!(String::from_utf8_lossy(&f[0].data).contains("<svg") && f[0].data == f[1].data);
    assert!(f[2].data.starts_with(b"\x89PNG"));
    run(&mut s, "prefs.set", json!({"key": "copyAsPdf", "value": true}));
    assert_eq!(mimes(&s), [TEXT, SVG, PDF, PNG]);
    assert!(s.clipboard_flavours()[2].data.starts_with(b"%PDF"));
    run(&mut s, "prefs.set", json!({"values": {"copyAsSvg": false, "copyAsPdf": false}}));
    assert_eq!(mimes(&s), [PNG]);
    // Type only: its text is the plain text, even without SVG.
    copy_text(&mut s, &["Words"]);
    assert_eq!(mimes(&s), [TEXT, PNG]);
    assert_eq!(s.clipboard_flavours()[0].data, b"Words");
    run(&mut s, "prefs.set", json!({"key": "copyAsSvg", "value": true}));
    let f = s.clipboard_flavours();
    assert_eq!((f[0].data.as_slice(), f[1].mime), (b"Words".as_slice(), SVG));
}

/// The Type tool pastes the text it copied with its formatting, unless pasted text is kept plain.
fn paste_copied_range(plain: bool) -> f64 {
    let mut s = session();
    let v = ViewInfo::default();
    let id = run(&mut s, "text.create", json!({"x": 100, "y": 100, "text": "Big small", "size": 10}))["id"].clone();
    run(&mut s, "text.setRangeStyle", json!({"id": id, "start": 0, "end": 3, "size": 40}));
    if plain {
        run(&mut s, "prefs.set", json!({"key": "pasteTextFormatting", "value": "plain"}));
    }
    s.select_tool("type", v).unwrap();
    let NodeKind::Text(t) = &s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind else { panic!() };
    let at = t.xf * Point::new(30.0, -3.0);
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, at.x, at.y), v).unwrap();
    }
    s.set_tool_option("select", &json!({"start": 0, "end": 3}));
    let r = run(&mut s, "text.getRange", json!({"id": id, "start": 0, "end": 3}));
    s.set_tool_option("copy", &r["runs"]);
    s.tool_key(ToolKey::End, Mods::default(), v).unwrap();
    s.tool_text("Big", v).unwrap();
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    let n = s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().clone();
    let t = text_of(&n);
    assert_eq!(t.plain_text(), "Big smallBig");
    t.runs.last().unwrap().style.size
}

#[test]
fn keep_plain_text_preference_drops_pasted_formatting() {
    assert_eq!(paste_copied_range(false), 40.0, "Keep Formatting");
    assert_eq!(paste_copied_range(true), 10.0, "Keep Plain Text");
}
