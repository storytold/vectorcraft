//! Live effects in SVG export: a fill or stroke's own raster effects become filters on that item's
//! element, and geometry effects on type are baked through its outlines.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, CharStyle, Document, Effect, Node, NodeId, NodeKind, TextObject};
use vectorcraft_geom::{Point, Rect, shapes};
use vectorcraft_svg::{ExportOptions, export};

fn doc_with(mut n: Node) -> Document {
    let mut d = Document::new(200.0, 200.0);
    n.id = d.alloc_id();
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

fn fx(id: &str) -> Effect {
    Effect { id: id.into(), params: json!({}), visible: true }
}

#[test]
fn revolve_exports_shaded_vector_faces_and_keeps_the_source_live() {
    let bp = vectorcraft_geom::BezPath::from_svg("M100 30 L130 50 L130 140 L100 160").unwrap();
    let mut n = Node::path(
        NodeId(0),
        vectorcraft_geom::PathData::from_bezpath(&bp),
        Appearance::basic(Paint::solid(Color::rgb(0.8, 0.2, 0.1)), Paint::None, 0.0),
    );
    n.appearance.effects.push(fx("threeD.revolve"));
    let d = doc_with(n);
    let before = d.clone();
    let svg = export(&d, &ExportOptions::default());
    assert!(svg.matches("<path").count() > 50, "Revolve should export vector surfaces");
    assert!(!svg.contains("<image"), "no raster fallback");
    assert_eq!(d, before, "export leaves source editable");
}

#[test]
fn a_strokes_own_glow_filters_the_stroke_element_only() {
    let mut n = Node::path(
        NodeId(0),
        shapes::rectangle(Rect::new(20.0, 20.0, 80.0, 80.0)),
        Appearance::basic(Paint::solid(Color::WHITE), Paint::solid(Color::BLACK), 2.0),
    );
    n.appearance.items[1].effects_mut().push(fx("stylize.outerGlow"));
    let svg = export(&doc_with(n), &ExportOptions::default());
    assert_eq!(svg.matches("<filter ").count(), 1, "{svg}");
    // The filter group holds the stroke's path, and the fill's path comes before it, unfiltered.
    let open = svg.find("<g filter=\"url(#").expect("a filter group");
    let inside = &svg[open..svg[open..].find("</g>").map(|i| open + i).unwrap()];
    assert!(inside.contains("stroke=\"#000000\"") && !inside.contains("fill=\"#ffffff\""), "{inside}");
    assert!(svg[..open].contains("fill=\"#ffffff\""), "{svg}");
    // A hidden effect exports nothing.
    let mut n = Node::path(
        NodeId(0),
        shapes::rectangle(Rect::new(20.0, 20.0, 80.0, 80.0)),
        Appearance::basic(Paint::solid(Color::WHITE), Paint::solid(Color::BLACK), 2.0),
    );
    n.appearance.items[1].effects_mut().push(Effect { visible: false, ..fx("stylize.outerGlow") });
    assert!(!export(&doc_with(n), &ExportOptions::default()).contains("<filter "));
}

#[test]
fn geometry_effects_on_type_export_as_its_reshaped_outlines() {
    let style = CharStyle { size: 40.0, fill: Paint::solid(Color::BLACK), ..Default::default() };
    let mut n = Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 60.0), "Hi", style))));
    let plain = export(&doc_with(n.clone()), &ExportOptions::default());
    assert!(plain.contains("<text"), "{plain}");
    n.appearance.effects =
        vec![Effect { id: "distort.roughen".into(), params: json!({"size": 3, "relative": false}), visible: true }, fx("stylize.dropShadow")];
    let svg = export(&doc_with(n), &ExportOptions::default());
    assert!(!svg.contains("<text") && svg.contains("<path"), "{svg}");
    assert!(svg.contains("<filter "), "the shadow stays a filter: {svg}");
}
