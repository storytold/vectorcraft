//! Raster effects listed before a Transform effect follow it: the Transform effect transforms
//! what they paint.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Effect, Node, NodeId};
use vectorcraft_geom::{Affine, Anchor, PathData, Point, Rect, SubPath, shapes};

use crate::geometry::{Budget, MAX_EFFECT_LINES, apply_geometry_carrying_within};
use crate::{Carry, GeomContext, RasterFx, apply_geometry_carrying, bake_document, expand_outlined, raster_effects};

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
/// shadow between them, which the first one leaves as it is; a Transform effect that moves at random
/// holds the shadow before it back from the Transform effects after it.
#[test]
fn a_fills_later_transform_turns_the_shadow_after_its_first() {
    let n = fill_square(
        12,
        vec![transform(json!({"scaleH": 200, "scaleV": 200, "strokes": true})), shadow(10.0, 0.0, 0.0), transform(json!({"rotate": 90}))],
    );
    let art = crate::reshape(None, &n, None).unwrap();
    assert!(near(&shadows_in(&art), &[(0.0, -10.0)]), "{:?}", shadows_in(&art));
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

/// A fill's own Transform effect with Transform Patterns or Scale Strokes & Effects on leaves the
/// fill's raster effects after it as they are: they paint around the transformed art, its copies
/// included. With Transform Objects off it leaves the raster effects before it as they are too.
/// Drawn, expanded and baked alike.
#[test]
fn a_fills_own_transform_leaves_the_shadow_after_it() {
    let cases = [
        vec![transform(json!({"scaleH": 200, "scaleV": 200, "strokes": true})), shadow(10.0, 0.0, 0.0)],
        vec![transform(json!({"scaleH": 200, "scaleV": 200, "patterns": true, "strokes": true})), shadow(10.0, 0.0, 0.0)],
        vec![transform(json!({"scaleH": 200, "scaleV": 200, "moveH": 120, "copies": 1, "strokes": true})), shadow(10.0, 0.0, 0.0)],
        vec![transform(json!({"scaleH": 200, "scaleV": 200, "objects": false, "strokes": true})), shadow(10.0, 0.0, 0.0)],
        vec![shadow(10.0, 0.0, 0.0), transform(json!({"scaleH": 200, "scaleV": 200, "objects": false, "strokes": true}))],
        // A later Transform effect with an option leaves the shadow after it as well.
        vec![
            transform(json!({"scaleH": 200, "scaleV": 200, "patterns": true})),
            transform(json!({"scaleH": 150, "scaleV": 150, "strokes": true})),
            shadow(10.0, 0.0, 0.0),
        ],
    ];
    for effects in cases {
        let n = fill_square(18, effects.clone());
        let art = crate::reshape(None, &n, None).unwrap();
        let drawn = shadows_in(&art);
        assert!(!drawn.is_empty() && near(&drawn, &vec![(10.0, 0.0); drawn.len()]), "{effects:?}: drawn {drawn:?}");
        let expanded = shadows_in(&expand_outlined(None, &n, None).unwrap());
        assert!(!expanded.is_empty() && near(&expanded, &vec![(10.0, 0.0); expanded.len()]), "{effects:?}: expanded {expanded:?}");
        let mut d = Document::new(800.0, 800.0);
        let layer = d.layers[0].id;
        d.insert(Some(layer), 0, n).unwrap();
        let baked = bake_document(&d).unwrap();
        let b = shadows_in(baked.node(NodeId(18)).unwrap());
        assert!(!b.is_empty() && near(&b, &vec![(10.0, 0.0); b.len()]), "{effects:?}: baked {b:?}");
    }
}

/// A geometry effect after a fill's own Transform effect with Scale Strokes & Effects on reshapes
/// the transformed art by its own distance, as it does after an object's Transform effect.
#[test]
fn a_fills_own_transform_leaves_the_offset_after_it() {
    let offset = fx("path.offsetPath", json!({"offset": 5}));
    let scaled = transform(json!({"scaleH": 200, "scaleV": 200, "strokes": true}));
    let n = fill_square(19, vec![scaled.clone(), offset.clone()]);
    let on_fill = crate::reshape(None, &n, None).unwrap();
    let on_object = crate::reshape(None, &square(20, vec![scaled, offset]), None).unwrap();
    let (fill, object) = (on_fill.geometric_bounds().unwrap(), on_object.geometric_bounds().unwrap());
    // The 50 pt square scaled to 100 pt about its center, then offset by 5 pt on each side.
    assert!((fill.width() - 110.0).abs() < 1e-6 && (fill.height() - 110.0).abs() < 1e-6, "{fill:?}");
    assert!((fill.width() - object.width()).abs() < 1e-6 && (fill.x0 - object.x0).abs() < 1e-6, "{fill:?} {object:?}");
    let expanded = expand_outlined(None, &n, None).unwrap().geometric_bounds().unwrap();
    assert!((expanded.width() - 110.0).abs() < 1e-6 && (expanded.x0 - fill.x0).abs() < 1e-6, "expanded {expanded:?}");
}

/// The SVG filters on `n` and the nodes under it, from the object's effects and from each fill's
/// and stroke's.
fn filters_in(n: &Node) -> Vec<Effect> {
    let mut out = vec![];
    n.walk(&mut |m| {
        let lists = std::iter::once(&m.appearance.effects).chain(m.appearance.items.iter().map(|i| i.effects()));
        out.extend(lists.flatten().filter(|e| e.id == crate::SVG_FILTER).cloned());
    });
    out
}

/// An SVG filter after a fill's own Transform effect with Scale Strokes & Effects on keeps its
/// scale and the origin of its user space, as it does after the same Transform effect without
/// options and after an object's own Transform effect.
#[test]
fn a_fills_own_transform_leaves_the_svg_filter_after_it() {
    let filter = fx(crate::SVG_FILTER, json!({"scale": 1.0, "origin": [10.0, 10.0]}));
    for params in [
        json!({"scaleH": 200, "scaleV": 200, "moveH": 100, "strokes": true}),
        json!({"scaleH": 200, "scaleV": 200, "moveH": 100, "patterns": true}),
        json!({"scaleH": 200, "scaleV": 200, "moveH": 100, "copies": 1, "strokes": true}),
        json!({"scaleH": 200, "scaleV": 200, "objects": false, "strokes": true}),
    ] {
        let art = crate::reshape(None, &fill_square(21, vec![transform(params.clone()), filter.clone()]), None).unwrap();
        let drawn = filters_in(&art);
        assert!(!drawn.is_empty() && drawn.iter().all(|e| *e == filter), "{params}: {drawn:?}");
    }
    let object = square(22, vec![transform(json!({"scaleH": 200, "scaleV": 200, "moveH": 100, "strokes": true})), filter.clone()]);
    let art = crate::reshape(None, &object, None).unwrap();
    assert!(!filters_in(&art).is_empty() && filters_in(&art).iter().all(|e| *e == filter), "{:?}", filters_in(&art));
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

/// An object's Transform effect transforms the raster effects of its fills when only a fill has a
/// 3D effect: the shadow after that fill's 3D effect and another fill's shadow, as it does without
/// the 3D effect.
#[test]
fn an_objects_transform_moves_its_fills_shadows_beside_a_fills_3d_effect() {
    for three_d in [crate::EXTRUDE, crate::REVOLVE] {
        let art = |n: &Node| if three_d == crate::EXTRUDE { crate::solid_art(n) } else { crate::revolve_art(n) };
        let mut n = fill_square(18, vec![fx(three_d, json!({})), shadow(10.0, 10.0, 0.0)]);
        let mut other = n.appearance.items[0].clone();
        *other.effects_mut() = vec![shadow(10.0, 10.0, 0.0)];
        n.appearance.items.push(other);
        n.appearance.effects = vec![transform(json!({"scaleH": 200, "scaleV": 100}))];
        let drawn = shadows_in(&art(&n).unwrap());
        assert!(drawn.len() > 1 && near(&drawn, &vec![(20.0, 10.0); drawn.len()]), "{three_d}: {drawn:?}");
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

/// A group of `members` with `effects` of its own.
fn group(id: u64, members: Vec<Node>, effects: Vec<Effect>) -> Node {
    let mut g = Node::group(NodeId(id), members.into_iter().map(std::sync::Arc::new).collect());
    g.appearance.effects = effects;
    g
}

/// The node with `id` under `n`.
fn node_in(n: &Node, id: u64) -> Node {
    let mut found = None;
    n.walk(&mut |m| {
        if m.id == NodeId(id) && found.is_none() {
            found = Some(m.clone());
        }
    });
    found.unwrap_or_else(|| panic!("no node {id}"))
}

/// The offset of the Drop Shadow that the first fill of `n` draws, its own effects and `n`'s
/// Transform effects applied ([`crate::item_effects_in`]).
fn fill_shadow(n: &Node) -> (f64, f64) {
    let item = n.appearance.items.first().unwrap();
    match raster_effects(&crate::item_effects_in(&n.appearance.effects, item.effects())).first() {
        Some(RasterFx::DropShadow { dx, dy, .. }) => (*dx, *dy),
        other => panic!("expected a drop shadow, got {other:?}"),
    }
}

/// A group's Transform effect transforms the raster effects of what the group holds with the art,
/// whatever its options: a member's own shadow, a member's fill's shadow and a nested group's own
/// shadow, drawn (the group's art) and baked.
#[test]
fn a_groups_transform_moves_its_members_shadows() {
    let cases = [
        (json!({"scaleH": 200, "scaleV": 200}), (20.0, 20.0)),
        (json!({"scaleH": 200, "scaleV": 100, "strokes": true}), (20.0, 10.0)),
        (json!({"rotate": 90, "patterns": true}), (10.0, -10.0)),
        (json!({"rotate": 90, "strokes": true}), (10.0, -10.0)),
    ];
    for (params, want) in cases {
        let members = vec![
            square(21, vec![shadow(10.0, 10.0, 0.0)]),
            fill_square(22, vec![shadow(10.0, 10.0, 0.0)]),
            group(23, vec![square(24, vec![])], vec![shadow(10.0, 10.0, 0.0)]),
        ];
        let g = group(20, members, vec![transform(params.clone())]);
        let art = crate::evaluate_container(None, &g).unwrap();
        assert!(near(&shadows_in(&art), &[want; 3]), "{params}: drawn {:?}", shadows_in(&art));
        let mut d = Document::new(800.0, 800.0);
        let layer = d.layers[0].id;
        d.insert(Some(layer), 0, g).unwrap();
        let baked = bake_document(&d).unwrap();
        for id in [21, 22, 23] {
            let b = baked.node(NodeId(id)).unwrap();
            assert!(near(&shadows_in(b), &[want]), "{params}: {id} baked {:?}", shadows_in(b));
        }
    }
}

/// A group's Transform effect that makes copies or moves at random leaves its members' shadows as
/// they are, as it leaves the shadows listed before it in its own effects.
#[test]
fn a_groups_copying_transform_leaves_its_members_shadows() {
    for params in [json!({"scaleH": 200, "scaleV": 200, "copies": 1}), json!({"scaleH": 200, "scaleV": 200, "random": true})] {
        let g = group(25, vec![square(26, vec![shadow(10.0, 10.0, 0.0)]), square(27, vec![])], vec![transform(params.clone())]);
        let art = crate::evaluate_container(None, &g).unwrap();
        let drawn = shadows_in(&art);
        assert!(!drawn.is_empty() && near(&drawn, &vec![(10.0, 10.0); drawn.len()]), "{params}: {drawn:?}");
    }
}

/// The Transform effects inside a group apply before the group's: a member's own turn of 90°
/// applies to its shadow before the group's uneven scale does, for the member's own shadow, its
/// fill's shadow, and the shadow of a member of a nested group that has the turn.
#[test]
fn a_groups_transform_applies_after_its_members_own() {
    let turn = || transform(json!({"rotate": 90}));
    let uneven = || transform(json!({"scaleH": 200, "scaleV": 100}));
    // Turned first, (10, 0) goes to (0, -10), which the scale leaves; scaled first, it would be (0, -20).
    let want = (0.0, -10.0);
    let own = group(30, vec![square(31, vec![shadow(10.0, 0.0, 0.0), turn()])], vec![uneven()]);
    let art = crate::evaluate_container(None, &own).unwrap();
    assert!(near(&shadows_in(&art), &[want]), "own: {:?}", shadows_in(&art));
    let mut on_fill = fill_square(33, vec![shadow(10.0, 0.0, 0.0)]);
    on_fill.appearance.effects = vec![turn()];
    let art = crate::evaluate_container(None, &group(32, vec![on_fill], vec![uneven()])).unwrap();
    // The member is drawn as its art: a group with its id holding the turned square.
    let filled = node_in(&art, 33).children().and_then(|ch| ch.first().map(|c| (**c).clone())).unwrap();
    assert!(near(&[fill_shadow(&filled)], &[want]), "fill: {:?}", fill_shadow(&filled));
    let nested = group(34, vec![group(35, vec![square(36, vec![shadow(10.0, 0.0, 0.0)])], vec![turn()])], vec![uneven()]);
    let art = crate::evaluate_container(None, &nested).unwrap();
    assert!(near(&shadows_in(&art), &[want]), "nested: {:?}", shadows_in(&art));
}

/// An object's Transform effect transforms the raster effects of its fills and strokes with the
/// art, whatever its options: drawn, drawn through the art an option makes, and baked. One that
/// makes copies leaves them.
#[test]
fn an_objects_transform_moves_its_fills_shadows() {
    let cases = [
        (json!({"scaleH": 200, "scaleV": 100}), (20.0, 10.0)),
        (json!({"scaleH": 200, "scaleV": 100, "strokes": true}), (20.0, 10.0)),
        (json!({"rotate": 90, "patterns": true}), (10.0, -10.0)),
        (json!({"rotate": 90, "strokes": true}), (10.0, -10.0)),
        (json!({"scaleH": 200, "scaleV": 200, "copies": 1}), (10.0, 10.0)),
    ];
    for (params, want) in cases {
        let mut n = fill_square(40, vec![shadow(10.0, 10.0, 0.0)]);
        n.appearance.effects = vec![transform(params.clone())];
        if crate::has_transform_art(&n) {
            let art = crate::reshape(None, &n, None).unwrap();
            let drawn = shadows_in(&art);
            assert!(!drawn.is_empty() && near(&drawn, &vec![want; drawn.len()]), "{params}: drawn {drawn:?}");
        } else {
            assert!(near(&[fill_shadow(&n)], &[want]), "{params}: drawn {:?}", fill_shadow(&n));
        }
        let mut d = Document::new(800.0, 800.0);
        let layer = d.layers[0].id;
        d.insert(Some(layer), 0, n).unwrap();
        let baked = bake_document(&d).unwrap();
        let drawn = shadows_in(baked.node(NodeId(40)).unwrap());
        assert!(!drawn.is_empty() && near(&drawn, &vec![want; drawn.len()]), "{params}: baked {drawn:?}");
    }
}

/// A 10 pt image at the origin with `effects`.
fn image(id: u64, effects: Vec<Effect>) -> Node {
    let mut n = Node::new(
        NodeId(id),
        vectorcraft_doc::NodeKind::Image(Box::new(vectorcraft_doc::ImageObject {
            key: "k".into(),
            width: 10,
            height: 10,
            xf: vectorcraft_geom::Affine::IDENTITY,
            link: None,
            placement: Default::default(),
            pixels: None,
        })),
    );
    n.appearance.effects = effects;
    n
}

/// A group's Transform effect transforms, with Scale Strokes & Effects off or on, the raster
/// effects that a member's own Transform effect with copies holds back: they keep their value
/// through the member's effect and transform with the group's. That holds for a shadow listed
/// before the member's copying effect (which each copy carries), one listed after it, a fill's
/// shadow under it, the shadow of a member of a nested group with such an effect, and an image's
/// shadow.
#[test]
fn a_groups_transform_moves_the_shadows_its_members_copies_hold_back() {
    let copies = || transform(json!({"moveH": 60, "copies": 1}));
    // Halved and turned 90° counterclockwise on screen, (10, 10) goes to (5, -5).
    let want = (5.0, -5.0);
    for strokes in [false, true] {
        let mut on_fill = fill_square(73, vec![shadow(10.0, 10.0, 0.0)]);
        on_fill.appearance.effects = vec![copies()];
        let members = vec![
            square(71, vec![shadow(10.0, 10.0, 0.0), copies()]),
            square(72, vec![copies(), shadow(10.0, 10.0, 0.0)]),
            on_fill,
            group(74, vec![square(75, vec![shadow(10.0, 10.0, 0.0)])], vec![copies()]),
            image(76, vec![shadow(10.0, 10.0, 0.0)]),
        ];
        let g = group(70, members, vec![transform(json!({"scaleH": 50, "scaleV": 50, "rotate": 90, "strokes": strokes}))]);
        let art = crate::evaluate_container(None, &g).unwrap();
        for id in [71, 72, 73, 75, 76] {
            let drawn = shadows_in(&node_in(&art, id));
            let wanted = if id == 71 { vec![want, want] } else { vec![want] };
            assert!(near(&drawn, &wanted), "Scale Strokes & Effects {strokes}: {id} {drawn:?}");
        }
    }
}

/// A group's Transform effect transforms the raster effects in the art of a member's linked opacity
/// mask with the member, a path's or an image's, with Scale Strokes & Effects off or on.
#[test]
fn a_groups_transform_moves_the_shadow_in_a_members_mask() {
    let masked = |mut n: Node, id: u64| {
        n.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(square(id, vec![shadow(10.0, 10.0, 0.0)]), true)));
        n
    };
    for strokes in [false, true] {
        let params = json!({"scaleH": 50, "scaleV": 50, "rotate": 90, "strokes": strokes});
        let g = group(80, vec![masked(square(81, vec![]), 82), masked(image(83, vec![]), 84)], vec![transform(params)]);
        let art = crate::evaluate_container(None, &g).unwrap();
        for id in [81, 83] {
            let member = node_in(&art, id);
            let mask = member.extras().mask.as_deref().unwrap();
            assert!(near(&shadows_in(&mask.art), &[(5.0, -5.0)]), "Scale Strokes & Effects {strokes}: {id} {:?}", shadows_in(&mask.art));
        }
    }
}

/// A group's Transform effect transforms the shadows of its members' fills that carry a Transform
/// effect of their own with Scale Strokes & Effects on, with the group's Scale Strokes & Effects off
/// or on: a shadow listed after the fill's Transform effect keeps its value through that effect and
/// transforms with the group's, and one listed before it follows the fill's Transform effect and
/// then the group's. Drawn through each member's art.
#[test]
fn a_groups_transform_moves_its_members_fills_shadows_beside_their_own_transform() {
    let scaled = || transform(json!({"scaleH": 200, "scaleV": 200, "strokes": true}));
    // Halved and turned 90° counterclockwise on screen, (10, 10) goes to (5, -5); doubled first by
    // the fill's own Transform effect, it goes to (10, -10).
    let cases = [(91, vec![scaled(), shadow(10.0, 10.0, 0.0)], (5.0, -5.0)), (92, vec![shadow(10.0, 10.0, 0.0), scaled()], (10.0, -10.0))];
    for strokes in [false, true] {
        let members = cases.iter().map(|(id, effects, _)| fill_square(*id, effects.clone())).collect();
        let g = group(90, members, vec![transform(json!({"scaleH": 50, "scaleV": 50, "rotate": 90, "strokes": strokes}))]);
        let art = crate::evaluate_container(None, &g).unwrap();
        for (id, _, want) in &cases {
            let member = node_in(&art, *id);
            // The member is drawn as its art, its fill's own Transform effect applied.
            let drawn = shadows_in(&member);
            assert!(!drawn.is_empty() && near(&drawn, &vec![*want; drawn.len()]), "Scale Strokes & Effects {strokes}: {id} {drawn:?}");
        }
    }
}

/// A closed path of `n` curves along the x axis, 2 pt apart, each drawn with 14 lines at 1 px/pt
/// and with 425 once scaled 1000 times ([`crate::util::line_estimate`]).
fn wavy(n: usize) -> PathData {
    let anchors = (0..=n)
        .map(|i| {
            let x = 2.0 * i as f64;
            Anchor::with_handles(Point::new(x, 0.0), Point::new(x - 0.5, -20.0), Point::new(x + 0.5, 20.0))
        })
        .collect();
    PathData::single(SubPath::new(anchors, true))
}

/// A red path `id` along `path` with object effects `effects`.
fn path_node(id: u64, path: PathData, effects: Vec<Effect>) -> Node {
    let mut n = Node::path(NodeId(id), path, Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0));
    n.appearance.effects = effects;
    n
}

/// A Transform effect scaling 1000 times.
fn thousandfold() -> Effect {
    transform(json!({"scaleH": 100000, "scaleV": 100000}))
}

/// `n` baked on its own ([`bake_document`]).
fn baked(n: Node) -> Node {
    let id = n.id;
    let mut d = Document::new(800.0, 800.0);
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, n).unwrap();
    bake_document(&d).unwrap().node(id).unwrap().clone()
}

