//! What live-effect plug-ins show besides their geometry ([`plugin_overlays`]): the overlays of the
//! effects drawing runs, in the order it runs them, from the plug-ins' cache only, and never in
//! what the renderer draws; and the objects drawing draws with their plug-ins
//! ([`plugin_effect_objects`]).

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Effect, Node, NodeKind};
use vectorcraft_geom::{PathData, Point, SubPath};
use vectorcraft_plugins::overlay::{FORMAT, Line, Overlay};
use vectorcraft_plugins::wat as tpl;

use super::*;

/// An overlay of one line from (`x`, 10) to (`x`, 90), in `color`, 2 pt wide.
fn line_at(x: f32, color: [u8; 4]) -> Overlay {
    Overlay { lines: vec![Line { color, width: 2.0, points: vec![[x, 10.0], [x, 90.0]] }] }
}

/// Installs live-effect plug-in `id`, which shows `overlay` and keeps the geometry it is given;
/// → its effect record.
fn showing(id: &str, overlay: &Overlay) -> Effect {
    let output = json!({"overlay": {"format": FORMAT, "data": vectorcraft_doc::base64::encode(&overlay.encode())}});
    let module = wat::parse_str(tpl::returning(&tpl::manifest(id, "effect", "{}"), &output.to_string())).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
    plugin_effect(id)
}

/// The effect record of live-effect plug-in `id`.
fn plugin_effect(id: &str) -> Effect {
    Effect { id: vectorcraft_plugins::effect::effect_id(id), params: json!({}), visible: true }
}

/// A red 20 × 20 square at (10, 10) with no stroke.
fn square() -> Node {
    let corners = [Point::new(10.0, 10.0), Point::new(30.0, 10.0), Point::new(30.0, 30.0), Point::new(10.0, 30.0)];
    Node::path(
        NodeId(1),
        PathData::single(SubPath::polyline(&corners, true)),
        Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0),
    )
}

/// What `n` shows once a document that holds it has been drawn: its overlays, as they come.
fn drawn_overlays(n: &Node) -> Option<Vec<Overlay>> {
    let mut d = Document::new(100.0, 100.0);
    let (layer, mut n) = (d.layers[0].id, n.clone());
    n.id = d.alloc_id();
    let id = d.insert(Some(layer), 0, n).unwrap();
    Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default());
    Some(plugin_overlays(d.node(id)?)?.iter().map(|o| (**o).clone()).collect())
}

/// The overlays come in the order drawing runs the effects: the object's, then each painting
/// fill's. Hidden effects, the effects of a fill with no paint, and guides show none.
#[test]
fn overlays_come_in_the_order_drawing_runs_the_effects() {
    let (a, b, c) = (line_at(50.0, [0, 0, 255, 255]), line_at(60.0, [0, 255, 0, 255]), line_at(70.0, [255, 0, 255, 255]));
    let mut n = square();
    assert_eq!(plugin_overlays(&n), Some(vec![]), "no plug-in effect, no overlay");
    n.appearance.effects = vec![showing("test.render.overlay.a", &a)];
    // The fill's own effects run after the object's, on the object's geometry.
    if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
        f.effects = vec![showing("test.render.overlay.b", &b)];
    }
    assert_eq!(drawn_overlays(&n), Some(vec![a.clone(), b.clone()]));
    // A hidden effect, and the effects of a fill that doesn't paint, don't run when it is drawn.
    n.appearance.effects.push(Effect { visible: false, ..showing("test.render.overlay.c", &c) });
    assert_eq!(drawn_overlays(&n), Some(vec![a.clone(), b.clone()]));
    if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
        f.paint = Paint::None;
    }
    assert_eq!(drawn_overlays(&n), Some(vec![a.clone()]));
    // Guides are drawn without their effects; other objects show none.
    let mut guide = n.clone();
    if let NodeKind::Path { guide: g, .. } = &mut guide.kind {
        *g = true;
    }
    assert_eq!(plugin_overlays(&guide), Some(vec![]));
    for id in ["test.render.overlay.a", "test.render.overlay.b", "test.render.overlay.c"] {
        vectorcraft_plugins::registry::remove(id);
    }
}

