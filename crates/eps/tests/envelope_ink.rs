//! Reading an `.ai` file makes envelopes measure type by its glyph outlines, as the app does. One
//! test in its own process, so nothing else has registered the measure before the file is read.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use vectorcraft_doc::{CharStyle, Node, NodeId, NodeKind, TextObject};
use vectorcraft_geom::{Point, Rect};

/// The largest distance between the edges of `a` and `b`.
fn off(a: Rect, b: Rect) -> f64 {
    [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().fold(0.0f64, |m, v| m.max(v.abs()))
}

#[test]
fn reading_an_ai_file_measures_type_in_envelopes_by_its_glyphs() {
    let t = TextObject::point(Point::new(10.0, 60.0), "Type", CharStyle { size: 48.0, ..Default::default() });
    let n = Arc::new(Node::new(NodeId(1), NodeKind::Text(Box::new(t.clone()))));
    let laid = n.geometric_bounds().unwrap();
    let ink = t.xf.transform_rect_bbox(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).ink_bounds().unwrap());
    assert!(laid.height() - ink.height() > 1.0, "the laid-out box {laid:?} reaches past the glyphs {ink:?}");
    let before = vectorcraft_doc::live::envelope_content_bounds(std::slice::from_ref(&n)).unwrap();
    assert!(off(before, laid) < 1e-6, "with nothing registered, type keeps its laid-out box: {before:?} vs {laid:?}");

    let file = "%!PS-Adobe-3.0\n%%Creator: test\n%AI5_FileFormat 3.0\n%AI3_Cropmarks: 0 0 200 100\n%%EndComments\n%%BeginProlog\n%%EndProlog\n%%BeginSetup\n%%EndSetup\n%AI5_BeginLayer\n1 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n(L) Ln\nLB\n%AI5_EndLayer--\n%%PageTrailer\ngsave annotatepage grestore showpage\n%%Trailer\n%%EOF\n";
    vectorcraft_eps::import(file.as_bytes()).unwrap();
    let after = vectorcraft_doc::live::envelope_content_bounds(&[n]).unwrap();
    assert!(off(after, ink) < 1e-6, "after reading, type is measured by its glyphs: {after:?} vs {ink:?}");
}