/// A Transform effect whose result passes the 8 million lines of the effect budget is left out, so
/// the art stays where it is, and the shadow before it keeps its offset and blur with it: drawn,
/// baked for export, and once shown after baking when it is hidden. One within the budget moves
/// and scales both.
#[test]
fn a_transform_past_the_line_budget_moves_the_art_with_its_shadow() {
    let path = wavy(30_000);
    assert!(Budget::EFFECTS.fits(Budget::of(&path)), "{:?}", Budget::of(&path));
    let candidate = Budget::of(&path.transformed(Affine::scale(1000.0)));
    assert!(candidate.lines > MAX_EFFECT_LINES, "{candidate:?}");
    let b = path.bounds().unwrap();
    let n = path_node(30, path.clone(), vec![shadow(10.0, 5.0, 4.0), thousandfold()]);
    let (g, carry) = apply_geometry_carrying(&n.appearance.effects, &path, b, &GeomContext::of(&n));
    assert_eq!(g, path, "the art stays where it is");
    assert!(carry.skipped());
    let mut drawn = n.appearance.effects.clone();
    carry.hide_skipped(&mut drawn);
    assert!(close(drawn_shadow(&drawn), (10.0, 5.0, 4.0)), "drawn {:?}", drawn_shadow(&drawn));
    let out = baked(n.clone());
    assert_eq!(out.geometric_bounds(), Some(b));
    assert!(close(drawn_shadow(&out.appearance.effects), (10.0, 5.0, 4.0)), "baked {:?}", drawn_shadow(&out.appearance.effects));
    // A hidden shadow keeps its values too, once shown.
    let mut hidden = shadow(10.0, 5.0, 4.0);
    hidden.visible = false;
    let out = baked(path_node(31, path.clone(), vec![hidden, thousandfold()]));
    let mut shown = out.appearance.effects.clone();
    shown.iter_mut().for_each(|e| e.visible = true);
    assert!(close(drawn_shadow(&shown), (10.0, 5.0, 4.0)), "shown {:?}", drawn_shadow(&shown));
    // A fill's own shadow before its own Transform effect stays too.
    let mut fill = path_node(35, path.clone(), vec![]);
    if let Some(item) = fill.appearance.items.first_mut() {
        *item.effects_mut() = vec![shadow(10.0, 5.0, 4.0), thousandfold()];
    }
    let out = baked(fill);
    assert!(near(&shadows_in(&out), &[(10.0, 5.0)]), "baked fill {:?}", shadows_in(&out));
    // Within the budget, both move.
    let doubled = path_node(32, path.clone(), vec![shadow(10.0, 5.0, 4.0), transform(json!({"scaleH": 200, "scaleV": 200}))]);
    let (g, carry) = apply_geometry_carrying(&doubled.appearance.effects, &path, b, &GeomContext::of(&doubled));
    assert!(g != path && !carry.skipped());
    let out = baked(doubled);
    assert!(close(drawn_shadow(&out.appearance.effects), (20.0, 10.0, 8.0)), "baked {:?}", drawn_shadow(&out.appearance.effects));
}

