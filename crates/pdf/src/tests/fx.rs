//! Effects in PDF export: raster effects are not written yet, so a fill's, a stroke's or the
//! object's own raster effect is reported; geometry effects are baked into what the file draws.

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, CharStyle, Document, Effect, Node, NodeId, NodeKind, TextObject};
use vectorcraft_geom::{Point, Rect, shapes};

use crate::*;

fn warnings(item: Option<usize>) -> Vec<String> {
    let mut d = Document::new(100.0, 100.0);
    let mut n = Node::path(
        NodeId(0),
        shapes::rectangle(Rect::new(20.0, 20.0, 80.0, 80.0)),
        Appearance::basic(Paint::solid(Color::WHITE), Paint::solid(Color::BLACK), 2.0),
    );
    n.appearance.effects_mut(item).unwrap().push(Effect { id: "stylize.outerGlow".into(), params: Default::default(), visible: true });
    n.id = d.alloc_id();
    let layer = d.default_layer().unwrap();
    d.insert(Some(layer), 0, n).unwrap();
    export_with_report(&d, &PdfOptions::default()).unwrap().warnings
}

#[test]
fn raster_effects_warn_on_fills_strokes_and_objects() {
    for item in [None, Some(0), Some(1)] {
        assert!(warnings(item).iter().any(|w| w.starts_with("raster effects")), "{item:?}");
    }
}

/// What `n` paints, in paint order, with consecutive repeats dropped: 'b' for a blue fill, 'r' for a
/// red fill and 'g' for a green stroke, each with whether a clip group clips it. Clipping paths
/// themselves are left out.
fn painted_order(n: &Node, clipped: bool, out: &mut Vec<(char, bool)>) {
    let solid = |p: &Paint, r: f32, g: f32, b: f32| matches!(p, Paint::Solid { color, .. } if *color == Color::rgb(r, g, b));
    for i in &n.appearance.items {
        let mark = match i {
            AppearanceItem::Fill(f) if solid(&f.paint, 0.0, 0.0, 1.0) => 'b',
            AppearanceItem::Fill(f) if solid(&f.paint, 1.0, 0.0, 0.0) => 'r',
            AppearanceItem::Stroke(s) if s.width > 0.0 && solid(&s.paint, 0.0, 1.0, 0.0) => 'g',
            _ => continue,
        };
        if out.last() != Some(&(mark, clipped)) {
            out.push((mark, clipped));
        }
    }
    let clips = matches!(n.kind, NodeKind::Group { clip: true, .. });
    for c in n.children().into_iter().flatten().skip(usize::from(clips)) {
        painted_order(c, clipped || clips, out);
    }
}

#[test]
fn painted_clipping_type_exports_with_group_effect() {
    // Type filled blue and stroked green clips a red square in a group with an Offset Path effect:
    // the PDF paints the type's fill inside the clip behind the square, and its stroke over the
    // clipped art, outside the clip.
    let blue = Paint::solid(Color::rgb(0.0, 0.0, 1.0));
    let green = Paint::solid(Color::rgb(0.0, 1.0, 0.0));
    let style = CharStyle { size: 40.0, fill: blue, stroke: green, stroke_width: 2.0, ..Default::default() };
    let mut d = Document::new(200.0, 200.0);
    let text = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(5.0, 60.0), "IIII", style))));
    let red = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let square = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 200.0)), red);
    let clip = Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(text), Arc::new(square)], clip: true });
    let mut outer = Node::group(d.alloc_id(), vec![Arc::new(clip)]);
    outer.appearance.effects.push(Effect { id: "path.offsetPath".into(), params: serde_json::json!({"offset": 1}), visible: true });
    let layer = d.default_layer().unwrap();
    d.insert(Some(layer), 0, outer).unwrap();
    let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
    let mut order = vec![];
    for l in &back.layers {
        painted_order(l, false, &mut order);
    }
    assert_eq!(order, [('b', true), ('r', true), ('g', false)]);
}
