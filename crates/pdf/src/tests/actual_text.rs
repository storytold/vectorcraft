//! Original, synthetic PDF content; no external fonts needed for semantic coverage.

use crate::{ImportOptions, ImportReport, TextAs, import_with_report};
use vectorcraft_doc::{NodeKind, TextObject};

fn stream(entries: &str, content: &[u8]) -> Vec<u8> {
    let mut bytes = format!("<< /Length {} {entries} >>\nstream\n", content.len()).into_bytes();
    bytes.extend_from_slice(content);
    bytes.extend_from_slice(b"\nendstream");
    bytes
}

fn document(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend(format!("{} 0 obj\n", i + 1).bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).bytes());
    for offset in offsets.iter().skip(1) {
        bytes.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    bytes.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len()).bytes());
    bytes
}

fn pdf(pages: &[&str]) -> Vec<u8> {
    let kids = (0..pages.len()).map(|i| format!("{} 0 R", 4 + i * 2)).collect::<Vec<_>>().join(" ");
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", pages.len()).into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    for (i, content) in pages.iter().enumerate() {
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F 3 0 R >> /Properties << /Named << /ActualText (Named) >> >> >> /Contents {} 0 R >>", 5 + i * 2).into_bytes());
        objects.push(stream("", format!("BT /F 20 Tf 1 0 0 1 20 700 Tm {content} ET").as_bytes()));
    }
    document(&objects)
}

fn open(content: &str) -> ImportReport {
    import_with_report(&pdf(&[content]), &ImportOptions::default()).unwrap()
}

// Geometry assertions need the PDF and layout to use the same widths. Unembedded
// Helvetica is not installed on every platform; missing-font layout fits the overall
// line length, so removing characters changes even an ordinary suffix's position.
fn open_positioned(content: &str) -> ImportReport {
    let data = include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf");
    let face = vectorcraft_text::FontDb::global().find_postscript("SourceSans3-Regular").unwrap();
    assert_eq!(face.file_data(), data);
    let widths = (32u8..=126)
        .map(|c| {
            let glyph = face.glyph_for(char::from(c));
            assert_ne!(glyph, 0);
            (face.advance(glyph) * 1000.0 / face.units_per_em()).to_string()
        })
        .collect::<Vec<_>>()
        .join(" ");
    let font = format!(
        "<< /Type /Font /Subtype /TrueType /BaseFont /SourceSans3-Regular /Encoding /WinAnsiEncoding /FontDescriptor 6 0 R /FirstChar 32 /LastChar 126 /Widths [{widths}] >>"
    );
    let descriptor = b"<< /Type /FontDescriptor /FontName /SourceSans3-Regular /Flags 32 /FontBBox [-500 -300 1500 1000] /ItalicAngle 0 /Ascent 1000 /Descent -300 /CapHeight 700 /StemV 80 /FontFile2 7 0 R >>".to_vec();
    let bytes = custom_pdf(content, font.as_bytes(), "", &[descriptor, stream("", data)]);
    import_with_report(&bytes, &ImportOptions::default()).unwrap()
}

fn texts(report: &ImportReport) -> Vec<TextObject> {
    let mut result = vec![];
    report.document.walk(|node| {
        if let NodeKind::Text(text) = &node.kind {
            result.push((**text).clone());
        }
    });
    result
}

fn plain(report: &ImportReport) -> String {
    texts(report).iter().map(TextObject::plain_text).collect()
}

fn utf16(text: &str) -> String {
    format!("<FEFF{}>", text.encode_utf16().map(|u| format!("{u:04X}")).collect::<String>())
}

fn character_origin(report: &ImportReport, character: char) -> kurbo::Point {
    for text in texts(report) {
        let plain = text.plain_text();
        if let Some(byte) = plain.find(character) {
            let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &text);
            let glyph = layout.glyphs.iter().find(|g| g.byte == byte).unwrap();
            return text.xf * glyph.origin;
        }
    }
    panic!("missing {character}");
}

