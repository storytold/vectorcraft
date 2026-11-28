//! Dashes fitted to corners are written as the canvas's filled outlines; exact dashes stay a
//! PDF dash array.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Dash, Document, Node, NodeId, StrokeLayer};
use vectorcraft_geom::{Rect, shapes};
use vectorcraft_testkit::raster::{assert_similar, render_artboard};

use crate::tests_stroke::{line_doc, painted_boxes};
use crate::*;

fn dashed(align_corners: bool) -> StrokeLayer {
    let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), 4.0);
    st.dash = Some(Dash { pattern: vec![10.0, 10.0], offset: 0.0, align_corners });
    st
}

#[test]
fn fitted_dashes_are_written_as_fills_and_exact_ones_as_a_dash_array() {
    // 60 pt: three periods with half a dash at each end, so the dashes reach both end points.
    let (fills, strokes) = painted_boxes(&line_doc(dashed(true)).0);
    assert!(strokes.is_empty(), "{strokes:?}");
    assert_eq!(fills.len(), 1);
    let b = fills[0];
    assert!((b.x0 - 20.0).abs() < 0.01 && (b.x1 - 80.0).abs() < 0.01 && (b.height() - 4.0).abs() < 0.01, "{b:?}");
    let (d, _) = line_doc(dashed(false));
    let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
    let mut dashes = vec![];
    back.walk(|m| dashes.extend(m.appearance.stroke().and_then(|s| s.dash.clone())));
    assert_eq!(dashes.len(), 1);
    assert_eq!(dashes[0].pattern, vec![10.0, 10.0]);
}

#[test]
fn fitted_dashes_on_a_rectangle_look_as_on_the_canvas() {
    let mut d = Document::new(140.0, 90.0);
    let mut n = Node::path(
        NodeId(0),
        shapes::rectangle(Rect::new(20.0, 20.0, 120.0, 70.0)),
        Appearance { items: vec![AppearanceItem::Stroke(dashed(true))], ..Default::default() },
    );
    n.id = d.alloc_id();
    let layer = d.default_layer().unwrap();
    d.insert(Some(layer), 0, n).unwrap();
    let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
    assert_similar(&render_artboard(&d), &render_artboard(&back), 24.0, 0.002);
}
