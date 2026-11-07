//! Compatibility PDF 1.3: which standards allow it, the header and metadata that say so, the
//! writer refusing transparency (the app flattens it first) and 40-bit RC4 encryption.

use serde_json::json;
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node};
use vectorcraft_geom::{Rect, shapes};

use crate::*;

/// A document with a black rectangle; `opacity` below 1 makes it transparent.
fn doc(opacity: f32) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let l = d.default_layer().unwrap();
    let mut n = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 50.0)),
        Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
    );
    n.opacity = opacity;
    d.insert(Some(l), 0, n).unwrap();
    d
}

fn options(v: serde_json::Value) -> PdfOptions {
    PdfOptions { settings: serde_json::from_value(v).unwrap(), created: Some(0), ..PdfOptions::uncompressed() }
}

#[test]
fn which_standards_allow_pdf_1_3() {
    use Compatibility::*;
    assert!(Standard::None.allows(Pdf13) && Standard::PdfX1a.allows(Pdf13) && Standard::PdfX3.allows(Pdf13));
    assert!(!Standard::PdfA2b.allows(Pdf13) && !Standard::PdfX4.allows(Pdf13));
    // PDF/X-1a and PDF/X-3 files are PDF 1.3; PDF 1.4 is still accepted, and written as 1.3.
    assert_eq!((Standard::PdfX1a.version(), Standard::PdfX3.version()), (Pdf13, Pdf13));
    assert!(Standard::PdfX1a.allows(Pdf14));
    let s = |v| serde_json::from_value::<PdfSettings>(v).unwrap();
    assert!(s(json!({"compatibility": "1.3"})).pdf13() && s(json!({"standard": "pdfX3", "compatibility": "1.4"})).pdf13());
    assert!(!s(json!({"compatibility": "1.4"})).pdf13() && !s(json!({"standard": "pdfX4"})).pdf13());
    assert!(matches!(s(json!({"standard": "pdfA2b", "compatibility": "1.3"})).check(), Err(PdfError::BadSetting(m)) if m.contains("PDF 1.3")));
    assert_eq!(Compatibility::IDS.first(), Some(&"1.3"), "first in the dropdown");
    assert_eq!(Encryption::for_compatibility(Pdf13), Encryption::Rc440);
}

#[test]
fn pdf_1_3_files_say_so_and_have_no_transparency() {
    let mut d = doc(1.0);
    // Isolated page blending is a transparency group: a flat file has none.
    d.page_isolate = true;
    let r = export_with_report(&d, &options(json!({"compatibility": "1.3"}))).unwrap();
    let text = String::from_utf8_lossy(&r.bytes);
    assert!(r.bytes.starts_with(b"%PDF-1.3") && text.contains("<pdf:PDFVersion>1.3<"), "the header and the metadata");
    assert!(!text.contains("/Transparency") && !text.contains("/SMask"));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    // The same at 1.4 has its page group.
    assert!(String::from_utf8_lossy(&export(&d, &options(json!({"compatibility": "1.4"}))).unwrap()).contains("/S/Transparency"));
    // Transparency the app didn't flatten is refused.
    let e = export(&doc(0.5), &options(json!({"compatibility": "1.3"}))).unwrap_err();
    assert!(matches!(&e, PdfError::Unsupported(m) if m.contains("PDF 1.3 files have no transparency")), "{e}");
    let mut d = doc(1.0);
    if let Some(n) = d.layers[0].children().and_then(|c| c.first()).map(|n| n.id) {
        d.node_mut(n).unwrap().blend = BlendMode::Multiply;
    }
    assert!(matches!(export(&d, &options(json!({"compatibility": "1.3"}))), Err(PdfError::Unsupported(m)) if m.contains("PDF 1.3")));
}

#[test]
fn pdf_1_3_files_are_encrypted_with_40_bit_rc4() {
    let o = options(json!({"compatibility": "1.3", "security": {"openPassword": "open", "permissionsPassword": "owner", "plaintextMetadata": true}}));
    let r = export_with_report(&doc(1.0), &o).unwrap();
    assert!(String::from_utf8_lossy(&r.bytes).contains("/Filter/Standard/V 1/R 2/Length 40/P "));
    assert!(r.warnings.iter().any(|w| w.contains("PDF 1.3 encryption covers the metadata too")), "{:?}", r.warnings);
    assert!(matches!(import(&r.bytes), Err(PdfError::NeedsPassword)));
    for password in ["open", "owner"] {
        let opened = import_with_report(&r.bytes, &ImportOptions { password: Some(password.into()), ..Default::default() }).unwrap();
        assert_eq!(opened.document.layers[0].children().map_or(0, |c| c.len()), 1, "{password}");
    }
    assert_eq!(crate::encrypt::user_password(&r.bytes, "owner").as_deref(), Some("open"));
}