#[test]
fn actual_text_replaces_once_and_keeps_suffix_position() {
    let replaced = open_positioned("/Span << /ActualText (AB) >> BDC (xy) Tj EMC (C) Tj");
    let ordinary = open_positioned("(xyC) Tj");
    assert_eq!(plain(&replaced), "ABC");
    let delta = character_origin(&replaced, 'C') - character_origin(&ordinary, 'C');
    assert!(delta.hypot() < 0.05, "suffix moved: {delta:?}");
}

#[test]
fn actual_text_empty_and_nested_owners() {
    for (content, expected) in [
        ("(P) Tj /Span << /ActualText () >> BDC (xy) Tj EMC (C) Tj", "PC"),
        ("/Span << /ActualText (AB) >> BDC /Span << /ActualText (INNER) >> BDC (x) Tj EMC (y) Tj EMC (C) Tj", "ABC"),
        ("/Span << /ActualText () >> BDC /Span << /ActualText (INNER) >> BDC (xy) Tj EMC EMC (C) Tj", "C"),
        ("/Span BMC /Span << /ActualText (AB) >> BDC (xy) Tj EMC EMC (C) Tj", "ABC"),
        ("/Span << /ActualText (A) >> BDC (x) Tj EMC /Span << /ActualText (B) >> BDC (y) Tj EMC (C) Tj", "ABC"),
    ] {
        assert_eq!(plain(&open(content)), expected, "{content}");
    }
}

#[test]
fn actual_text_decodes_pdf_strings_and_preserves_authored_order() {
    for (encoded, expected) in [
        (utf16("ل\u{200d}ا\u{200c}ب"), "ل\u{200d}ا\u{200c}ب"),
        ("<8093A0>".into(), "•ﬁ€"),
        ("<EFBBBF4162>".into(), "Ab"),
        ("( A )".into(), " A "),
        ("( )".into(), " "),
        (utf16("A\nB"), "A\nB"),
    ] {
        assert_eq!(plain(&open(&format!("/Span << /ActualText {encoded} >> BDC (xy) Tj EMC"))), expected);
    }
    assert_eq!(plain(&open("/Span /Named BDC (xy) Tj EMC")), "Named");
}

#[test]
fn actual_text_across_operations_styles_and_line_flushes() {
    let report = open_positioned("/Span << /ActualText (AB) >> BDC (x) Tj [(y) -400 (z)] TJ 1 0 0 rg /F 24 Tf (w) Tj EMC (C) Tj");
    assert_eq!(plain(&report), "ABC");
    let report = open_positioned("/Span << /ActualText (AB) >> BDC (x) Tj 0 -40 Td (y) Tj EMC (C) Tj");
    assert_eq!(plain(&report), "ABC");
    let ordinary = open_positioned("(x) Tj 0 -40 Td (yC) Tj");
    assert!((character_origin(&report, 'C') - character_origin(&ordinary, 'C')).hypot() < 0.05);
    // The replacement belongs to the first placed glyph's style and line.
    assert_eq!(texts(&report)[0].plain_text(), "AB");
}

#[test]
fn actual_text_fill_and_stroke_is_not_duplicated() {
    let report = open("2 Tr /Span << /ActualText (AB) >> BDC [(xy) 0 (zw)] TJ EMC (C) Tj");
    assert_eq!(plain(&report), "ABC");
    assert!(texts(&report).iter().flat_map(|t| &t.runs).all(|r| r.style.has_stroke()));
}