/// The art of a path whose Transform effect is left out past the budget ([`crate::shape_art`],
/// as a group's or a mask's member is drawn) keeps the object's shadow and its fill's shadow with
/// their own offsets.
#[test]
fn the_shape_art_of_a_skipped_transform_keeps_its_shadows() {
    let mut n = path_node(34, wavy(30_000), vec![shadow(10.0, 5.0, 4.0), thousandfold()]);
    if let Some(item) = n.appearance.items.first_mut() {
        *item.effects_mut() = vec![shadow(1.0, 0.5, 0.0)];
    }
    let art = crate::shape_art(None, &n, None).unwrap();
    assert!(near(&shadows_in(&art), &[(10.0, 5.0), (1.0, 0.5)]), "{:?}", shadows_in(&art));
}

/// A path whose own Transform effect is left out past the budget, in a group whose geometry effect
/// (a move, which leaves offsets as they are) runs after its members' own, is drawn with its shadow
/// at its own offset; one within the budget scales the shadow with the path.
#[test]
fn a_members_skipped_transform_keeps_its_shadow_under_its_groups_effects() {
    for (last, want) in [(thousandfold(), (10.0, 5.0)), (transform(json!({"scaleH": 200, "scaleV": 200})), (20.0, 10.0))] {
        let member = path_node(38, wavy(30_000), vec![shadow(10.0, 5.0, 4.0), last.clone()]);
        let g = group(37, vec![member], vec![transform(json!({"moveH": 5, "moveV": 5}))]);
        let art = crate::evaluate_container(None, &g).unwrap();
        assert!(near(&shadows_in(&node_in(&art, 38)), &[want]), "{last:?}: {:?}", shadows_in(&node_in(&art, 38)));
    }
}

