//! `document.open` PDF options `textAs` and `layers`.

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_pdf::TextAs;
use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf, pdf_with, pdf_with_catalog};

use super::*;

fn open(s: &mut Session, bytes: &[u8], extra: Value) -> crate::Result<Value> {
    let mut p = json!({"name": "art.pdf", "dataBase64": vectorcraft_format::base64_encode(bytes)});
    if let (Some(o), Value::Object(e)) = (p.as_object_mut(), extra) {
        o.extend(e);
    }
    s.execute("document.open", &p)
}

fn kinds(s: &Session) -> Vec<&'static str> {
    let mut out = vec![];
    s.active().unwrap().doc.walk(|n| out.push(n.kind_label()));
    out
}

#[test]
fn text_opens_as_type_or_outlines() {
    let page = PdfPage {
        resources: "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >>".into(),
        ..PdfPage::new(200.0, 100.0, "BT /F1 12 Tf 10 50 Td (Hello) Tj ET")
    };
    let bytes = pdf(&[page], None);
    let mut s = Session::new();
    open(&mut s, &bytes, json!({})).unwrap();
    assert!(kinds(&s).contains(&"Type"), "{:?}", kinds(&s));
    let r = open(&mut s, &bytes, json!({"textAs": "outlines"})).unwrap();
    assert!(!kinds(&s).contains(&"Type"));
    assert!(r["warnings"].to_string().contains("outlines"), "{r}");
    let e = open(&mut s, &bytes, json!({"textAs": "glyphs"})).unwrap_err().to_string();
    assert!(e.contains("textAs must be one of text, outlines"), "{e}");
}

fn live_text_pdf() -> Vec<u8> {
    let page = |text: &str| PdfPage {
        resources: "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >>".into(),
        ..PdfPage::new(200.0, 100.0, &format!("BT /F1 12 Tf 10 50 Td ({text}) Tj ET"))
    };
    pdf(&[page("First page"), page("Second page")], None)
}

fn live_texts(s: &Session) -> Vec<String> {
    let mut texts = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            texts.push(t.plain_text());
        }
    });
    texts
}

#[test]
fn pdf_place_keeps_live_text_from_bytes_and_unlinked_paths() {
    let bytes = live_text_pdf();
    let dir = vectorcraft_testkit::temp_dir("pdf-place-live-text");
    let path = dir.join("text.pdf");
    std::fs::write(&path, &bytes).unwrap();
    for source in [json!({"name": "text.pdf", "dataBase64": vectorcraft_format::base64_encode(&bytes)}), json!({"path": path, "link": false})] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let mut p = source;
        p["page"] = json!(2);
        let r = s.execute("file.place", &p).unwrap();
        assert_eq!(live_texts(&s), ["Second page"], "{r}");
        assert!(!r["warnings"].to_string().contains("outlines"), "{r}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pdf_clipboard_paste_keeps_live_text_on_the_selected_page() {
    let bytes = live_text_pdf();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let r = s.execute("clipboard.importPdf", &json!({"dataBase64": vectorcraft_format::base64_encode(&bytes), "page": 2})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(live_texts(&s), ["Second page"], "{r}");
    assert!(!r["warnings"].to_string().contains("outlines"), "{r}");
}

#[test]
fn pdf_place_honors_explicit_text_outlines() {
    let bytes = live_text_pdf();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let r =
        s.execute("file.place", &json!({"name": "text.pdf", "dataBase64": vectorcraft_format::base64_encode(&bytes), "textAs": "outlines"})).unwrap();
    assert!(live_texts(&s).is_empty());
    assert!(r["warnings"].to_string().contains("outlines"), "{r}");
}

#[test]
fn layers_option_keeps_or_flattens_optional_content() {
    let (a, b) = (first_extra(1), first_extra(1) + 1);
    let catalog = format!("/OCProperties << /OCGs [{a} 0 R {b} 0 R] /D << /OFF [{b} 0 R] >> >> ");
    let page = PdfPage {
        resources: format!("/Properties << /A {a} 0 R /B {b} 0 R >>"),
        ..PdfPage::new(100.0, 100.0, "/OC /A BDC 0 g 0 0 10 10 re f EMC /OC /B BDC 0 g 20 20 10 10 re f EMC")
    };
    let bytes = pdf_with_catalog(&[page], &["<< /Type /OCG /Name (Ink) >>", "<< /Type /OCG /Name (Notes) >>"], &catalog, None);
    let mut s = Session::new();
    open(&mut s, &bytes, json!({})).unwrap();
    let layers: Vec<(String, bool)> = s.active().unwrap().doc.layers.iter().map(|l| (l.name.clone().unwrap_or_default(), l.visible)).collect();
    assert_eq!(layers, [("Ink".to_string(), true), ("Notes".to_string(), false)]);
    open(&mut s, &bytes, json!({"layers": false})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 1);
    assert_eq!(doc.layers[0].children().unwrap().len(), 1, "the hidden art is left out");
    assert!(matches!(doc.layers[0].kind, NodeKind::Layer { .. }));
    assert!(open(&mut s, &bytes, json!({"layers": "no"})).unwrap_err().to_string().contains("layers must be true or false"));
}

/// A PDF-compatible `.ai` without the editor's private data opens its layers from its optional
/// content groups, whose names without a byte order mark are PDFDocEncoding.
#[test]
fn ai_layer_names_are_read_as_pdf_doc_encoding() {
    let (a, b) = (first_extra(1), first_extra(1) + 1);
    let catalog = format!("/OCProperties << /OCGs [{a} 0 R {b} 0 R] /D << >> >> ");
    let page = PdfPage {
        resources: format!("/Properties << /A {a} 0 R /B {b} 0 R >>"),
        ..PdfPage::new(100.0, 100.0, "/OC /A BDC 0 g 0 0 10 10 re f EMC /OC /B BDC 0 g 20 20 10 10 re f EMC")
    };
    let bytes = pdf_with_catalog(&[page], &[r"<< /Type /OCG /Name (Art\205final) >>", "<< /Type /OCG /Name (Notes) >>"], &catalog, None);
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "art.ai", "dataBase64": vectorcraft_format::base64_encode(&bytes)})).unwrap();
    let names: Vec<String> = s.active().unwrap().doc.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(names, ["Art\u{2013}final", "Notes"]);
}

