//! Current-main import contracts across the shared reader's older Device API.

use vectorcraft_color::Paint;
use vectorcraft_doc::{Document, NodeKind};

use crate::{ImportOptions, TextAs, import_with_report};

const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");

fn stream(dictionary: &str, content: &[u8]) -> Vec<u8> {
    let mut bytes = format!("<< {dictionary} /Length {} >>\nstream\n", content.len()).into_bytes();
    bytes.extend_from_slice(content);
    bytes.extend_from_slice(b"\nendstream");
    bytes
}

fn assemble(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![];
    for (i, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let start = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer << /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n", objects.len() + 1).as_bytes());
    bytes
}

fn page(resources: &str, content: &[u8], extra: &[Vec<u8>]) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 60] /Contents 4 0 R /Resources << {resources} >> >>").into_bytes(),
        stream("", content),
    ];
    objects.extend_from_slice(extra);
    assemble(&objects)
}

pub(crate) fn resource_scope_cases() -> Vec<(&'static str, Vec<u8>, bool)> {
    [
        ("absent", "", true),
        ("empty", "/Resources << >>", false),
        ("unrelated", "/Resources << /ProcSet [/PDF] >>", false),
        ("own-off", "/Resources << /ExtGState << /On << /OP false /op false /OPM 1 >> >> >>", false),
        ("own-on", "/Resources << /ExtGState << /On << /OP true /op true /OPM 1 >> >> >>", true),
    ]
    .into_iter()
    .map(|(name, scope, first)| {
        let form = stream(&format!("/Type /XObject /Subtype /Form /BBox [0 0 100 60] {scope}"), b"/On gs 10 10 20 20 re f");
        let bytes =
            page("/XObject << /Outer 5 0 R >> /ExtGState << /On << /OP true /op true /OPM 1 >> >>", b"/Outer Do /On gs 50 10 20 20 re f", &[form]);
        (name, bytes, first)
    })
    .collect()
}

pub(crate) fn resource_scope_contract() {
    for (name, bytes, first) in resource_scope_cases() {
        let report = import_with_report(&bytes, &ImportOptions::default()).unwrap();
        let mut overprints = vec![];
        report.document.walk(|node| {
            if matches!(node.kind, NodeKind::Path { clipping: false, .. }) {
                overprints.extend(node.appearance.fill().map(|fill| fill.overprint));
            }
        });
        assert_eq!(overprints, [first, true], "form scope {name}; the caller's resources return after the form");
        assert!(report.warnings.is_empty(), "{name}: {:?}", report.warnings);
    }
}

#[test]
fn form_resource_scopes_keep_scanned_overprint_in_step_with_interpretation() {
    resource_scope_contract();
}

pub(crate) fn text_pdf(content: &str) -> Vec<u8> {
    page(
        "/Font << /F1 5 0 R >> /ExtGState << /G << /ca 0.25 /CA 0.75 >> >>",
        content.as_bytes(),
        &[
            b"<< /Type /Font /Subtype /TrueType /BaseFont /Inter-Regular /Encoding /WinAnsiEncoding /FirstChar 65 /LastChar 67 /Widths [600 600 600] /FontDescriptor 6 0 R >>".to_vec(),
            b"<< /Type /FontDescriptor /FontName /Inter-Regular /Flags 32 /FontFile2 7 0 R >>".to_vec(),
            stream("", INTER),
        ],
    )
}

pub(crate) fn text_content(mode: u8, explicit: bool, following: bool) -> String {
    let show = "1 0 0 1 10 20 Tm [(A) 400 (B)] TJ";
    let paint = if explicit { format!("0 Tr {show} {} Tr {show}", if mode == 6 { 5 } else { 1 }) } else { format!("{mode} Tr {show}") };
    // The green rectangle also exercises the text clip installed at ET for mode 6.
    let after = if following { " (C) Tj" } else { "" };
    format!("/G gs 1 0 0 rg 0 0 1 RG 2 w BT /F1 20 Tf {paint}{after} ET 0 1 0 rg 0 0 100 60 re f")
}

fn text_document(mode: u8, explicit: bool, text_as: TextAs) -> Document {
    import_with_report(&text_pdf(&text_content(mode, explicit, false)), &ImportOptions { text_as, ..Default::default() }).unwrap().document
}