/// Of two Transform effects after a shadow, the shadow follows only the ones that ran: the first
/// left out past the budget, or hidden, and the second within it; and both when the first's
/// result is exactly at the limit.
#[test]
fn only_transforms_that_ran_move_a_preceding_shadow() {
    let circle = shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0));
    let b = circle.bounds().unwrap();
    let doubled = Budget::of(&circle.transformed(Affine::scale(2.0))).lines;
    assert!(doubled > Budget::of(&circle).lines);
    let run = |effects: &[Effect], lines: usize| {
        let mut carry = Carry::default();
        apply_geometry_carrying_within(effects, &circle, b, &GeomContext::default(), Budget { anchors: 1000, lines }, Some(&mut carry));
        let mut drawn = effects.to_vec();
        carry.hide_skipped(&mut drawn);
        drawn_shadow(&drawn)
    };
    let effects = vec![shadow(10.0, 0.0, 4.0), transform(json!({"scaleH": 200, "scaleV": 200})), transform(json!({"rotate": 90}))];
    // Turned 90° counterclockwise on screen, (10, 0) goes to (0, -10).
    let only_turned = run(&effects, doubled - 1);
    assert!(only_turned.0.abs() < 1e-9 && (only_turned.1 + 10.0).abs() < 1e-9 && (only_turned.2 - 4.0).abs() < 1e-9, "{only_turned:?}");
    let both = run(&effects, doubled);
    assert!(both.0.abs() < 1e-9 && (both.1 + 20.0).abs() < 1e-9 && (both.2 - 8.0).abs() < 1e-9, "{both:?}");
    let mut hidden = effects.clone();
    hidden[1].visible = false;
    let turned = run(&hidden, doubled);
    assert!(turned.0.abs() < 1e-9 && (turned.1 + 10.0).abs() < 1e-9 && (turned.2 - 4.0).abs() < 1e-9, "{turned:?}");
}

