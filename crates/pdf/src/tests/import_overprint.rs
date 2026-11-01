//! Overprint on import (#1388): the graphics state's `/OP` (strokes), `/op` (fills, `/OP` when
//! absent) and `/OPM` (ISO 32000-1 §8.6.7, Table 58) become the fills' and strokes' overprint.
//! Every file here is written by hand from the specification.

use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with};

use crate::tests_import_fidelity::stream;
use crate::*;

const ON: &str = "/ExtGState << /Off << /OP false /op false /OPM 0 >> /On << /OP true /op true /OPM 1 >> /Stroke << /OP true /op false /OPM 1 >> /Both << /OP true /OPM 1 >> /Zero << /OP true >> >>";

fn open(content: &str, resources: &str, extra: &[&str]) -> ImportReport {
    let page = PdfPage { resources: format!("{ON} {resources}"), ..PdfPage::new(400.0, 400.0, content) };
    import_with_report(&pdf_with(&[page], extra, None), &ImportOptions::default()).unwrap()
}

/// Each painted object in paint order: whether its fill and its stroke overprint (`None`: it has
/// none), type by its first run.
fn overprints(d: &Document) -> Vec<(Option<bool>, Option<bool>)> {
    let mut out = vec![];
    d.walk(|n: &Node| {
        if let NodeKind::Text(t) = &n.kind {
            out.extend(t.runs.first().map(|r| (Some(r.style.overprint_fill), (!r.style.stroke.is_none()).then_some(r.style.overprint_stroke))));
        } else if !n.appearance.items.is_empty() {
            out.push((n.appearance.fill().map(|f| f.overprint), n.appearance.stroke().map(|s| s.overprint)));
        }
    });
    out
}

fn about_overprint(r: &ImportReport) -> Vec<&String> {
    r.warnings.iter().filter(|w| w.contains("overprint")).collect()
}

const SPOT: &str = "/ColorSpace << /CS0 [/Separation /Spot1 /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 1 0 0] /N 1 >>] >>";

#[test]
fn a_spot_colour_overprinting_a_process_colour_stays_overprinting_through_export() {
    // The file of the report: a cyan square that knocks out, under a spot square that overprints.
    let r = open("/Off gs 1 0 0 0 k 50 50 200 200 re f /On gs /CS0 cs 1 scn 150 150 200 200 re f", SPOT, &[]);
    assert_eq!(overprints(&r.document), [(Some(false), None), (Some(true), None)]);
    assert!(about_overprint(&r).is_empty(), "{:?}", r.warnings);
    let spot = r.document.layers[0].children().unwrap().iter().find(|n| n.appearance.fill().is_some_and(|f| f.overprint)).unwrap();
    assert!(format!("{:?}", spot.appearance.fill().unwrap().paint).contains("Spot1"), "the spot square is the one that overprints");
    // Written again it overprints, and comes back the same.
    let out = export(&r.document, &PdfOptions::uncompressed()).unwrap();
    assert!(String::from_utf8_lossy(&out).contains("/OP true/op true/OPM 1"));
    let again = import_with_report(&out, &ImportOptions::default()).unwrap();
    assert_eq!(overprints(&again.document).iter().filter(|o| o.0 == Some(true)).count(), 1);
    assert_eq!(overprints(&again.document).iter().filter(|o| o.0 == Some(false)).count(), 1);
}

#[test]
fn op_sets_strokes_and_fills_unless_the_state_has_its_own_fill_entry() {
    // `/OP` alone is both; with `/op false` only strokes; `B` is one object with both paints.
    let r = open("1 0 0 RG 0 0 1 rg /Both gs 10 10 50 50 re B /Stroke gs 100 10 50 50 re B 200 10 50 50 re f 200 100 50 50 re S", "", &[]);
    assert_eq!(overprints(&r.document), [(Some(true), Some(true)), (Some(false), Some(true)), (Some(false), None), (None, Some(true))]);
    assert!(about_overprint(&r).is_empty(), "{:?}", r.warnings);
}

