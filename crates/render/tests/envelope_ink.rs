//! A renderer made without an engine session makes envelopes measure type by its glyph outlines.
//! One test in its own process, so nothing else has registered the measure before the renderer is
//! made.
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
fn a_new_renderer_measures_type_in_envelopes_by_its_glyphs() {
    let t = TextObject::point(Point::new(10.0, 60.0), "Type", CharStyle { size: 48.0, ..Default::default() });
    let n = Arc::new(Node::new(NodeId(1), NodeKind::Text(Box::new(t.clone()))));
    let laid = n.geometric_bounds().unwrap();
    let ink = t.xf.transform_rect_bbox(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).ink_bounds().unwrap());
    assert!(laid.height() - ink.height() > 1.0, "the laid-out box {laid:?} reaches past the glyphs {ink:?}");
    let before = vectorcraft_doc::live::envelope_content_bounds(std::slice::from_ref(&n)).unwrap();
    assert!(off(before, laid) < 1e-6, "with nothing registered, type keeps its laid-out box: {before:?} vs {laid:?}");

    let _ = vectorcraft_render::Renderer::new();
    let after = vectorcraft_doc::live::envelope_content_bounds(&[n]).unwrap();
    assert!(off(after, ink) < 1e-6, "after a renderer is made, type is measured by its glyphs: {after:?} vs {ink:?}");
}