#[test]
fn load_options_read_text_and_layers() {
    let o = LoadOptions::from_params("x", &json!({"textAs": "Outlines", "layers": false})).unwrap();
    assert_eq!((o.text_as, o.layers), (TextAs::Outlines, false));
    assert!(o.is_partial(), "without its hidden layers, Save doesn't write the file back");
    let d = LoadOptions::default();
    assert_eq!((d.text_as, d.layers, d.is_partial()), (TextAs::Text, true, false));
}

#[test]
fn exports_that_draw_type_in_a_missing_font_say_it_is_the_fallback_font() {
    let page = PdfPage {
        resources: "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Zyxwvu-Serif >> >>".into(),
        ..PdfPage::new(200.0, 100.0, "BT /F1 12 Tf 10 50 Td (Hello) Tj ET")
    };
    let mut s = Session::new();
    let r = open(&mut s, &pdf(&[page], None), json!({})).unwrap();
    let fonts = s.execute("text.fonts", &json!({})).unwrap();
    let family = fonts[0]["family"].as_str().unwrap().to_string();
    assert_eq!(fonts[0]["status"], "missing", "{fonts}");
    assert!(r["warnings"].to_string().contains(&family), "opening lists it: {r}");
    let warned = |s: &mut Session, p: Value| {
        let w = s.execute("document.serialize", &p).unwrap()["warnings"].to_string();
        w.contains(&family) && w.contains(vectorcraft_text::FALLBACK_FAMILY)
    };
    for p in [
        json!({"format": "pdf"}),
        json!({"format": "pdf", "advanced": {"outlineText": false}}),
        json!({"format": "png"}),
        json!({"format": "eps"}),
        json!({"format": "emf"}),
        json!({"format": "svg", "outlineText": true}),
        json!({"format": "svg", "embedFonts": true}),
    ] {
        assert!(warned(&mut s, p.clone()), "{p}");
    }
    // Live SVG type and the native file name the font itself.
    for p in [json!({"format": "svg", "outlineText": false}), json!({"format": "vectorcraft"}), json!({"format": "txt"})] {
        assert!(!warned(&mut s, p.clone()), "{p}");
    }
    let pdf_export = s.execute("document.exportPdf", &json!({})).unwrap();
    assert!(pdf_export["warnings"].to_string().contains(&family), "{pdf_export}");
    s.execute("text.replaceFont", &json!({"from": {"family": family}, "to": {"family": "Source Serif 4"}})).unwrap();
    assert!(!warned(&mut s, json!({"format": "pdf"})), "replaced: nothing is substituted");
}