/// A fill's own Transform effect left out past the budget, after a Transform effect of that fill
/// that makes it a piece of its own (Transform Patterns), leaves the piece and the shadow before it
/// as they were. One within the budget scales both.
#[test]
fn a_skipped_item_transform_keeps_its_shadow() {
    let path = wavy(30_000);
    let b = path.bounds().unwrap();
    for (last, want) in [(thousandfold(), (10.0, 5.0)), (transform(json!({"scaleH": 200, "scaleV": 200})), (20.0, 10.0))] {
        let mut n = path_node(33, path.clone(), vec![]);
        if let Some(item) = n.appearance.items.first_mut() {
            *item.effects_mut() = vec![transform(json!({"patterns": true})), shadow(10.0, 5.0, 4.0), last.clone()];
        }
        let art = crate::reshape(None, &n, None).unwrap();
        assert!(near(&shadows_in(&art), &[want]), "{last:?}: {:?}", shadows_in(&art));
        if want == (10.0, 5.0) {
            assert_eq!(art.geometric_bounds(), Some(b), "the piece stays where it is");
        }
        // The same with the Transform effect before the one that makes the piece.
        let mut n = path_node(36, path.clone(), vec![]);
        if let Some(item) = n.appearance.items.first_mut() {
            *item.effects_mut() = vec![shadow(10.0, 5.0, 4.0), last.clone(), transform(json!({"patterns": true}))];
        }
        let art = crate::reshape(None, &n, None).unwrap();
        assert!(near(&shadows_in(&art), &[want]), "{last:?} first: {:?}", shadows_in(&art));
    }
}

