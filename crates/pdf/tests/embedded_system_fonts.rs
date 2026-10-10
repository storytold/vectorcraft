//! PDF import names type by the installed fonts in a new session (#130). One test in its own
//! process, so the process-wide font database starts fresh, as it does when the app starts.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fmt::Write;

use skrifa::instance::{LocationRef, Size};
use skrifa::raw::TableProvider;
use skrifa::string::StringId;
use skrifa::{FontRef, MetadataProvider};
use vectorcraft_doc::NodeKind;
use vectorcraft_pdf::{ImportOptions, import_with_report};
use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with};
use vectorcraft_text::{FontDb, system_font_dirs};

#[test]
fn a_fresh_session_resolves_pdf_fonts_to_installed_families() {
    let bundled = FontDb::with_font_dirs(vec![]).families();
    let probe = FontDb::with_font_dirs(system_font_dirs());
    probe.load_system_fonts();
    let installed = probe.families();
    // The separate probe supplies the PDF font; the global database stays fresh until import.
    let Some((family, face, postscript_name)) = installed
        .iter()
        .filter(|f| !bundled.contains(f) && f.contains(' ') && f.chars().all(|c| c.is_ascii_alphanumeric() || c == ' '))
        .find_map(|family| {
            let face = probe.face(family, "Regular")?;
            // FontFile2 embeds a standalone TrueType program, not a collection or a named
            // variable instance. WinAnsi's H/i must map to actual glyphs in that program.
            if !face.family.eq_ignore_ascii_case(family)
                || face.face_index() != 0
                || !face.file_data().starts_with(&[0, 1, 0, 0])
                || !face.variations().is_empty()
                || face.italic
                || !face.embeddable()
                || ['H', 'i'].iter().any(|&c| face.glyph_for(c) == 0)
            {
                return None;
            }
            let font = FontRef::from_index(face.file_data(), face.face_index()).ok()?;
            font.head().ok()?;
            font.glyf().ok()?;
            let base_font = font.localized_strings(StringId::POSTSCRIPT_NAME).english_or_first()?.to_string();
            (!base_font.is_empty() && base_font.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
                .then_some((family, face, base_font))
        })
    else {
        eprintln!("no usable installed standalone TrueType face for this PDF fixture: nothing to check");
        return;
    };
    let font = FontRef::from_index(face.file_data(), face.face_index()).unwrap();
    let head = font.head().unwrap();
    let scale = 1000.0 / face.units_per_em();
    let (ascent, descent) = face.vertical_metrics();
    let cap_height = font.metrics(Size::unscaled(), LocationRef::default()).cap_height.map(f64::from).unwrap_or(ascent);
    let flags = 32 | u8::from(font.post().ok().is_some_and(|post| post.is_fixed_pitch() != 0));
    let descriptor_number = first_extra(1);
    // pdf_with accepts string objects; ASCIIHex carries the runtime font bytes losslessly.
    let mut encoded = String::with_capacity(face.file_data().len() * 2 + 1);
    for byte in face.file_data() {
        write!(encoded, "{byte:02X}").unwrap();
    }
    encoded.push('>');
    let font_program =
        format!("<< /Length {} /Length1 {} /Filter /ASCIIHexDecode >>\nstream\n{encoded}\nendstream", encoded.len(), face.file_data().len(),);
    let widths = (b'H'..=b'i').map(|code| format!("{:.6}", face.advance(face.glyph_for(char::from(code))) * scale)).collect::<Vec<_>>().join(" ");
    let packed_family = family.replace(' ', "");
    // Keep the original packed-family boundary first: only this import sees a fresh global DB.
    // Both names use the same embedded glyph program, widths and descriptor metrics.
    for (case, base_font) in [("packed-family", packed_family.as_str()), ("PostScript", postscript_name.as_str())] {
        let descriptor = format!(
            "<< /Type /FontDescriptor /FontName /{base_font} /Flags {flags} /FontBBox [{:.6} {:.6} {:.6} {:.6}] \
             /ItalicAngle 0 /Ascent {:.6} /Descent {:.6} /CapHeight {:.6} /StemV 80 /FontFile2 {} 0 R >>",
            f64::from(head.x_min()) * scale,
            f64::from(head.y_min()) * scale,
            f64::from(head.x_max()) * scale,
            f64::from(head.y_max()) * scale,
            ascent * scale,
            -descent * scale,
            cap_height * scale,
            descriptor_number + 1,
        );
        let resources = format!(
            "/Font << /F1 << /Type /Font /Subtype /TrueType /BaseFont /{base_font} /Encoding /WinAnsiEncoding \
             /FirstChar 72 /LastChar 105 /Widths [{widths}] /FontDescriptor {descriptor_number} 0 R >> >>"
        );
        let page = PdfPage { resources, ..PdfPage::new(100.0, 100.0, "BT /F1 12 Tf 10 30 Td (Hi) Tj ET") };
        let r = import_with_report(&pdf_with(&[page], &[&descriptor, &font_program], None), &ImportOptions::default()).unwrap();
        let mut families = vec![];
        let mut node_types = vec![];
        r.document.walk(|n| {
            let kind = match &n.kind {
                NodeKind::Text(_) => "Text",
                NodeKind::Path { .. } => "Path",
                _ => "Other",
            };
            node_types.push((n.id.0, kind, n.name.clone()));
            if let NodeKind::Text(t) = &n.kind {
                families.extend(t.runs.iter().map(|r| r.style.font_family.clone()));
            }
        });
        let diagnostics = format!(
            "case={case}, selected_family={family:?}, base_font={base_font:?}, postscript_name={postscript_name:?}, warnings={:?}, node_types={node_types:?}",
            r.warnings
        );
        eprintln!("PDF system-font import: {diagnostics}");
        assert_eq!(families, std::slice::from_ref(family), "{diagnostics}");
        assert!(!r.warnings.iter().any(|w| w.contains(family.as_str())), "{:?}", r.warnings);
    }
}
