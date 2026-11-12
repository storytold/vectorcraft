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

/// An SVG filter keeps its options: it paints in its own user space.
#[test]
fn an_svg_filter_before_a_transform_keeps_its_options() {
    let filter = fx(crate::SVG_FILTER, json!({"origin": [10, 10]}));
    let effects = [filter.clone(), transform(json!({"scaleH": 200, "scaleV": 200, "rotate": 90}))];
    assert_eq!(crate::raster::followed(&effects)[0], filter);
    assert_eq!(crate::raster::without_geometry(&effects), vec![filter]);
}

/// The offset of every Drop Shadow drawn on `n` and the nodes under it, from the object's effects
/// and from each fill's and stroke's.
fn shadows_in(n: &Node) -> Vec<(f64, f64)> {
    let mut out = vec![];
    n.walk(&mut |m| {
        let lists = std::iter::once(&m.appearance.effects).chain(m.appearance.items.iter().map(|i| i.effects()));
        for list in lists {
            for r in raster_effects(list) {
                if let RasterFx::DropShadow { dx, dy, .. } = r {
                    out.push((dx, dy));
                }
            }
        }
    });
    out
}

fn near(a: &[(f64, f64)], b: &[(f64, f64)]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(p, q)| (p.0 - q.0).abs() < 1e-9 && (p.1 - q.1).abs() < 1e-9)
}

/// A square whose fill carries `effects` of its own.
fn fill_square(id: u64, effects: Vec<Effect>) -> Node {
    let mut n = square(id, vec![]);
    if let Some(item) = n.appearance.items.first_mut() {
        *item.effects_mut() = effects;
    }
    n
}

/// A fill's own Transform effect transforms the fill's raster effects before it: drawn as they are
/// (no options), drawn through the art the Transform Patterns or Scale Strokes & Effects option
/// makes, expanded and baked. Scale Strokes & Effects does not scale a shadow that follows the
/// Transform effect a second time.
#[test]
fn a_fills_shadow_follows_the_fills_own_transform() {
    let cases = [
        (json!({"rotate": 90}), (0.0, -10.0)),
        (json!({"rotate": 90, "strokes": true}), (0.0, -10.0)),
        (json!({"scaleH": 200, "scaleV": 200, "strokes": true}), (20.0, 0.0)),
        (json!({"scaleH": 200, "scaleV": 200, "patterns": true}), (20.0, 0.0)),
        (json!({"reflectX": true, "patterns": true}), (-10.0, 0.0)),
    ];
    for (params, want) in cases {
        let n = fill_square(11, vec![shadow(10.0, 0.0, 0.0), transform(params.clone())]);
        // Drawn and expanded through its art when an option makes some.
        if crate::has_transform_art(&n) {
            let art = crate::reshape(None, &n, None).unwrap();
            assert!(near(&shadows_in(&art), &[want]), "{params}: drawn {:?}", shadows_in(&art));
            let expanded = expand_outlined(None, &n, None).unwrap();
            assert!(near(&shadows_in(&expanded), &[want]), "{params}: expanded {:?}", shadows_in(&expanded));
        } else {
            assert!(near(&shadows_in(&n), &[want]), "{params}: drawn {:?}", shadows_in(&n));
        }
        let mut d = Document::new(800.0, 800.0);
        let layer = d.layers[0].id;
        d.insert(Some(layer), 0, n).unwrap();
        let baked = bake_document(&d).unwrap();
        let b = baked.node(NodeId(11)).unwrap();
        assert!(near(&shadows_in(b), &[want]), "{params}: baked {:?}", shadows_in(b));
    }
}

/// A Transform effect after a fill's own Transform effect with Scale Strokes & Effects on turns the
/// shadow between them, which the first one scaled; a Transform effect that moves at random holds
/// the shadow before it back from the Transform effects after it.
#[test]
fn a_fills_later_transform_turns_the_shadow_its_options_scaled() {
    let n = fill_square(
        12,
        vec![transform(json!({"scaleH": 200, "scaleV": 200, "strokes": true})), shadow(10.0, 0.0, 0.0), transform(json!({"rotate": 90}))],
    );
    let art = crate::reshape(None, &n, None).unwrap();
    assert!(near(&shadows_in(&art), &[(0.0, -20.0)]), "{:?}", shadows_in(&art));
    let held = fill_square(
        13,
        vec![
            shadow(10.0, 0.0, 0.0),
            transform(json!({"scaleH": 200, "scaleV": 200, "random": true, "strokes": true})),
            transform(json!({"rotate": 90})),
        ],
    );
    let art = crate::reshape(None, &held, None).unwrap();
    let drawn = shadows_in(&art);
    assert!(drawn.len() == 1 && drawn[0].0 > 0.0 && drawn[0].1.abs() < 1e-9, "{drawn:?}");
}