/// The overlays are those of the runs drawing made: before the object is drawn there are none to
/// show, and looking for them runs no plug-in.
#[test]
fn overlays_come_from_drawing_only() {
    let a = line_at(40.0, [0, 0, 255, 255]);
    let mut n = square();
    n.appearance.effects = vec![showing("test.render.overlay.drawing", &a)];
    assert_eq!(plugin_overlays(&n), None, "not drawn yet");
    assert_eq!(plugin_overlays(&n), None, "looking ran nothing");
    assert_eq!(drawn_overlays(&n), Some(vec![a]));
    vectorcraft_plugins::registry::remove("test.render.overlay.drawing");
}

/// The renderer draws the geometry, which an overlay alone leaves as it was, and never the
/// overlay: exports, prints and thumbnails all go through it.
#[test]
fn the_renderer_draws_no_overlay() {
    let mut d = Document::new(100.0, 100.0);
    let mut n = square();
    n.id = d.alloc_id();
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, n.clone()).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let before = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts);
    let fx = showing("test.render.overlay.drawn", &line_at(50.0, [0, 0, 255, 255]));
    let id = n.id;
    d.node_mut(id).unwrap().appearance.effects = vec![fx];
    let after = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts);
    assert_eq!(after.pixel(50, 50), [255, 255, 255, 255], "no line where the overlay is");
    assert_eq!(after.pixels, before.pixels, "the square as it was");
    assert_eq!(plugin_overlays(d.node(id).unwrap()).map(|o| o.len()), Some(1));
    vectorcraft_plugins::registry::remove("test.render.overlay.drawn");
}

/// The objects drawn with their plug-ins are those drawing draws with effects: not in a hidden
/// layer, a template layer left out, or a layer drawn in outline (Preview off, or Outline view);
/// in Outline view, those of a template layer, which stays in Preview. Exports, which leave
/// templates out, draw the other layers as they are.
#[test]
fn the_objects_drawn_with_their_plug_ins() {
    let mut d = Document::new(100.0, 100.0);
    let layers = [d.layers[0].id, d.add_layer(Some("Preview off")), d.add_layer(Some("Template")), d.add_layer(Some("Hidden"))];
    // A square with the plug-in's effect, on top of `parent`'s contents.
    let add = |d: &mut Document, parent: NodeId| {
        let mut n = square();
        n.id = d.alloc_id();
        n.appearance.effects = vec![plugin_effect("test.render.objects")];
        d.insert(Some(parent), usize::MAX, n).unwrap()
    };
    let a = add(&mut d, layers[0]);
    let hidden = add(&mut d, layers[0]);
    d.node_mut(hidden).unwrap().visible = false;
    let group = d.alloc_id();
    d.insert(Some(layers[0]), usize::MAX, Node::group(group, vec![])).unwrap();
    let in_group = add(&mut d, group);
    let [b, c, _] = [1, 2, 3].map(|i| add(&mut d, layers[i]));
    if let NodeKind::Layer { preview, .. } = &mut d.node_mut(layers[1]).unwrap().kind {
        *preview = false;
    }
    if let NodeKind::Layer { template, .. } = &mut d.node_mut(layers[2]).unwrap().kind {
        *template = true;
    }
    d.node_mut(layers[3]).unwrap().visible = false;
    let objects = |opts: RenderOptions| plugin_effect_objects(&d, &opts).iter().map(|n| n.id).collect::<Vec<_>>();
    assert_eq!(objects(RenderOptions::default()), [a, in_group, c], "in stacking order");
    assert_eq!(objects(RenderOptions { hide_templates: true, ..Default::default() }), [a, in_group], "Hide Template");
    assert_eq!(objects(RenderOptions { outline: true, ..Default::default() }), [c], "Outline view");
    assert_eq!(objects(RenderOptions { skip_templates: true, ..Default::default() }), [a, in_group, b], "an export");
}
