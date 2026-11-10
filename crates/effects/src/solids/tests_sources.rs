use serde_json::json;
use vectorcraft_doc::color::Color;
use vectorcraft_doc::{AppearanceItem, BlendSpacing, BlendSpec, Document, FillLayer, ImageObject, NodeId, StrokeLayer, Symbol};
use vectorcraft_geom::{Affine, shapes};

use super::*;

const TYPES: [&str; 5] = [EXTRUDE, CLASSIC_EXTRUDE, INFLATE, ROTATE, crate::REVOLVE];

/// Rotate facing the viewer: the art drawn where it is.
fn front() -> Value {
    json!({"rotationX": 0.0, "rotationY": 0.0, "rotationZ": 0.0})
}

fn rect(id: u64, x0: f64, y0: f64, x1: f64, y1: f64, stroke: f64) -> Node {
    let path = shapes::rectangle(Rect::new(x0, y0, x1, y1));
    let stroke_paint = if stroke > 0.0 { Paint::solid(Color::BLACK) } else { Paint::None };
    Node::path(NodeId(id), path, Appearance::basic(Paint::solid(Color::rgb(0.2, 0.5, 0.8)), stroke_paint, stroke))
}

fn with(mut n: Node, id: &str, params: Value) -> Node {
    n.appearance.effects.push(crate::new_effect(id, &params).unwrap());
    n
}

fn art(d: &Document, n: &Node, id: &str) -> Option<Node> {
    if id == crate::REVOLVE { crate::revolve_art_in(Some(d), n) } else { solid_art_in(Some(d), n) }
}

fn bounds(n: &Node) -> Rect {
    n.visual_bounds().unwrap()
}

fn close(a: Rect, b: Rect, tolerance: f64) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(p, q)| (p - q).abs() <= tolerance)
}

fn document() -> Document {
    let mut d = Document::new(1000.0, 1000.0);
    d.symbols.push(Symbol { dynamic: false, name: "Box".into(), art: Arc::new(rect(0, 0.0, 0.0, 100.0, 100.0, 2.0)) });
    d
}

/// A clipping group: `art` inside a rectangular clipping path covering `clip`.
fn clip_group(id: u64, clip: Rect, art: Node) -> Node {
    let mut path = Node::path(NodeId(id + 1), shapes::rectangle(clip), Appearance::default());
    if let NodeKind::Path { clipping, .. } = &mut path.kind {
        *clipping = true;
    }
    Node::new(NodeId(id), NodeKind::Group { children: vec![Arc::new(path), Arc::new(art)], clip: true })
}

fn blend(id: u64, steps: u32) -> Node {
    let keys = vec![Arc::new(rect(id + 1, 100.0, 100.0, 140.0, 140.0, 0.0)), Arc::new(rect(id + 2, 300.0, 120.0, 360.0, 180.0, 0.0))];
    let spec = BlendSpec { spacing: BlendSpacing::Steps(steps), ..Default::default() };
    Node::new(NodeId(id), NodeKind::Blend { children: keys, spec: Box::new(spec) })
}

fn instance(id: u64, symbol: &str, xf: Affine) -> Node {
    Node::new(NodeId(id), NodeKind::SymbolInstance { symbol: symbol.into(), xf })
}

fn image(id: u64) -> Node {
    Node::new(
        NodeId(id),
        NodeKind::Image(Box::new(ImageObject {
            key: "k".into(),
            width: 10,
            height: 10,
            xf: Affine::IDENTITY,
            link: None,
            placement: Default::default(),
            pixels: None,
        })),
    )
}

