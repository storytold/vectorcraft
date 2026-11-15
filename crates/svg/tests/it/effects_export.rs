//! Live effects in SVG export: a fill or stroke's own raster effects become filters on that item's
//! element, and geometry effects on type are baked through its outlines.

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
fn revolve_caps_export_as_additional_vector_faces_and_leave_source_editable() {
    let path = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_svg("M100 30 L100 160").unwrap());
    let mut n = Node::path(NodeId(0), path, Appearance::basic(Paint::solid(Color::rgb(0.8, 0.2, 0.1)), Paint::None, 0.0));
    n.appearance.effects.push(Effect { params: json!({"offset":30,"rotationX":-35,"segments":16,"cap":false}), ..fx("threeD.revolve") });
    let mut d = doc_with(n);
    let open = export(&d, &ExportOptions::default());
    let id = d.layers[0].children().unwrap()[0].id;
    d.node_mut(id).unwrap().appearance.effects[0].params["cap"] = json!(true);
    let before = d.clone();
    let capped = export(&d, &ExportOptions::default());
    assert_eq!(
        vectorcraft_testkit::svg_shapes(&capped),
        vectorcraft_testkit::svg_shapes(&open) + 2,
        "two continuous caps, each with a fill and rim stroke"
    );
    assert!(!capped.contains("<image"));
    assert_eq!(d, before);
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
    assert!(vectorcraft_testkit::svg_shapes(&svg) > 50, "Revolve should export vector surfaces");
    assert!(!svg.contains("<image"), "no raster fallback");
    assert_eq!(d, before, "export leaves source editable");
}

#[test]
fn smooth_revolve_exports_vector_gradients_without_rasterizing_or_editing_the_source() {
    let bp = vectorcraft_geom::BezPath::from_svg("M100 30 C140 50 140 120 100 160").unwrap();
    let mut n = Node::path(
        NodeId(0),
        vectorcraft_geom::PathData::from_bezpath(&bp),
        Appearance::basic(Paint::solid(Color::rgb(0.2, 0.55, 0.9)), Paint::None, 0.0),
    );
    n.appearance.effects.push(Effect { params: json!({"offset":20,"rotationX":-25,"cap":true,"segments":16}), ..fx("threeD.revolve") });
    let d = doc_with(n);
    let before = d.clone();
    let svg = export(&d, &ExportOptions::default());
    assert!(svg.matches("<linearGradient").count() > 50);
    assert!(svg.contains("gradientUnits=\"userSpaceOnUse\""));
    assert!(vectorcraft_testkit::svg_shapes(&svg) > 100);
    assert!(svg.matches("stroke-linejoin=\"round\"").count() > 100, "seam strokes keep bounded joins in SVG");
    assert!(!svg.contains("stroke-miterlimit"));
    assert!(!svg.contains("<image") && !svg.contains("<filter"));
    assert_eq!(d, before);
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
fn a_clipping_paths_transform_effect_moves_the_written_clip_path() {
    // A clip group whose clipping path covers x 0 to 50; `effect` moves it 50 pt right.
    let clipped = |x0: f64, effect: bool| {
        let mut clip =
            Node::path(NodeId(0), shapes::rectangle(Rect::new(x0, 0.0, x0 + 50.0, 100.0)), Appearance::basic(Paint::None, Paint::None, 0.0));
        if let NodeKind::Path { clipping, .. } = &mut clip.kind {
            *clipping = true;
        }
        if effect {
            clip.appearance.effects.push(Effect { id: "distort.transform".into(), params: json!({"moveH": 50}), visible: true });
        }
        let art = Node::path(
            NodeId(0),
            shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        let mut d = Document::new(200.0, 200.0);
        let (clip, art) = (Node { id: d.alloc_id(), ..clip }, Node { id: d.alloc_id(), ..art });
        let g = Node::new(d.alloc_id(), NodeKind::Group { children: vec![std::sync::Arc::new(clip), std::sync::Arc::new(art)], clip: true });
        let l = d.layers[0].id;
        d.insert(Some(l), 0, g).unwrap();
        let svg = export(&d, &ExportOptions::default());
        let open = svg.find("<clipPath").expect("a clip path");
        let body = &svg[open..open + svg[open..].find("</clipPath>").expect("its end")];
        body[body.find('>').unwrap()..].to_string()
    };
    assert_eq!(clipped(0.0, true), clipped(50.0, false), "the clip path is written where the effect moves it");
    assert_ne!(clipped(0.0, true), clipped(0.0, false));
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