/// A group's Transform effect moves each member's shadow as it moves that member: the member whose
/// result would pass what the budget leaves keeps its shape and its shadow, and the other takes
/// both.
#[test]
fn a_group_transform_follows_only_the_members_it_moved() {
    let small = path_node(41, shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), vec![shadow(1.0, 0.5, 0.0)]);
    let large = path_node(42, wavy(30_000), vec![shadow(1.0, 0.5, 0.0)]);
    let g = group(40, vec![small, large], vec![thousandfold()]);
    let art = crate::evaluate_container(None, &g).unwrap();
    assert!(near(&shadows_in(&node_in(&art, 41)), &[(1000.0, 500.0)]), "{:?}", shadows_in(&node_in(&art, 41)));
    assert!(near(&shadows_in(&node_in(&art, 42)), &[(1.0, 0.5)]), "{:?}", shadows_in(&node_in(&art, 42)));
}

/// A Transform effect that makes copies, or has Transform Objects off, keeps its own rule for the
/// shadow before it, past the budget or not: the shadow stays as it is, and the pass does not
/// report it as left out.
#[test]
fn copying_and_object_disabled_transforms_keep_their_existing_shadow_rules() {
    let path = wavy(30_000);
    let b = path.bounds().unwrap();
    for t in [
        transform(json!({"scaleH": 100000, "scaleV": 100000, "copies": 1})),
        transform(json!({"scaleH": 100000, "scaleV": 100000, "objects": false})),
    ] {
        let effects = vec![shadow(10.0, 5.0, 4.0), t.clone()];
        let (_, carry) = apply_geometry_carrying(&effects, &path, b, &GeomContext::default());
        assert!(!carry.skipped(), "{t:?}");
        assert!(close(drawn_shadow(&effects), (10.0, 5.0, 4.0)), "{t:?}: {:?}", drawn_shadow(&effects));
    }
}

