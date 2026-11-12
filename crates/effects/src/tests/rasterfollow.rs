//! Raster effects listed before a Transform effect follow it: the Transform effect transforms
//! what they paint.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Effect, Node, NodeId};
use vectorcraft_geom::{Rect, shapes};

use crate::{RasterFx, bake_document, expand_outlined, raster_effects};

fn fx(id: &str, params: Value) -> Effect {
    Effect { id: id.into(), params, visible: true }
}

fn shadow(x: f64, y: f64, blur: f64) -> Effect {
    fx("stylize.dropShadow", json!({"x": x, "y": y, "blur": blur}))
}

fn transform(params: Value) -> Effect {
    fx("distort.transform", params)
}

/// The offset and blur of the first Drop Shadow `effects` draw.
fn drawn_shadow(effects: &[Effect]) -> (f64, f64, f64) {
    match raster_effects(effects).first() {
        Some(RasterFx::DropShadow { dx, dy, blur, .. }) => (*dx, *dy, *blur),
        other => panic!("expected a drop shadow, got {other:?}"),
    }
}

fn close(a: (f64, f64, f64), b: (f64, f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9 && (a.2 - b.2).abs() < 1e-9
}

/// A 50 pt red square with `effects`.
fn square(id: u64, effects: Vec<Effect>) -> Node {
    let mut n = Node::path(
        NodeId(id),
        shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)),
        Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0),
    );
    n.appearance.effects = effects;
    n
}

#[test]
fn a_transform_after_a_drop_shadow_scales_its_offset_and_blur() {
    for strokes in [false, true] {
        let effects = [shadow(10.0, 10.0, 4.0), transform(json!({"scaleH": 200, "scaleV": 200, "strokes": strokes}))];
        assert!(close(drawn_shadow(&effects), (20.0, 20.0, 8.0)), "Scale Strokes & Effects {strokes}: {:?}", drawn_shadow(&effects));
    }
}

#[test]
fn a_drop_shadow_after_a_transform_keeps_its_offset() {
    let effects = [transform(json!({"scaleH": 200, "scaleV": 200, "strokes": true})), shadow(10.0, 10.0, 4.0)];
    assert!(close(drawn_shadow(&effects), (10.0, 10.0, 4.0)), "{:?}", drawn_shadow(&effects));
}

#[test]
fn the_offset_follows_an_uneven_scale_a_rotation_and_a_reflection() {
    let uneven = [shadow(10.0, 10.0, 4.0), transform(json!({"scaleH": 200, "scaleV": 100}))];
    assert!(close(drawn_shadow(&uneven), (20.0, 10.0, 4.0 * 2f64.sqrt())), "{:?}", drawn_shadow(&uneven));
    // A turn of 90° counterclockwise on screen takes an offset to the right to one upward.
    let turned = [shadow(10.0, 0.0, 4.0), transform(json!({"rotate": 90}))];
    let (dx, dy, blur) = drawn_shadow(&turned);
    assert!(dx.abs() < 1e-9 && (dy + 10.0).abs() < 1e-9 && (blur - 4.0).abs() < 1e-9, "{dx} {dy} {blur}");
    let reflected = [shadow(10.0, 0.0, 4.0), transform(json!({"reflectX": true}))];
    assert!(close(drawn_shadow(&reflected), (-10.0, 0.0, 4.0)), "{:?}", drawn_shadow(&reflected));
}

#[test]
fn transforms_after_the_shadow_apply_in_order() {
    let effects = [shadow(10.0, 0.0, 0.0), transform(json!({"scaleH": 200, "scaleV": 200})), transform(json!({"rotate": 90}))];
    let (dx, dy, _) = drawn_shadow(&effects);
    assert!(dx.abs() < 1e-9 && (dy + 20.0).abs() < 1e-9, "{dx} {dy}");
    // Only the Transform after a shadow moves it.
    let between = [shadow(10.0, 0.0, 0.0), transform(json!({"scaleH": 200, "scaleV": 200})), shadow(0.0, 10.0, 0.0)];
    let drawn = raster_effects(&between);
    let offsets: Vec<(f64, f64)> = drawn
        .iter()
        .filter_map(|r| match r {
            RasterFx::DropShadow { dx, dy, .. } => Some((*dx, *dy)),
            _ => None,
        })
        .collect();
    assert_eq!(offsets, vec![(20.0, 0.0), (0.0, 10.0)]);
}