fn sources() -> [(&'static str, Node); 3] {
    [
        ("clipping group", clip_group(10, Rect::new(100.0, 100.0, 150.0, 200.0), rect(12, 100.0, 100.0, 200.0, 200.0, 2.0))),
        ("blend", blend(20, 3)),
        ("symbol instance", instance(30, "Box", Affine::translate((300.0, 0.0)))),
    ]
}

#[test]
fn every_three_d_type_builds_clipping_groups_blends_and_symbol_instances_alone_and_in_a_group() {
    let d = document();
    for (what, source) in sources() {
        for id in TYPES {
            let n = with(source.clone(), id, json!({}));
            assert_eq!(validate_three_d_in(Some(&d), &n, id, &json!({})), Ok(()), "{id} on a {what}");
            assert!(art(&d, &n, id).is_some_and(|a| a.visual_bounds().is_some()), "{id} on a {what} draws");
            assert!(three_d_meshes_in(Some(&d), &n).is_ok_and(|m| !m.is_empty()), "{id} on a {what} exports");
            let g = with(Node::group(NodeId(90), vec![Arc::new(source.clone()), Arc::new(rect(91, 600.0, 600.0, 650.0, 650.0, 0.0))]), id, json!({}));
            assert_eq!(validate_three_d_in(Some(&d), &g, id, &json!({})), Ok(()), "{id} on a group holding a {what}");
            assert!(art(&d, &g, id).is_some(), "{id} on a group holding a {what} draws");
            assert!(three_d_meshes_in(Some(&d), &g).is_ok_and(|m| !m.is_empty()), "{id} on a group holding a {what} exports");
        }
    }
}

#[test]
fn a_clipping_groups_solid_is_built_from_its_art_inside_the_clip() {
    let d = document();
    let clip = Rect::new(100.0, 100.0, 150.0, 200.0);
    let fill_only = with(clip_group(10, clip, rect(12, 100.0, 100.0, 200.0, 200.0, 0.0)), ROTATE, front());
    let cut = with(Node::group(NodeId(10), vec![Arc::new(rect(12, 100.0, 100.0, 150.0, 200.0, 0.0))]), ROTATE, front());
    assert!(close(bounds(&solid_art_in(Some(&d), &fill_only).unwrap()), bounds(&solid_art(&cut).unwrap()), 0.01));
    // The stroke is cut to the clip too: nothing reaches past it but the faces' seams.
    let stroked = with(clip_group(10, clip, rect(12, 100.0, 100.0, 200.0, 200.0, 8.0)), EXTRUDE, front());
    let b = bounds(&solid_art_in(Some(&d), &stroked).unwrap());
    let outside = b.x0 < clip.x0 - 1.0 || b.x1 > clip.x1 + 1.0;
    let extruded =
        bounds(&solid_art(&with(Node::group(NodeId(10), vec![Arc::new(rect(12, 100.0, 100.0, 150.0, 200.0, 0.0))]), EXTRUDE, front())).unwrap());
    assert!(!outside && close(b, extruded, 1.0), "{b:?} within {clip:?}");
    // A clip inside a clip keeps what both show.
    let nested =
        with(clip_group(40, Rect::new(120.0, 0.0, 400.0, 400.0), clip_group(10, clip, rect(12, 100.0, 100.0, 200.0, 200.0, 0.0))), ROTATE, front());
    let both = with(Node::group(NodeId(10), vec![Arc::new(rect(12, 120.0, 100.0, 150.0, 200.0, 0.0))]), ROTATE, front());
    assert!(close(bounds(&solid_art_in(Some(&d), &nested).unwrap()), bounds(&solid_art(&both).unwrap()), 0.01));
    // A clipping group used as the clipping path clips by its own clipping path, not by its art.
    let mask = Arc::new(clip_group(70, clip, rect(72, 100.0, 100.0, 110.0, 110.0, 0.0)));
    let by_mask = with(
        Node::new(NodeId(10), NodeKind::Group { children: vec![mask, Arc::new(rect(12, 100.0, 100.0, 200.0, 200.0, 0.0))], clip: true }),
        ROTATE,
        front(),
    );
    assert!(close(bounds(&solid_art_in(Some(&d), &by_mask).unwrap()), bounds(&solid_art(&cut).unwrap()), 0.01));
    // A stroke between two fills keeps its place below the top fill's solid.
    let mut layered = rect(12, 100.0, 100.0, 150.0, 200.0, 0.0);
    layered.appearance = Appearance {
        items: vec![
            AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::rgb(1.0, 0.0, 0.0)))),
            AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::rgb(0.0, 1.0, 0.0)), 6.0)),
            AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::rgb(0.0, 0.0, 1.0)))),
        ],
        ..Default::default()
    }
    .into();
    let colors = |n: &Node| three_d_meshes_in(Some(&d), &with(n.clone(), EXTRUDE, json!({}))).unwrap().iter().map(|m| m.color).collect::<Vec<_>>();
    let plain = colors(&Node::group(NodeId(10), vec![Arc::new(layered.clone())]));
    assert_eq!(plain.len(), 2, "the stroke's solid and the top fill's");
    assert_eq!(colors(&clip_group(10, Rect::new(0.0, 0.0, 400.0, 400.0), layered)), plain);
    // Art nested past the limit inside a clipping group is refused, not followed to any depth.
    let mut deep = rect(12, 100.0, 100.0, 200.0, 200.0, 0.0);
    for i in 0..200 {
        deep = Node::group(NodeId(1000 + i), vec![Arc::new(deep)]);
    }
    let n = with(clip_group(10, clip, deep), ROTATE, front());
    assert!(validate_three_d_in(Some(&d), &n, ROTATE, &json!({})).is_err_and(|e| e.contains("levels deep")));
    assert!(solid_art_in(Some(&d), &n).is_none());
    // A clip that shows none of the art builds nothing, and says so.
    let empty = clip_group(10, Rect::new(500.0, 500.0, 550.0, 550.0), rect(12, 100.0, 100.0, 200.0, 200.0, 2.0));
    for id in TYPES {
        let n = with(empty.clone(), id, json!({}));
        assert!(validate_three_d_in(Some(&d), &n, id, &json!({})).is_err_and(|e| e.contains("at least two anchors")), "{id}");
        assert!(art(&d, &n, id).is_none(), "{id}");
    }
}

