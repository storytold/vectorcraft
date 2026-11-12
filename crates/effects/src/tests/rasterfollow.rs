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
    assert!(near(&[fill_shadow(&node_in(&art, 33))], &[want]), "fill: {:?}", fill_shadow(&node_in(&art, 33)));
    let nested = group(34, vec![group(35, vec![square(36, vec![shadow(10.0, 0.0, 0.0)])], vec![turn()])], vec![uneven()]);
    let art = crate::evaluate_container(None, &nested).unwrap();
    let inner = crate::evaluate_container(None, &node_in(&art, 35)).unwrap();
    assert!(near(&shadows_in(&inner), &[want]), "nested: {:?}", shadows_in(&inner));
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
/// before the member's copying effect, one listed after it, a fill's shadow under it, the shadow
/// of a member of a nested group with such an effect, and an image's shadow.
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
            assert!(near(&drawn, &[want]), "Scale Strokes & Effects {strokes}: {id} {drawn:?}");
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