#[test]
fn saved_states_and_forms_keep_their_own_overprint() {
    let form = stream("/Type /XObject /Subtype /Form /BBox [0 0 400 400]", "200 10 50 50 re f /Off gs 200 100 50 50 re f");
    let x = first_extra(1);
    let r = open(
        "0 0 1 rg q /On gs 10 10 50 50 re f Q 100 10 50 50 re f /On gs /X0 Do 300 10 50 50 re f",
        &format!("/XObject << /X0 {x} 0 R >>"),
        &[&form],
    );
    // In `q … Q`; after it; in the form (the page's state, then its own); after the form.
    assert_eq!(overprints(&r.document).iter().map(|o| o.0).collect::<Vec<_>>(), [true, false, true, false, true].map(Some));
}

#[test]
fn overprint_mode_0_is_reported() {
    // No `/OPM`: mode 0, where a DeviceCMYK colour's zero components knock out.
    let r = open("/Zero gs 0 0 0 1 k 10 10 50 50 re f", "", &[]);
    assert_eq!(overprints(&r.document), [(Some(true), None)]);
    assert_eq!(about_overprint(&r).len(), 1, "{:?}", r.warnings);
    assert!(about_overprint(&r)[0].contains("/OPM 0"));
    // Nothing overprints: nothing to say.
    assert!(about_overprint(&open("0 0 0 1 k 10 10 50 50 re f", "", &[])).is_empty());
}

const FONT: &str = "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >>";

#[test]
fn text_overprints_as_its_graphics_state_says() {
    // Type or its outlines, as the fonts of the machine allow.
    let r = open("0 0 1 rg 10 300 50 50 re f /On gs BT /F1 36 Tf 40 200 Td (Ink) Tj ET /Off gs 100 300 50 50 re f", FONT, &[]);
    assert_eq!(overprints(&r.document).iter().map(|o| o.0).collect::<Vec<_>>(), [false, true, false].map(Some), "{:?}", r.warnings);
    assert!(about_overprint(&r).is_empty(), "{:?}", r.warnings);
}

#[test]
fn text_of_two_overprint_states_between_two_paths_knocks_out_with_a_warning() {
    let r = open("0 0 1 rg BT /F1 36 Tf 40 200 Td /On gs (Ink) Tj /Off gs 0 -50 Td (Paper) Tj ET", FONT, &[]);
    assert!(overprints(&r.document).iter().all(|o| o.0 == Some(false)), "{:?}", overprints(&r.document));
    assert_eq!(about_overprint(&r).len(), 1, "{:?}", r.warnings);
}

#[test]
fn a_page_whose_paths_cant_be_counted_knocks_out_with_a_warning() {
    // A shading the interpreter can't read draws nothing: the paths after it would take the
    // overprint of the ones before them.
    let r = open("0 0 1 rg /Sh0 sh /On gs 10 10 50 50 re f /Off gs 100 10 50 50 re f", "/Shading << /Sh0 << /ShadingType 99 >> >>", &[]);
    assert_eq!(overprints(&r.document), [(Some(false), None), (Some(false), None)]);
    assert_eq!(about_overprint(&r).len(), 1, "{:?}", r.warnings);
}

#[test]
fn hidden_optional_content_and_pattern_cells_dont_shift_the_count() {
    let ocg = "<< /Type /OCG /Name (Hidden) >>";
    let cell = stream(
        "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >>",
        "1 0 0 rg 0 0 5 5 re f 5 5 5 5 re f",
    );
    let x = first_extra(1);
    let page = PdfPage {
        resources: format!("{ON} /Properties << /MC0 {x} 0 R >> /Pattern << /P0 {} 0 R >>", x + 1),
        ..PdfPage::new(
            400.0,
            400.0,
            "/OC /MC0 BDC 0 0 1 rg 10 10 50 50 re f EMC /Pattern cs /P0 scn 100 10 50 50 re f /On gs 0 0 1 rg 200 10 50 50 re f",
        )
    };
    let catalog = format!("/OCProperties << /OCGs [{x} 0 R] /D << /OFF [{x} 0 R] >> >>");
    let bytes = vectorcraft_testkit::pdf::pdf_with_catalog(&[page], &[ocg, &cell], &catalog, None);
    for layers in [false, true] {
        let r = import_with_report(&bytes, &ImportOptions { layers, ..Default::default() }).unwrap();
        let on: Vec<_> = overprints(&r.document).into_iter().filter(|o| o.0 == Some(true)).collect();
        assert_eq!(on.len(), 1, "layers {layers}: {:?} {:?}", overprints(&r.document), r.warnings);
        assert!(about_overprint(&r).is_empty(), "layers {layers}: {:?}", r.warnings);
    }
}