#[test]
fn a_symbol_instances_solid_is_its_symbols_art_where_the_instance_places_it() {
    let d = document();
    let placed = with(instance(30, "Box", Affine::translate((300.0, 50.0))), EXTRUDE, json!({}));
    let same = with(Node::group(NodeId(30), vec![Arc::new(rect(31, 300.0, 50.0, 400.0, 150.0, 2.0))]), EXTRUDE, json!({}));
    assert!(close(bounds(&solid_art_in(Some(&d), &placed).unwrap()), bounds(&solid_art(&same).unwrap()), 0.01));
    // A redefined symbol reshapes the solid.
    let mut wider = d.clone();
    wider.symbols[0].art = Arc::new(rect(0, 0.0, 0.0, 200.0, 100.0, 2.0));
    assert!(bounds(&solid_art_in(Some(&wider), &placed).unwrap()).width() > bounds(&solid_art_in(Some(&d), &placed).unwrap()).width() + 50.0);
    // Without the symbol there is nothing to build from.
    let missing = with(instance(30, "Gone", Affine::IDENTITY), EXTRUDE, json!({}));
    assert!(validate_three_d_in(Some(&d), &missing, EXTRUDE, &json!({})).is_err_and(|e| e.contains("\"Gone\"")));
    assert!(validate_three_d(&placed, EXTRUDE, &json!({})).is_err(), "no document: no symbol art");
    // A symbol holding an instance of itself is refused, not followed without end.
    let mut looped = d.clone();
    let inner = Arc::new(instance(0, "Loop", Affine::translate((10.0, 10.0))));
    looped.symbols.push(Symbol {
        dynamic: false,
        name: "Loop".into(),
        art: Arc::new(Node::group(NodeId(0), vec![Arc::new(rect(0, 0.0, 0.0, 10.0, 10.0, 0.0)), inner])),
    });
    let n = with(instance(30, "Loop", Affine::IDENTITY), EXTRUDE, json!({}));
    assert!(validate_three_d_in(Some(&looped), &n, EXTRUDE, &json!({})).is_err());
    assert!(solid_art_in(Some(&looped), &n).is_none());
}

#[test]
fn a_blends_solid_is_built_from_its_keys_and_steps() {
    let d = document();
    for steps in [1, 3, 8] {
        let n = with(blend(20, steps), ROTATE, front());
        let expanded = Node::group(NodeId(20), crate::expand_live(Some(&d), &blend(20, steps)).into_iter().map(Arc::new).collect());
        assert_eq!(expanded.children().unwrap().len(), steps as usize + 2);
        let same = with(expanded, ROTATE, front());
        let (a, b) = (solid_art_in(Some(&d), &n).unwrap(), solid_art(&same).unwrap());
        assert_eq!(a.children().unwrap().len(), b.children().unwrap().len(), "a surface for each key and step");
        assert!(close(bounds(&a), bounds(&b), 0.01));
    }
    // A blend making more objects than a 3D effect takes is refused.
    let keys = (0..6).map(|i| Arc::new(rect(21 + i, 100.0 * i as f64, 0.0, 100.0 * i as f64 + 40.0, 40.0, 0.0))).collect();
    let spec = BlendSpec { spacing: BlendSpacing::Steps(1000), ..Default::default() };
    let n = with(Node::new(NodeId(20), NodeKind::Blend { children: keys, spec: Box::new(spec) }), ROTATE, front());
    assert!(validate_three_d_in(Some(&d), &n, ROTATE, &json!({})).is_err_and(|e| e.contains("at most 4096 objects")));
}

#[test]
fn images_are_refused_alone_and_in_a_group_with_image_trace_named() {
    let d = document();
    for id in TYPES {
        for n in [
            with(image(50), id, json!({})),
            with(Node::group(NodeId(51), vec![Arc::new(rect(52, 0.0, 0.0, 10.0, 10.0, 0.0)), Arc::new(image(53))]), id, json!({})),
            with(clip_group(60, Rect::new(0.0, 0.0, 10.0, 10.0), image(62)), id, json!({})),
        ] {
            let err = validate_three_d_in(Some(&d), &n, id, &json!({})).unwrap_err();
            assert!(err.contains("Object › Image Trace › Make"), "{id}: {err}");
            assert!(art(&d, &n, id).is_none(), "{id}");
            assert!(three_d_meshes_in(Some(&d), &n).is_err_and(|e| e.contains("Image Trace")), "{id}");
        }
        // A hidden image is not drawn, so it builds nothing and refuses nothing.
        let mut hidden = image(53);
        hidden.visible = false;
        let g = with(Node::group(NodeId(51), vec![Arc::new(rect(52, 0.0, 0.0, 10.0, 10.0, 0.0)), Arc::new(hidden)]), id, json!({}));
        assert_eq!(validate_three_d_in(Some(&d), &g, id, &json!({})), Ok(()), "{id}");
    }
}