/// The 3D effects keep the shadow before a Transform effect that shapes the art they project, the
/// object's and its fill's 3D effect alike.
#[test]
fn a_shadow_before_a_transform_follows_it_into_3d_art() {
    let before = vec![shadow(10.0, 0.0, 0.0), transform(json!({"scaleH": 200, "scaleV": 200}))];
    for three_d in [crate::EXTRUDE, crate::REVOLVE] {
        let art = |n: &Node| if three_d == crate::EXTRUDE { crate::solid_art(n) } else { crate::revolve_art(n) };
        let mut on_object = before.clone();
        on_object.push(fx(three_d, json!({})));
        let projected = art(&square(14, on_object)).unwrap();
        assert!(near(&shadows_in(&projected), &[(20.0, 0.0)]), "{three_d} on the object: {:?}", shadows_in(&projected));
        let mut on_fill = square(15, before.clone());
        if let Some(item) = on_fill.appearance.items.first_mut() {
            *item.effects_mut() = vec![fx(three_d, json!({}))];
        }
        let projected = art(&on_fill).unwrap();
        assert!(near(&shadows_in(&projected), &[(20.0, 0.0)]), "{three_d} on the fill: {:?}", shadows_in(&projected));
    }
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
fn pixel_filter_sizes_scale_with_the_transform_after_them() {
    let effects = [
        fx("artistic.coloredPencil", json!({"pencilWidth": 5})),
        fx("artistic.paintDaubs", json!({"brushSize": 10})),
        transform(json!({"scaleH": 200, "scaleV": 200})),
    ];
    let drawn = raster_effects(&effects);
    assert!(matches!(drawn.first(), Some(RasterFx::Pixel(crate::PixelFx::ColoredPencil { width, .. })) if (width - 10.0).abs() < 1e-9), "{drawn:?}");
    assert!(matches!(drawn.get(1), Some(RasterFx::Pixel(crate::PixelFx::PaintDaubs { size, .. })) if (size - 20.0).abs() < 1e-9), "{drawn:?}");
}

/// A Transform effect after a 3D effect that makes copies or moves at random holds back the shadow
/// before the 3D effect, as it does without the 3D effect.
#[test]
fn a_copying_transform_after_3d_holds_the_shadow_back() {
    for later in [json!({"copies": 1}), json!({"scaleH": 150, "scaleV": 150, "random": true})] {
        let plain = [shadow(10.0, 0.0, 0.0), transform(json!({"scaleH": 200, "scaleV": 200})), transform(later.clone())];
        assert!(close(drawn_shadow(&plain), (10.0, 0.0, 0.0)), "{later}: {:?}", drawn_shadow(&plain));
        for three_d in [crate::EXTRUDE, crate::REVOLVE] {
            let art = |n: &Node| if three_d == crate::EXTRUDE { crate::solid_art(n) } else { crate::revolve_art(n) };
            let effects = vec![plain[0].clone(), plain[1].clone(), fx(three_d, json!({})), plain[2].clone()];
            let projected = art(&square(16, effects)).unwrap();
            let drawn = shadows_in(&projected);
            assert!(!drawn.is_empty() && near(&drawn, &vec![(10.0, 0.0); drawn.len()]), "{three_d} {later}: {drawn:?}");
        }
    }
}

/// In a fill's own effects, a Transform effect that makes copies after a Transform effect with
/// Transform Patterns on holds back the shadow before both, and before a Transform effect between
/// them.
#[test]
fn a_fills_copying_transform_holds_back_the_shadow_before_an_earlier_transform() {
    let patterns = transform(json!({"scaleH": 200, "scaleV": 200, "patterns": true}));
    let copies = transform(json!({"copies": 1}));
    let lists = [
        vec![shadow(10.0, 0.0, 0.0), patterns.clone(), copies.clone()],
        vec![shadow(10.0, 0.0, 0.0), patterns, transform(json!({"rotate": 90})), copies],
    ];
    for effects in lists {
        let n = fill_square(17, effects.clone());
        let art = crate::reshape(None, &n, None).unwrap();
        let drawn = shadows_in(&art);
        assert!(!drawn.is_empty() && near(&drawn, &vec![(10.0, 0.0); drawn.len()]), "{} effects: {drawn:?}", effects.len());
    }
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
