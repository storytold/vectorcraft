//! Preserve Editing: the native document as an embedded file, found again (also under its legacy
//! name) and checked against the pages it was written with.

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node};
use vectorcraft_geom::{Rect, shapes};

use crate::*;

fn doc() -> Document {
    let mut d = Document::new(100.0, 100.0);
    let layer = d.default_layer().unwrap();
    let id = d.alloc_id();
    let n = Node::path(id, shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 50.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    d.insert(Some(layer), 0, n).unwrap();
    d
}

/// The document's PDF with `native` embedded (content streams and the payload uncompressed when
/// `compress` is false).
fn pdf_with(native: Option<&[u8]>, compress: bool) -> ExportReport {
    let mut settings: PdfSettings = serde_json::from_value(json!({"preserveEditing": true})).unwrap();
    settings.compression.compress_text = compress;
    export_with_report(&doc(), &PdfOptions { settings, native: native.map(<[u8]>::to_vec), created: Some(0), ..Default::default() }).unwrap()
}

/// `pdf` with `from` replaced by `to` (the same length, so the cross-reference stays valid).
fn patched(pdf: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    assert_eq!(from.len(), to.len());
    let at = pdf.windows(from.len()).position(|w| w == from).unwrap_or_else(|| panic!("no {:?}", String::from_utf8_lossy(from)));
    let mut out = pdf.to_vec();
    out[at..at + to.len()].copy_from_slice(to);
    out
}

#[test]
fn the_native_document_comes_back_intact() {
    for compress in [true, false] {
        let r = pdf_with(Some(b"{\"native\": true}"), compress);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert!(r.bytes.starts_with(b"%PDF-"));
        let e = editing(&r.bytes).unwrap();
        assert_eq!((e.data.as_slice(), e.intact), (&b"{\"native\": true}"[..], true));
        // The import reports it too, and the pages still import as artwork.
        let imported = import_with_report(&r.bytes, &ImportOptions::default()).unwrap();
        assert_eq!(imported.native, Some(e));
        assert_eq!(imported.document.artboards.len(), 1);
    }
}

#[test]
fn the_native_documents_modification_date_is_the_time_of_writing() {
    // Created in 1970: the information dictionary's dates say so, the embedded file's doesn't.
    let year = |t: Option<i64>| vectorcraft_doc::metadata::civil(t.unwrap())[0];
    let before = year(vectorcraft_doc::metadata::output_time());
    let text = String::from_utf8_lossy(&pdf_with(Some(b"native"), false).bytes).into_owned();
    let after = year(vectorcraft_doc::metadata::output_time());
    assert!(text.contains("/CreationDate(D:1970"), "the file's creation date");
    assert!([before, after].iter().any(|y| text.contains(&format!("/ModDate(D:{y:04}"))), "the embedded file is dated when it is written");
}

#[test]
fn changed_pages_are_noticed() {
    let r = pdf_with(Some(b"native"), false);
    let resized = patched(&r.bytes, b"/MediaBox[0 0 100 100]", b"/MediaBox[0 0 100 101]");
    assert!(!editing(&resized).unwrap().intact, "another page size");
    // Drawing changed elsewhere: the rectangle's first corner moves.
    let moved = patched(&r.bytes, b"10 10 m", b"10 11 m");
    assert!(!editing(&moved).unwrap().intact, "other page content");
    // The description (not part of the hash) can change: re-saving keeps the data usable.
    assert!(editing(&r.bytes).unwrap().intact);
}

/// `pdf` with its editing file's description rewritten by another app as "hash " and the hash it
/// held, in the text string encoding `bom` starts (UTF-16BE or UTF-8), padded with spaces to its
/// length.
fn described(pdf: &[u8], bom: &[u8]) -> Vec<u8> {
    let from = b"(VectorCraft editing data; page hash ";
    let at = pdf.windows(from.len()).position(|w| w == from).unwrap_or_else(|| panic!("no description"));
    let (hash, len) = (&pdf[at + from.len()..at + from.len() + 16], from.len() + 17);
    let mut desc = [b"(", bom].concat();
    for &c in b"hash ".iter().chain(hash) {
        if bom.len() == 2 {
            desc.push(0);
        }
        desc.push(c);
    }
    desc.push(b')');
    desc.resize(len, b' ');
    patched(pdf, &pdf[at..at + len], &desc)
}

#[test]
fn the_hash_is_read_from_a_description_in_any_encoding() {
    let r = pdf_with(Some(b"native"), false);
    let resized = patched(&r.bytes, b"/MediaBox[0 0 100 100]", b"/MediaBox[0 0 100 101]");
    for bom in [&[0xFE, 0xFF][..], &[0xEF, 0xBB, 0xBF]] {
        assert!(editing(&described(&r.bytes, bom)).unwrap().intact, "{bom:?}: the same pages");
        assert!(!editing(&described(&resized, bom)).unwrap().intact, "{bom:?}: another page size");
    }
}

#[test]
fn the_legacy_name_is_read_and_plain_pdfs_have_no_editing_data() {
    let r = pdf_with(Some(b"legacy"), false);
    let name = format!("({EDITING_FILE})");
    let legacy = format!("({LEGACY_EDITING_FILE}){}", " ".repeat(EDITING_FILE.len() - LEGACY_EDITING_FILE.len()));
    let mut old = r.bytes.clone();
    // Every place the name is written (/F, /UF and the name tree key).
    while let Some(at) = old.windows(name.len()).position(|w| w == name.as_bytes()) {
        old[at..at + legacy.len()].copy_from_slice(legacy.as_bytes());
    }
    let e = editing(&old).unwrap();
    assert_eq!((e.data.as_slice(), e.intact), (&b"legacy"[..], true));
    // Off, or with no native document given: nothing embedded (the latter warns).
    let plain = export(&doc(), &PdfOptions::default()).unwrap();
    assert_eq!(editing(&plain), None);
    assert_eq!(import_with_report(&plain, &ImportOptions::default()).unwrap().native, None);
    let r = pdf_with(None, true);
    assert!(r.warnings.len() == 1 && r.warnings[0].contains("Preserve editing"), "{:?}", r.warnings);
    assert_eq!(editing(&r.bytes), None);
}

#[test]
fn pdf_a_carries_no_editing_data() {
    let s: PdfSettings = serde_json::from_value(json!({"standard": "pdfA2b", "preserveEditing": true})).unwrap();
    assert!(matches!(s.check(), Err(PdfError::BadSetting(m)) if m.contains("editing")));
    assert!(matches!(s.check_values(), Err(PdfError::BadSetting(_))));
}

/// A tiny PDF made of the given objects (numbered from 1), with the cross-reference table written.
fn raw_pdf(objects: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for at in offsets {
        out.extend_from_slice(format!("{at:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

#[test]
fn a_name_tree_node_shared_by_many_kids_is_searched_once() {
    // Objects 4..=15 are a chain of name tree nodes, each listing the next one ten times and
    // holding no attachment. Walking every path would take 10^12 steps; the editing file sits in
    // the leaf (object 16) listed after the chain.
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles 3 0 R >> >>".to_string(),
        "<< /Type /Pages /Kids [19 0 R] /Count 1 >>".to_string(),
        "<< /Kids [4 0 R 16 0 R] >>".to_string(),
    ];
    for node in 4..=14 {
        objects.push(format!("<< /Kids [{}] >>", format!("{} 0 R ", node + 1).repeat(10)));
    }
    objects.push("<< /Names [] >>".to_string());
    objects.push("<< /Names [(a) 17 0 R] >>".to_string());
    objects.push(format!("<< /Type /Filespec /UF ({EDITING_FILE}) /EF << /F 18 0 R >> >>"));
    objects.push("<< /Length 5 >>\nstream\nhello\nendstream".to_string());
    objects.push("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>".to_string());
    let e = editing(&raw_pdf(&objects)).unwrap();
    assert_eq!((e.data.as_slice(), e.intact), (&b"hello"[..], true));
}

#[test]
fn direct_name_tree_kids_are_all_searched() {
    // Direct kids report the object holding them as their id; none of them may be taken as seen.
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles 3 0 R >> >>".to_string(),
        "<< /Type /Pages /Kids [6 0 R] /Count 1 >>".to_string(),
        "<< /Kids [<< /Names [] >> << /Kids [<< /Names [] >> << /Names [(a) 4 0 R] >>] >>] >>".to_string(),
        format!("<< /Type /Filespec /UF ({EDITING_FILE}) /EF << /F 5 0 R >> >>"),
        "<< /Length 5 >>\nstream\nhello\nendstream".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>".to_string(),
    ];
    let e = editing(&raw_pdf(&objects)).unwrap();
    assert_eq!(e.data, b"hello");
}

#[test]
fn a_file_name_written_as_utf16_is_found() {
    let utf16: String = EDITING_FILE.bytes().map(|c| format!("00{c:02X}")).collect();
    for key in ["UF", "F"] {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles 3 0 R >> >>".to_string(),
            "<< /Type /Pages /Kids [6 0 R] /Count 1 >>".to_string(),
            "<< /Names [(a) 4 0 R] >>".to_string(),
            format!("<< /Type /Filespec /{key} <FEFF{utf16}> /EF << /F 5 0 R >> >>"),
            "<< /Length 5 >>\nstream\nhello\nendstream".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>".to_string(),
        ];
        let e = editing(&raw_pdf(&objects)).unwrap_or_else(|| panic!("/{key}: not found"));
        assert_eq!(e.data, b"hello", "/{key}");
    }
}
