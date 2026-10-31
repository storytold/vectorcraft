//! Synthetic page-content regressions for the shared reader's windowed input.

use std::io::Write;

use crate::{ImportOptions, ImportReport, import_with_report};
use vectorcraft_doc::NodeKind;

fn stream(entries: &str, bytes: &[u8]) -> Vec<u8> {
    let mut out = format!("<< /Length {} {entries} >>\nstream\n", bytes.len()).into_bytes();
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\nendstream");
    out
}

/// A classic xref keeps the fixture's content-stream policy independent of xref decoding.
fn pdf(contents: &[Vec<u8>], resources: &str, catalog: &str, extra: &[Vec<u8>]) -> Vec<u8> {
    let refs = (0..contents.len()).map(|i| format!("{} 0 R", i + 4)).collect::<Vec<_>>().join(" ");
    let mut objects = vec![
        format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>").into_bytes(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << {resources} >> /Contents [{refs}] >>").into_bytes(),
    ];
    objects.extend_from_slice(contents);
    objects.extend_from_slice(extra);
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).bytes());
        out.extend_from_slice(object);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).bytes());
    for offset in offsets.iter().skip(1) {
        out.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len()).bytes());
    out
}

fn open(bytes: &[u8]) -> ImportReport {
    import_with_report(bytes, &ImportOptions::default()).unwrap()
}

fn fills(report: &ImportReport) -> Vec<bool> {
    let mut fills = vec![];
    report.document.walk(|n| {
        if matches!(n.kind, NodeKind::Path { .. }) {
            fills.extend(n.appearance.fill().map(|f| f.overprint));
        }
    });
    fills
}

fn assert_incomplete(bytes: &[u8]) {
    let error = import_with_report(bytes, &ImportOptions::default()).expect_err("known incomplete content must fail");
    assert!(error.to_string().contains("no document was imported"), "{error}");
}

