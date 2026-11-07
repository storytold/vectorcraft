//! Strokes on type in PDF: the characters' and the object's strokes keep their join and dashes.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, CharStyle, Dash, Document, LineJoin, Node, NodeKind, StrokeLayer, TextObject};
use vectorcraft_geom::Point;

use crate::*;

/// The strokes of a PDF of `d` read back.
fn strokes_back(d: &Document) -> Vec<StrokeLayer> {
    let back = import(&export(d, &PdfOptions::default()).unwrap()).unwrap();
    let mut out = vec![];
    back.walk(|n| out.extend(n.appearance.items.iter().filter_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s.clone()) } else { None })));
    out
}

#[test]
fn character_and_object_strokes_on_type_keep_join_and_dashes() {
    let dash = Some(Dash { pattern: vec![3.0, 2.0], ..Default::default() });
    let style = CharStyle {
        size: 40.0,
        stroke: Paint::solid(Color::BLACK),
        stroke_width: 2.0,
        stroke_join: LineJoin::Round,
        stroke_dash: dash.clone(),
        ..Default::default()
    };
    let mut d = Document::new(200.0, 100.0);
    let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 60.0), "Type", style))));
    let mut own = StrokeLayer::new(Paint::solid(Color::rgb8(255, 0, 0)), 1.0);
    own.join = LineJoin::Bevel;
    own.dash = Some(Dash { pattern: vec![5.0, 5.0], ..Default::default() });
    n.appearance.items.push(AppearanceItem::Stroke(own));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let strokes = strokes_back(&d);
    let pattern = |s: &StrokeLayer| s.dash.as_ref().map(|d| d.pattern.iter().map(|v| (v * 100.0).round() / 100.0).collect::<Vec<_>>());
    assert!(strokes.iter().any(|s| s.join == LineJoin::Round && pattern(s) == Some(vec![3.0, 2.0])), "character stroke: {strokes:?}");
    assert!(strokes.iter().any(|s| s.join == LineJoin::Bevel && pattern(s) == Some(vec![5.0, 5.0])), "object stroke: {strokes:?}");
}