/// The blur of every Drop Shadow drawn on `n` and the nodes under it, in the order of
/// [`shadows_in`].
fn shadow_blurs_in(n: &Node) -> Vec<f64> {
    let mut out = vec![];
    n.walk(&mut |m| {
        let lists = std::iter::once(&m.appearance.effects).chain(m.appearance.items.iter().map(|i| i.effects()));
        for list in lists {
            for r in raster_effects(list) {
                if let RasterFx::DropShadow { blur, .. } = r {
                    out.push(blur);
                }
            }
        }
    });
    out
}

/// How many paths `n` holds.
fn paths_in(n: &Node) -> usize {
    let mut count = 0;
    n.walk(&mut |m| count += usize::from(matches!(m.kind, vectorcraft_doc::NodeKind::Path { .. })));
    count
}

/// A Transform effect that makes copies gives each copy the shadow listed before it, moved, turned,
/// reflected and scaled with that copy, the last copy lowest: drawn, expanded and baked. The object
/// itself keeps the shadow as it is, and paints none around all of them. With Scale Strokes &
/// Effects on, the shadow is not scaled a second time.
#[test]
fn each_copy_of_a_transform_effect_carries_the_shadow_listed_before_it() {
    let cases = [
        (json!({"moveH": 100, "copies": 2}), vec![(10.0, 0.0), (10.0, 0.0), (10.0, 0.0)], vec![4.0, 4.0, 4.0]),
        // A turn of 90° counterclockwise on screen takes an offset to the right to one upward.
        (json!({"rotate": 90, "copies": 2}), vec![(-10.0, 0.0), (0.0, -10.0), (10.0, 0.0)], vec![4.0, 4.0, 4.0]),
        (json!({"reflectX": true, "moveH": 100, "copies": 1}), vec![(-10.0, 0.0), (10.0, 0.0)], vec![4.0, 4.0]),
        (json!({"scaleH": 200, "scaleV": 200, "moveH": 100, "copies": 1}), vec![(20.0, 0.0), (10.0, 0.0)], vec![8.0, 4.0]),
        (json!({"scaleH": 200, "scaleV": 200, "moveH": 100, "copies": 1, "strokes": true}), vec![(20.0, 0.0), (10.0, 0.0)], vec![8.0, 4.0]),
        (json!({"scaleH": 200, "scaleV": 200, "moveH": 100, "copies": 1, "random": true}), vec![(10.0, 0.0), (10.0, 0.0)], vec![4.0, 4.0]),
    ];
    for (params, want, blurs) in cases {
        let n = square(91, vec![shadow(10.0, 0.0, 4.0), transform(params.clone())]);
        assert!(crate::has_transform_art(&n), "{params}");
        assert!(crate::outer_effects(&n).iter().all(|e| e.id != "stylize.dropShadow"), "{params}");
        let random = params["random"].as_bool().unwrap_or(false);
        for (how, art) in
            [("drawn", crate::reshape(None, &n, None).unwrap()), ("expanded", expand_outlined(None, &n, None).unwrap()), ("baked", baked(n.clone()))]
        {
            let drawn = shadows_in(&art);
            assert_eq!(paths_in(&art), want.len(), "{params}: {how} {art:?}");
            if random {
                // Random scales the copy by a share of the 200 %, and its shadow with it; the object
                // keeps its own.
                assert_eq!(drawn.len(), want.len(), "{params}: {how} {drawn:?}");
                assert!(near(&drawn[1..], &want[1..]), "{params}: {how} {drawn:?}");
                continue;
            }
            assert!(near(&drawn, &want), "{params}: {how} {drawn:?}");
            let b = shadow_blurs_in(&art);
            assert!(b.len() == blurs.len() && b.iter().zip(&blurs).all(|(x, y)| (x - y).abs() < 1e-9), "{params}: {how} blurs {b:?}");
        }
    }
}