#[test]
fn content_windows_preserve_layers_and_saved_overprint_state() {
    let mut content = b"/OC /Layer BDC q /On gs ".to_vec();
    // Force both the marked-content and graphics states to cross several 64 KiB windows.
    content.extend("q Q ".repeat(40_000).bytes());
    content.extend_from_slice(b"10 10 20 20 re f Q 40 10 20 20 re f EMC");
    let report = open(&pdf(
        &[stream("", &content)],
        "/Properties << /Layer 5 0 R >> /ExtGState << /On << /OP true /op true /OPM 1 >> >>",
        "/OCProperties << /OCGs [5 0 R] /D << /Order [5 0 R] >> >>",
        &[b"<< /Type /OCG /Name (Across windows) >>".to_vec()],
    ));
    assert_eq!(report.document.layers.len(), 1);
    assert_eq!(report.document.layers[0].name.as_deref(), Some("Across windows"));
    assert_eq!(fills(&report), [true, false]);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn long_operands_and_contents_array_boundaries_are_read_whole() {
    let mut first = b"/Span << /ActualText (".to_vec();
    first.extend(std::iter::repeat_n(b'a', 96_000));
    first.extend_from_slice(b") >> BDC 10 10");
    let report = open(&pdf(&[stream("", &first), stream("", b" 20 20 re f EMC\0 % trailing comment")], "", "", &[]));
    assert_eq!(fills(&report), [false]);
}

#[test]
fn inline_image_crossing_a_content_window_preserves_the_next_path() {
    let mut content = b"q 10 0 0 10 0 0 cm BI /W 300 /H 100 /CS /RGB /BPC 8 ID ".to_vec();
    content.extend(std::iter::repeat_n(127, 90_000));
    content.extend_from_slice(b" EI Q 20 20 30 30 re f");
    let report = open(&pdf(&[stream("", &content)], "", "", &[]));
    let mut images = 0;
    report.document.walk(|n| images += usize::from(matches!(n.kind, NodeKind::Image(_))));
    assert_eq!(images, 1);
    assert_eq!(fills(&report), [false]);
}

#[test]
fn undecidable_final_operands_are_rejected() {
    for tail in [b" 10 20".as_slice(), b" (unfinished", b" [1 2", b" << /Key"] {
        let mut content = b"10 10 20 20 re f".to_vec();
        content.extend_from_slice(tail);
        assert_incomplete(&pdf(&[stream("", &content)], "", "", &[]));
    }
}

#[test]
fn incomplete_instruction_window_limit_is_an_error() {
    let mut content = b"/Span << /ActualText (".to_vec();
    content.extend(std::iter::repeat_n(b'a', 4096));
    content.extend_from_slice(b") >> BDC 10 10 20 20 re f EMC");
    let bytes = pdf(&[stream("", &content)], "", "", &[]);
    assert_eq!(fills(&open(&bytes)), [false]);
    let old = crate::import_scan::TEST_MAX_WINDOW.replace(1024);
    // The override belongs to this thread; the public entry point uses a dedicated stack.
    let result = crate::import::read(&bytes, &ImportOptions::default());
    crate::import_scan::TEST_MAX_WINDOW.set(old);
    assert!(result.unwrap_err().to_string().contains("no document was imported"));
}

#[test]
fn failed_content_and_form_decoders_are_errors() {
    assert_incomplete(&pdf(&[stream("/Filter /ASCIIHexDecode", b"ZZ>")], "", "", &[]));
    assert_incomplete(&pdf(
        &[stream("", b"/Fm Do")],
        "/XObject << /Fm 5 0 R >>",
        "",
        &[stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Filter /ASCIIHexDecode", b"ZZ>")],
    ));
}

#[test]
fn truncated_flate_does_not_return_a_partial_document() {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(b"10 10 20 20 re f ").unwrap();
    let mut compressed = encoder.finish().unwrap();
    compressed.truncate(compressed.len() - 6);
    assert_incomplete(&pdf(&[stream("/Filter /FlateDecode", &compressed)], "", "", &[]));
}

#[test]
fn lower_document_decode_limit_rejects_run_length_prefixes() {
    let content = b"10 10 20 20 re f 40 40 20 20 re f";
    let mut encoded = vec![(content.len() - 1) as u8];
    encoded.extend_from_slice(content);
    encoded.push(128);
    for is_form in [false, true] {
        let bytes = if is_form {
            pdf(
                &[stream("", b"/Fm Do")],
                "/XObject << /Fm 5 0 R >>",
                "",
                &[stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Filter /RunLengthDecode", &encoded)],
            )
        } else {
            pdf(&[stream("/Filter /RunLengthDecode", &encoded)], "", "", &[])
        };
        assert_eq!(fills(&open(&bytes)), [false, false]);
        let mut options = crate::pages::open_options();
        options.decode_limits = options.decode_limits.with_max_decoded_stream(16);
        let error = crate::import::import_with_reader_options(&bytes, &ImportOptions::default(), &options).unwrap_err();
        assert!(error.to_string().contains("no document was imported"), "is_form={is_form}: {error}");
    }
}
#[test]
fn lowered_materialization_limit_rejects_incomplete_nested_content() {
    let content = format!("/Span << /A {}0{} >> BDC 10 10 20 20 re f EMC", "[".repeat(300), "]".repeat(300));
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(content.as_bytes()).unwrap();
    let bytes = pdf(&[stream("/Filter /FlateDecode", &encoder.finish().unwrap())], "", "", &[]);
    let mut options = crate::pages::open_options();
    options.decode_limits = options.decode_limits.with_max_decoded_stream(16);
    let error = crate::import::import_with_reader_options(&bytes, &ImportOptions::default(), &options)
        .expect_err("bounded operand parsing must reject incomplete streamed content");
    assert!(
        matches!(error, crate::PdfError::Parse(ref message) if message == "PDF content is incomplete or exceeds the reader's limits; no document was imported"),
        "{error}"
    );
}

#[test]
fn decoded_dictionary_delimiter_split_at_content_window_is_checked() {
    let mut content = vec![b' '; (64 << 10) - 1];
    content.extend(format!("{}0{} DP", "<< /A ".repeat(300), ">> ".repeat(300)).bytes());
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&content).unwrap();
    let bytes = pdf(&[stream("/Filter /FlateDecode", &encoder.finish().unwrap())], "", "", &[]);
    let error = import_with_report(&bytes, &ImportOptions::default()).unwrap_err();
    assert!(
        matches!(error, crate::PdfError::Parse(ref message) if message == "PDF content is incomplete or exceeds the reader's limits; no document was imported"),
        "{error}"
    );
}