/// A page with "Hamburg Zoo" in a font the file carries that no machine has (`Zyxwvu-Serif`: a
/// simple TrueType font in WinAnsiEncoding, its program the bundled Source Serif 4's).
fn carried_font_pdf() -> Vec<u8> {
    let data = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/SourceSerif4-Regular.ttf")).unwrap();
    let face = vectorcraft_text::FontDb::global().face("Source Serif 4", "Regular").unwrap();
    let widths: Vec<String> =
        (32u8..=126).map(|c| format!("{:.0}", face.advance(face.glyph_for(char::from(c))) * 1000.0 / face.units_per_em())).collect();
    let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
    let n = first_extra(1);
    let page = PdfPage { resources: format!("/Font << /F1 {n} 0 R >>"), ..PdfPage::new(300.0, 100.0, "BT /F1 24 Tf 20 50 Td (Hamburg Zoo) Tj ET") };
    let font = format!(
        "<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+Zyxwvu-Serif /FirstChar 32 /LastChar 126 /Widths [{}] /Encoding /WinAnsiEncoding /FontDescriptor {} 0 R >>",
        widths.join(" "),
        n + 1
    );
    let descriptor = format!(
        "<< /Type /FontDescriptor /FontName /ABCDEF+Zyxwvu-Serif /Flags 34 /FontBBox [-500 -300 1500 1000] /ItalicAngle 0 /Ascent 900 /Descent -300 /CapHeight 660 /StemV 80 /FontFile2 {} 0 R >>",
        n + 2
    );
    let file = format!("<< /Length {} /Filter /ASCIIHexDecode >>\nstream\n{hex}>\nendstream", hex.len() + 1);
    pdf_with(&[page], &[&font, &descriptor, &file], None)
}

/// The glyphs of the active document's type (document space origins) and how many come from fonts
/// files carried.
fn glyphs(s: &Session) -> (Vec<kurbo::Point>, usize) {
    let db = vectorcraft_text::FontDb::global();
    let (mut at, mut carried) = (vec![], 0);
    s.doc().unwrap().doc.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            for g in vectorcraft_text::layout(db, t).glyphs {
                at.push(t.xf * g.origin);
                carried += usize::from(db.is_file_font(g.font_id));
            }
        }
    });
    (at, carried)
}

/// Type in a font the PDF carries that isn't installed keeps the file's look until it is edited:
/// no export says it is substituted, it saves and reopens so, and it stays listed as missing. Edited,
/// it takes the substitute, which exports then name.
#[test]
fn type_in_a_font_the_pdf_carries_keeps_its_look_until_edited() {
    let mut s = Session::new();
    let r = open(&mut s, &carried_font_pdf(), json!({})).unwrap();
    let fonts = s.execute("text.fonts", &json!({})).unwrap();
    let family = fonts[0]["family"].as_str().unwrap().to_string();
    assert_eq!(fonts[0]["status"], "missing", "{fonts}");
    assert!(r["warnings"].to_string().contains("until their text is edited"), "{r}");
    let missing = s.execute("text.missingFonts", &json!({})).unwrap();
    assert!(missing["fonts"].to_string().contains(&family), "still missing: {missing}");
    let (at, carried) = glyphs(&s);
    assert_eq!((at.len(), carried), (11, 11), "every glyph the file's");
    let warned = |s: &mut Session| s.execute("document.serialize", &json!({"format": "png"})).unwrap()["warnings"].to_string().contains(&family);
    assert!(!warned(&mut s), "drawn as the file draws it: not substituted");
    // Saved and opened again: the same.
    let native = s.execute("document.serialize", &json!({"format": "vectorcraft"})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    s.execute("document.open", &json!({"name": "kept.vectorcraft", "dataBase64": native})).unwrap();
    assert_eq!(glyphs(&s), (at.clone(), carried));
    // Edited: the substitute, which exports name, and the font still missing.
    let mut id = None;
    s.doc().unwrap().doc.walk(|n| {
        if matches!(n.kind, NodeKind::Text(_)) {
            id = Some(n.id);
        }
    });
    s.execute("text.setText", &json!({"id": id.unwrap(), "text": "Hamburg Zoo!"})).unwrap();
    assert_eq!(glyphs(&s).1, 0, "edited: no glyph from the file's font");
    assert!(warned(&mut s), "edited: substituted");
    assert!(s.execute("text.missingFonts", &json!({})).unwrap()["fonts"].to_string().contains(&family));
    // Undone: the file's look again.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(glyphs(&s), (at, carried));
}