#[test]
fn actual_text_page_reset_and_unbalanced_end() {
    let bytes = pdf(&["/Span << /ActualText (AB) >> BDC (xy) Tj", "EMC (C) Tj"]);
    let report = import_with_report(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(plain(&report), "ABC");
}

#[test]
fn actual_text_outlines_are_unchanged() {
    let outline = |content| {
        let report = import_with_report(&pdf(&[content]), &ImportOptions { text_as: TextAs::Outlines, ..Default::default() }).unwrap();
        assert!(texts(&report).is_empty());
        let mut paths = vec![];
        report.document.walk(|n| {
            if let NodeKind::Path { path, .. } = &n.kind {
                paths.push(path.to_bezpath());
            }
        });
        paths
    };
    assert_eq!(outline("/Span << /ActualText (AB) >> BDC (xy) Tj EMC"), outline("(xy) Tj"));
}

#[test]
fn actual_text_bounded_replacements_recover_siblings() {
    let oversized = "A".repeat(64 * 1024 + 1);
    let report = open(&format!("/Span << /ActualText ({oversized}) >> BDC (x) Tj EMC /Span << /ActualText (B) >> BDC (y) Tj EMC"));
    assert_eq!(plain(&report), "xB");
    assert!(report.warnings.iter().any(|w| w.contains("ActualText beyond")));
    let deep = format!(
        "{} /Span << /ActualText (ignored) >> BDC (x) Tj EMC {} /Span << /ActualText (B) >> BDC (y) Tj EMC",
        "/Span BMC ".repeat(128),
        "EMC ".repeat(128)
    );
    assert_eq!(plain(&open(&deep)), "xB");
}

#[test]
fn actual_text_survives_reader_windows() {
    let content = format!("/Span << /ActualText (AB) >> BDC (x) Tj {} (y) Tj EMC (C) Tj", "q Q ".repeat(20_000));
    assert_eq!(plain(&open(&content)), "ABC");
}

fn custom_pdf(content: &str, font: &[u8], resources: &str, extra: &[Vec<u8>]) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>".to_vec(),
        font.to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F 3 0 R >> {resources} >> /Contents 5 0 R >>")
            .into_bytes(),
        stream("", format!("BT /F 20 Tf 1 0 0 1 20 700 Tm {content} ET").as_bytes()),
    ];
    objects.extend_from_slice(extra);
    document(&objects)
}