#[test]
fn transforms_that_leave_the_art_or_copy_it_leave_the_shadow() {
    for params in [
        json!({"scaleH": 200, "scaleV": 200, "objects": false, "strokes": true}),
        json!({"scaleH": 200, "scaleV": 200, "copies": 1}),
        json!({"scaleH": 200, "scaleV": 200, "random": true}),
    ] {
        let effects = [shadow(10.0, 10.0, 4.0), transform(params.clone())];
        assert!(close(drawn_shadow(&effects), (10.0, 10.0, 4.0)), "{params}: {:?}", drawn_shadow(&effects));
    }
    let mut hidden = transform(json!({"scaleH": 200, "scaleV": 200}));
    hidden.visible = false;
    let effects = [shadow(10.0, 10.0, 4.0), hidden];
    assert!(close(drawn_shadow(&effects), (10.0, 10.0, 4.0)), "{:?}", drawn_shadow(&effects));
}

/// A hidden shadow kept when the Transform effect is applied (Expand Appearance) draws as it would
/// have before, once it is shown.
#[test]
fn a_hidden_shadow_follows_the_transform_it_loses() {
    let mut hidden = shadow(10.0, 10.0, 4.0);
    hidden.visible = false;
    let kept = crate::raster::without_geometry(&[hidden, transform(json!({"scaleH": 200, "scaleV": 200}))]);
    assert_eq!(kept.len(), 1);
    let mut shown = kept[0].clone();
    shown.visible = true;
    assert!(close(drawn_shadow(&[shown]), (20.0, 20.0, 8.0)));
}

/// Rasterize keeps its room around the art: its image keeps its own pixel grid.
#[test]
fn rasterize_before_a_transform_keeps_its_options() {
    let rasterize = fx(crate::RASTERIZE, json!({"addAround": 6}));
    let effects = [rasterize.clone(), transform(json!({"scaleH": 200, "scaleV": 200}))];
    assert_eq!(crate::raster::followed(&effects)[0], rasterize);
    assert_eq!(crate::raster::without_geometry(&effects), vec![rasterize]);
}

#[test]
fn glows_blurs_and_feathers_scale_with_the_transform_after_them() {
    let effects = [
        fx("stylize.outerGlow", json!({"blur": 5})),
        fx("stylize.innerGlow", json!({"blur": 5})),
        fx("stylize.feather", json!({"radius": 5})),
        fx("blur.gaussian", json!({"radius": 5})),
        transform(json!({"scaleH": 50, "scaleV": 50})),
    ];
    let sizes: Vec<f64> = raster_effects(&effects)
        .iter()
        .map(|r| match r {
            RasterFx::OuterGlow { blur, .. } | RasterFx::InnerGlow { blur, .. } => *blur,
            RasterFx::Feather { radius } | RasterFx::GaussianBlur { radius, .. } => *radius,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(sizes, vec![2.5; 4]);
    // How far the shadow paints past the object follows too.
    let shadowed = [shadow(10.0, 10.0, 0.0), transform(json!({"scaleH": 200, "scaleV": 200}))];
    assert_eq!(crate::outset(&shadowed, Rect::new(0.0, 0.0, 100.0, 100.0)), 20.0);
}

#[test]
fn expanded_and_exported_objects_keep_the_shadow_the_canvas_draws() {
    let effects = vec![shadow(10.0, 10.0, 4.0), transform(json!({"scaleH": 200, "scaleV": 200}))];
    // A path drawn through its art (Scale Strokes & Effects on), expanded.
    let mut with_options = effects.clone();
    with_options[1] = transform(json!({"scaleH": 200, "scaleV": 200, "strokes": true}));
    let n = square(7, with_options);
    let expanded = expand_outlined(None, &n, None).unwrap();
    assert!(close(drawn_shadow(&expanded.appearance.effects), (20.0, 20.0, 8.0)), "{:?}", expanded.appearance.effects);
    // A path and a group, baked for export.
    let mut d = Document::new(800.0, 800.0);
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, square(8, effects.clone())).unwrap();
    let mut g = Node::group(NodeId(9), vec![std::sync::Arc::new(square(10, vec![]))]);
    g.appearance.effects = effects.clone();
    d.insert(Some(layer), 1, g).unwrap();
    let baked = bake_document(&d).unwrap();
    for id in [8, 9] {
        let b = baked.node(NodeId(id)).unwrap();
        assert!(!b.appearance.effects.iter().any(|e| e.id == "distort.transform"), "{id}: {:?}", b.appearance.effects);
        assert!(close(drawn_shadow(&b.appearance.effects), (20.0, 20.0, 8.0)), "{id}: {:?}", b.appearance.effects);
    }
}