pub(crate) fn text_order_contract() {
    for mode in [2, 6] {
        for text_as in [TextAs::Text, TextAs::Outlines] {
            let combined = text_document(mode, false, text_as);
            let explicit = text_document(mode, true, text_as);
            assert_eq!(
                serde_json::to_value(&combined).unwrap(),
                serde_json::to_value(&explicit).unwrap(),
                "{mode} Tr, {text_as:?}: imported objects must match a complete fill pass followed by a complete stroke pass"
            );
        }
    }
    let live = text_document(2, false, TextAs::Text);
    let mut texts = vec![];
    live.walk(|node| {
        if let NodeKind::Text(text) = &node.kind {
            texts.push((node.opacity, text.plain_text(), text.first_style().clone()));
        }
    });
    assert_eq!(texts.len(), 2, "the filled and stroked AB runs retain separate alpha: {texts:?}");
    assert_eq!((texts[0].0, texts[0].1.as_str()), (64.0 / 255.0, "AB"));
    assert_eq!((texts[1].0, texts[1].1.as_str()), (191.0 / 255.0, "AB"));
    assert!(texts[0].2.stroke.is_none() && !texts[0].2.fill.is_none());
    assert!(texts[1].2.fill.is_none() && !texts[1].2.stroke.is_none());

    let outlined = text_document(2, false, TextAs::Outlines);
    let mut paints = vec![];
    outlined.walk(|node| {
        if matches!(node.kind, NodeKind::Path { clipping: false, .. }) {
            if let Some(fill) = node.appearance.fill() {
                paints.push(('F', fill.opacity, fill.paint.clone()));
            }
            if let Some(stroke) = node.appearance.stroke() {
                paints.push(('S', stroke.opacity, stroke.paint.clone()));
            }
        }
    });
    assert_eq!(paints.len(), 3, "one whole-run fill, one whole-run stroke, then the green rectangle");
    assert_eq!((paints[0].0, paints[0].1), ('F', 64.0 / 255.0));
    assert_eq!((paints[1].0, paints[1].1), ('S', 191.0 / 255.0));
    assert_eq!(paints[2].0, 'F');
    assert!(matches!(paints[0].2, Paint::Solid { .. }) && matches!(paints[1].2, Paint::Solid { .. }));
}

#[test]
fn filled_stroked_tj_runs_preserve_app_text_outline_alpha_and_clip_grouping() {
    text_order_contract();
}

pub(crate) fn transfer_pdf(low: &str, high: &str) -> Vec<u8> {
    page(
        "/ExtGState << /G << /SMask << /S /Luminosity /G 5 0 R /TR 6 0 R >> >> >>",
        b"/G gs 1 0 0 rg 0 0 100 60 re f",
        &[
            stream("/Type /XObject /Subtype /Form /BBox [0 0 100 60] /Group << /S /Transparency /CS /DeviceRGB >>", b"1 1 1 rg 0 0 50 60 re f"),
            format!("<< /FunctionType 2 /Domain [0 1] /C0 [{low}] /C1 [{high}] /N 1 >>").into_bytes(),
        ],
    )
}

pub(crate) fn transfer_contract() {
    for (low, high, expected) in [("0.5001", "0.5", false), ("1", "0", true), ("0", "1", false)] {
        let report = import_with_report(&transfer_pdf(low, high), &ImportOptions::default()).unwrap();
        let mut inverted = vec![];
        report.document.walk(|node| inverted.extend(node.extras().mask.as_ref().map(|mask| mask.invert)));
        assert_eq!(inverted, [expected], "sampled endpoints C0={low}, C1={high}");
    }
}

#[test]
fn soft_mask_inversion_uses_quantized_transfer_endpoints() {
    transfer_contract();
}

/// #2675: text in rendering mode 3 (invisible, as a scanned page's OCR layer) comes in as type
/// with neither fill nor stroke. Where it can't come in as type (text read as outlines) it is
/// left out, and the import says so. Visible text beside it still comes in, and a page without
/// invisible text gets no such warning.
#[test]
fn invisible_text_is_reported_when_left_out() {
    let open = |content: &str, text_as: TextAs| import_with_report(&text_pdf(content), &ImportOptions { text_as, ..Default::default() }).unwrap();
    let invisible = "BT /F1 20 Tf 3 Tr 1 0 0 1 10 40 Tm (AB) Tj 0 Tr 1 0 0 1 10 10 Tm (CA) Tj ET";
    for text_as in [TextAs::Text, TextAs::Outlines] {
        let r = open(invisible, text_as);
        let warned = r.warnings.iter().any(|w| w == crate::INVISIBLE_TEXT_NOTE);
        assert_eq!(warned, text_as == TextAs::Outlines, "{text_as:?}: {:?}", r.warnings);
        // What came in: the type's text and whether it is painted, and where the painted art
        // lies. On the 60 pt page the visible line (baseline 10 pt up) lies 35–50 pt down, the
        // invisible one (40 pt up) 5–20 pt down.
        let (mut painted, mut unpainted, mut tops) = (String::new(), String::new(), vec![]);
        r.document.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                for run in &t.runs {
                    let out = if run.style.fill.is_none() && !run.style.has_stroke() { &mut unpainted } else { &mut painted };
                    out.push_str(&run.text);
                }
            }
            let shows = !matches!(&n.kind, NodeKind::Text(t) if t.runs.iter().all(|r| r.style.fill.is_none() && !r.style.has_stroke()));
            if shows
                && matches!(n.kind, NodeKind::Text(_) | NodeKind::Path { .. } | NodeKind::Compound { .. })
                && let Some(b) = n.visual_bounds()
            {
                tops.push(b.y0);
            }
        });
        if text_as == TextAs::Text {
            assert_eq!((painted.as_str(), unpainted.as_str()), ("CA", "AB"), "{text_as:?}");
        } else {
            assert_eq!(unpainted, "", "{text_as:?}");
        }
        assert!(!tops.is_empty(), "{text_as:?}: the visible line still comes in");
        assert!(tops.iter().all(|y| *y > 25.0), "{text_as:?}: only the visible line's art, tops at {tops:?}");
        let visible = open("BT /F1 20 Tf 1 0 0 1 10 10 Tm (CA) Tj ET", text_as);
        assert!(!visible.warnings.iter().any(|w| w == crate::INVISIBLE_TEXT_NOTE), "{text_as:?}: {:?}", visible.warnings);
    }
}
