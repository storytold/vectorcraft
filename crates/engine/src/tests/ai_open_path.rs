//! An open path in a PDF-compatible `.ai` opens as drawn (#2920). The PDF reader took a five-point
//! open polyline whose points reach every corner of its own bounding box for a rectangle, so the
//! zigzag below opened as a closed box. The file is written here from ISO 32000; it has no
//! Illustrator editing data, so it opens from its PDF content as such an `.ai` does.

use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_geom::SubPath;
use vectorcraft_testkit::pdf::{PdfPage, pdf};

use crate::*;

/// A 2 pt black stroke, no fill: a "W" whose middle peak rises only halfway. Its ends and the
/// points beside them are the four corners of its bounding box; the peak is on none of them.
const ZIGZAG: &str = "0 0 0 RG 2 w 80 38 m 80 21.75 l 60.571 29.875 l 40 21.75 l 40 38 l S";

fn subpaths(name: &str, content: &str) -> Vec<SubPath> {
    let bytes = pdf(&[PdfPage::new(120.0, 60.0, content)], None);
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(&bytes)})).unwrap();
    let mut out = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if let NodeKind::Path { path, .. } = &n.kind {
            out.extend(path.subpaths.iter().cloned());
        }
    });
    out
}

#[test]
fn an_open_zigzag_in_an_ai_file_opens_open_with_its_five_points() {
    let paths = subpaths("zigzag.ai", ZIGZAG);
    assert_eq!(paths.len(), 1, "{paths:?}");
    let zigzag = &paths[0];
    assert!(!zigzag.closed, "the open path was closed: {zigzag:?}");
    assert_eq!(zigzag.anchors.len(), 5, "{zigzag:?}");
    let xs: Vec<f64> = zigzag.anchors.iter().map(|a| a.p.x).collect();
    for (got, want) in xs.iter().zip([80.0, 80.0, 60.571, 40.0, 40.0]) {
        assert!((got - want).abs() < 1e-3, "anchors at x {xs:?}");
    }
    // The peak is halfway up, not on the bounding box's edge.
    let ys: Vec<f64> = zigzag.anchors.iter().map(|a| a.p.y).collect();
    assert!((ys[2] - (ys[1] + ys[0]) / 2.0).abs() < 1e-3, "anchors at y {ys:?}");
}

#[test]
fn the_same_zigzag_in_a_pdf_opens_open_too() {
    let paths = subpaths("zigzag.pdf", ZIGZAG);
    assert_eq!(paths.iter().map(|p| (p.closed, p.anchors.len())).collect::<Vec<_>>(), [(false, 5)]);
}

#[test]
fn a_rectangle_written_as_lines_still_opens_as_a_closed_rectangle() {
    let paths = subpaths("box.ai", "0 0 0 RG 2 w 40 21.75 m 80 21.75 l 80 38 l 40 38 l h S");
    assert_eq!(paths.len(), 1, "{paths:?}");
    assert!(paths[0].closed);
    assert_eq!(paths[0].anchors.len(), 4, "{paths:?}");
}
