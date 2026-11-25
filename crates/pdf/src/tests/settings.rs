//! Save PDF settings as the writer uses them: the checks run before an export, and the writer adds
//! its own notes on options. The model, its JSON and its rules are craft-pdf's.

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node};
use vectorcraft_geom::{Rect, shapes};

use crate::*;

fn settings(v: serde_json::Value) -> PdfSettings {
    serde_json::from_value(v).unwrap()
}

fn doc() -> Document {
    let mut d = Document::new(100.0, 100.0);
    let layer = d.default_layer().unwrap();
    let id = d.alloc_id();
    let n = Node::path(id, shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 50.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    d.insert(Some(layer), 0, n).unwrap();
    d
}

#[test]
fn export_refuses_settings_its_checks_refuse() {
    // PDF/X-1a is a PDF 1.3 standard: the default 1.7 is refused, 1.4 is accepted.
    let x1a = |v: serde_json::Value| export(&doc(), &PdfOptions { settings: settings(v), ..Default::default() });
    assert!(matches!(x1a(json!({"standard": "pdfX1a"})), Err(PdfError::BadSetting(_))));
    assert!(x1a(json!({"standard": "pdfX1a", "compatibility": "1.4"})).is_ok());
}

#[test]
fn the_writer_notes_thumbnails_it_was_not_given() {
    let r =
        export_with_report(&doc(), &PdfOptions { settings: settings(json!({"thumbnails": true, "marks": {"trim": true}})), ..Default::default() })
            .unwrap();
    // The writer embeds the thumbnails it is given: without them, the one note on the options says so.
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert!(r.setting_notes.len() == 1 && r.setting_notes[0].contains("thumbnails need the pages drawn"), "{:?}", r.setting_notes);
    assert!(r.bytes.starts_with(b"%PDF-1.7"));
}