/// A shadow listed after a Transform effect that makes copies paints once around all of them, and
/// one listed before it on each copy. A Transform effect that makes no copies, or has Transform
/// Objects off, leaves the object's shadows on the object.
#[test]
fn a_shadow_after_a_copying_transform_paints_once_around_the_copies() {
    let n = square(92, vec![shadow(10.0, 0.0, 0.0), transform(json!({"rotate": 90, "copies": 1})), shadow(0.0, 5.0, 0.0)]);
    let art = crate::reshape(None, &n, None).unwrap();
    assert!(near(&shadows_in(&art), &[(0.0, 5.0), (0.0, -10.0), (10.0, 0.0)]), "{:?}", shadows_in(&art));
    assert_eq!(crate::outer_effects(&n).len(), 2);
    for params in [json!({"rotate": 90}), json!({"rotate": 90, "copies": 1, "objects": false})] {
        let n = square(93, vec![shadow(10.0, 0.0, 0.0), transform(params.clone())]);
        assert!(!crate::has_transform_art(&n), "{params}");
        assert_eq!(crate::outer_effects(&n).len(), 2, "{params}");
    }
}

/// Copies that stop at the effect budget each keep their shadow: no copy goes without one, and no
/// shadow is left without its copy.
#[test]
fn copies_past_the_budget_each_keep_their_shadow() {
    let n = path_node(94, wavy(30_000), vec![shadow(1.0, 0.5, 0.0), transform(json!({"moveV": 50, "copies": 1000}))]);
    let art = crate::reshape(None, &n, None).unwrap();
    let paths = paths_in(&art);
    assert!(paths > 1 && paths < 1001, "{paths}");
    assert_eq!(shadows_in(&art).len(), paths);
}
