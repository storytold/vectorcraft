//! Targeting through the Layers panel's target circles (`layer.target`) and moving or copying
//! appearances between objects (`appearance.transfer`).

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::Node;

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 20, "width": 40, "height": 40}))["id"].as_u64().unwrap())
}

#[test]
fn a_targeted_layer_takes_transparency_appearance_and_effects() {
    let mut s = session();
    let a = rect(&mut s, 20.0);
    let b = rect(&mut s, 100.0);
    let layer = s.doc().unwrap().doc.layers[0].id;
    let r = run(&mut s, "layer.target", json!({"id": layer.0}));
    assert_eq!(r["selected"], json!([a.0, b.0]), "the layer's art is selected");
    assert_eq!(s.doc().unwrap().selection.target, Some(layer));
    run(&mut s, "transparency.set", json!({"opacity": 40, "blend": "multiply"}));
    let l = node(&s, layer);
    assert!((l.opacity - 0.4).abs() < 1e-6 && l.blend == BlendMode::Multiply);
    assert_eq!(node(&s, a).opacity, 1.0, "the art keeps its own opacity");
    assert_eq!(s.execute("transparency.info", &json!({})).unwrap()["ids"], json!([layer.0]));
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow"}));
    let l = node(&s, layer);
    assert_eq!(l.appearance.items.len(), 1);
    assert_eq!(l.appearance.effects.len(), 1);
    // The Appearance panel's active row is a row of the layer's own stack.
    run(&mut s, "appearance.setActiveItem", json!({"index": 0}));
    assert_eq!(s.appearance_item(), Some(0));
    // Undo is one step per command, and any other selection change ends the targeting.
    run(&mut s, "edit.undo", json!({}));
    assert!(node(&s, layer).appearance.effects.is_empty());
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    assert_eq!(s.doc().unwrap().selection.target, None);
    assert_eq!(s.appearance_item(), None);
    run(&mut s, "transparency.set", json!({"opacity": 50}));
    assert!((node(&s, a).opacity - 0.5).abs() < 1e-6);
    assert!((node(&s, layer).opacity - 0.4).abs() < 1e-6);
}

#[test]
fn targeting_an_object_selects_it_and_locked_layers_cannot_be_targeted() {
    let mut s = session();
    let a = rect(&mut s, 20.0);
    run(&mut s, "select.none", json!({}));
    run(&mut s, "layer.target", json!({"id": a.0}));
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    let layer = s.doc().unwrap().doc.layers[0].id;
    run(&mut s, "layer.setProps", json!({"id": layer.0, "locked": true}));
    assert!(s.execute("layer.target", &json!({"id": layer.0})).is_err());
    assert!(s.execute("layer.target", &json!({"id": 9999})).is_err());
}

#[test]
fn a_targeted_layer_gets_an_opacity_mask() {
    let mut s = session();
    rect(&mut s, 20.0);
    let layer = s.doc().unwrap().doc.layers[0].id;
    run(&mut s, "layer.target", json!({"id": layer.0}));
    assert_eq!(run(&mut s, "transparency.makeOpacityMask", json!({}))["id"], json!(layer.0));
    run(&mut s, "transparency.stopEditingOpacityMask", json!({}));
    assert!(node(&s, layer).extras().mask.is_some());
    assert_eq!(s.doc().unwrap().selection.target, Some(layer), "the masked layer stays targeted");
}

#[test]
fn transfer_moves_or_copies_the_appearance() {
    let mut s = session();
    let a = rect(&mut s, 20.0);
    let b = rect(&mut s, 100.0);
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "effect.apply", json!({"effect": "distort.twist"}));
    run(&mut s, "transparency.set", json!({"opacity": 30, "blend": "screen"}));
    let src = node(&s, a);
    // Copy (Alt-drag): both have it.
    run(&mut s, "appearance.transfer", json!({"source": a.0, "target": b.0, "copy": true}));
    let (na, nb) = (node(&s, a), node(&s, b));
    assert_eq!(na.appearance, src.appearance);
    assert_eq!(nb.appearance, src.appearance);
    assert!((nb.opacity - 0.3).abs() < 1e-6 && nb.blend == BlendMode::Screen);
    // Move onto the layer: the source is left cleared; one undo step brings it back.
    let layer = s.doc().unwrap().doc.layers[0].id;
    run(&mut s, "appearance.transfer", json!({"source": a.0, "target": layer.0}));
    let (na, nl) = (node(&s, a), node(&s, layer));
    assert_eq!(nl.appearance.items.len(), src.appearance.items.len());
    assert_eq!(nl.appearance.effects.len(), 1);
    assert!((nl.opacity - 0.3).abs() < 1e-6);
    assert!(na.appearance.effects.is_empty() && na.opacity == 1.0 && na.blend == BlendMode::Normal);
    assert_eq!(na.appearance.fill_paint(), Paint::None);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, a).appearance, src.appearance);
    assert!(node(&s, layer).appearance.items.is_empty());
    assert!(s.execute("appearance.transfer", &json!({"source": a.0, "target": a.0})).is_err());
    assert!(s.execute("appearance.transfer", &json!({"source": a.0})).is_err());
}