#[test]
fn actual_text_mixed_line_still_reconstructs_ordinary_rtl() {
    let map = stream("", b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange 4 beginbfchar <78> <05DD> <79> <05D5> <7A> <05DC> <77> <05E9> endbfchar endcmap end end");
    let content = format!("(xyzw) Tj /Span << /ActualText {} >> BDC (xy) Tj EMC (xyzw) Tj", utf16("ل\u{200d}ا"));
    let bytes = custom_pdf(&content, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>", "", &[map]);
    let report = import_with_report(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(texts(&report).len(), 1);
    assert_eq!(plain(&report), "שלוםل\u{200d}اשלום");
}

#[test]
fn actual_text_rescues_a_glyph_without_unicode() {
    // Identity-H with no ToUnicode or embedded font: CID 65535 has no character mapping.
    let font = b"<< /Type /Font /Subtype /Type0 /BaseFont /Unmapped /Encoding /Identity-H /DescendantFonts [6 0 R] >>";
    let descendant = b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Unmapped /CIDSystemInfo << /Registry (Unknown) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 7 0 R /DW 500 >>".to_vec();
    let descriptor = b"<< /Type /FontDescriptor /FontName /Unmapped /Flags 4 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 >>".to_vec();
    let extra = [descendant, descriptor];
    let import = |content| import_with_report(&custom_pdf(content, font, "", &extra), &ImportOptions::default()).unwrap();
    assert!(plain(&import("3 Tr <FFFF> Tj")).is_empty(), "the fixture really has no usable mapping");
    assert_eq!(plain(&import("3 Tr /Span << /ActualText (AB) >> BDC <FFFFFFFF> Tj EMC")), "AB");
}

#[test]
fn actual_text_matches_embedded_glyph_identity_not_replacement() {
    let font = b"<< /Type /Font /Subtype /TrueType /BaseFont /SourceSans3-Regular /Encoding /WinAnsiEncoding /FontDescriptor 6 0 R /FirstChar 120 /LastChar 121 /Widths [500 500] >>";
    let descriptor = b"<< /Type /FontDescriptor /FontName /SourceSans3-Regular /Flags 32 /FontBBox [-500 -300 1500 1000] /ItalicAngle 0 /Ascent 1000 /Descent -300 /CapHeight 700 /StemV 80 /FontFile2 7 0 R >>".to_vec();
    let data = include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf");
    let bytes = custom_pdf("/Span << /ActualText (AB) >> BDC (xy) Tj EMC", font, "", &[descriptor, stream("", data)]);
    let report = import_with_report(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(plain(&report), "AB");
    assert!(!report.warnings.iter().any(|w| w.contains("differ from the installed font")), "{:?}", report.warnings);
}

#[test]
fn actual_text_nested_mask_interpretation_does_not_consume_page_owner() {
    let mask = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 600 800] /Group << /S /Transparency /CS /DeviceGray >> /Resources << /Font << /F 3 0 R >> >>",
        b"/Span << /ActualText (MASK) >> BDC BT /F 20 Tf 1 0 0 1 20 700 Tm (mask) Tj ET EMC 1 g 0 0 600 800 re f",
    );
    let bytes = custom_pdf(
        "/Span << /ActualText (AB) >> BDC /Mask gs (xy) Tj EMC",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        "/ExtGState << /Mask << /SMask << /S /Luminosity /G 6 0 R >> >> >>",
        &[mask],
    );
    let report = import_with_report(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(plain(&report), "AB");
}

#[test]
fn actual_text_empty_span_keeps_following_position_without_slots() {
    for (content, baseline) in [
        ("(P) Tj /Span << /ActualText () >> BDC (xy) Tj EMC (CD) Tj", "(PxyCD) Tj"),
        ("/Span << /ActualText () >> BDC (xy) Tj EMC (CD) Tj", "(xyCD) Tj"),
    ] {
        let report = open_positioned(content);
        let ordinary = open_positioned(baseline);
        assert_eq!(plain(&report), if content.starts_with("(P)") { "PCD" } else { "CD" });
        for c in ['C', 'D'] {
            assert!((character_origin(&report, c) - character_origin(&ordinary, c)).hypot() < 0.05, "{content}: {c}");
        }
    }
}

#[test]
fn actual_text_waits_for_a_placeable_glyph() {
    let report = open("/Span << /ActualText (AB) >> BDC -1 0 0 1 20 700 Tm (x) Tj 1 0 0 1 20 660 Tm (y) Tj EMC (C) Tj");
    assert_eq!(plain(&report), "ABC");
}

#[test]
fn actual_text_stroke_does_not_restyle_ordinary_prefix() {
    let report = open("(P) Tj 2 Tr /Span << /ActualText (AB) >> BDC (xyzw) Tj EMC");
    assert_eq!(plain(&report), "PAB");
    for text in texts(&report) {
        for run in text.runs {
            assert_eq!(run.style.has_stroke(), !run.text.contains('P'), "{:?}", run.text);
        }
    }
}

#[test]
fn actual_text_vertical_column_keeps_replacement_once() {
    let font = b"<< /Type /Font /Subtype /Type0 /BaseFont /Vertical /Encoding /Identity-V /DescendantFonts [6 0 R] /ToUnicode 8 0 R >>";
    let descendant = b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Vertical /CIDSystemInfo << /Registry (Unknown) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 7 0 R /DW 500 /DW2 [880 -1000] >>".to_vec();
    let descriptor = b"<< /Type /FontDescriptor /FontName /Vertical /Flags 4 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 >>".to_vec();
    let map = stream("", b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange 3 beginbfchar <0001> <0078> <0002> <0079> <0003> <0043> endbfchar endcmap end end");
    let extra = [descendant, descriptor, map];
    for (replacement, expected) in [("AB", "ABC"), ("", "C")] {
        let content = format!("/Span << /ActualText ({replacement}) >> BDC <00010002> Tj EMC <0003> Tj");
        let bytes = custom_pdf(&content, font, "", &extra);
        let report = import_with_report(&bytes, &ImportOptions::default()).unwrap();
        assert_eq!(plain(&report), expected);
        assert!(texts(&report).iter().all(|text| text.vertical));
    }
}
