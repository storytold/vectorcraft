//! Raster effects listed before a Transform effect follow it: the Transform effect transforms
//! what they paint.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Effect, Node, NodeId};
use vectorcraft_geom::{Affine, Anchor, PathData, Point, Rect, SubPath, shapes};

use crate::geometry::{Budget, MAX_EFFECT_LINES, apply_geometry_carrying_within};
use crate::pixel::{Light, StrokeDirection};
use crate::{Carry, GeomContext, PixelFx, RasterFx, apply_geometry_carrying, bake_document, expand_outlined, raster_effects};

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

/// Rasterize keeps its room around the art: its image keeps its own pixel grid, and the Transform
/// effect moves the image instead ([`crate::rasterize_stage`]).
#[test]
fn rasterize_before_a_transform_keeps_its_options() {
    let rasterize = fx(crate::RASTERIZE, json!({"addAround": 6}));
    let effects = [rasterize.clone(), transform(json!({"scaleH": 200, "scaleV": 200}))];
    assert_eq!(crate::raster::followed(&effects)[0], rasterize);
    assert_eq!(crate::raster::without_geometry(&effects), vec![rasterize]);
    assert_eq!(crate::rasterize_stage(&effects), Some(vec![1]));
}

/// The Transform effects after the last Rasterize effect move its image when, from the first of
/// them on, every visible effect is a Transform effect that moves the art once. Hidden ones, and
/// ones with Transform Objects off, leave the image where it is.
#[test]
fn rasterize_following_ignores_hidden_and_objects_off() {
    let rasterize = || fx(crate::RASTERIZE, json!({}));
    let scale = || transform(json!({"scaleH": 200, "scaleV": 200}));
    let mut hidden = scale();
    hidden.visible = false;
    let off = transform(json!({"scaleH": 200, "objects": false}));
    let roughen = fx("distort.roughen", json!({}));
    let shadow = shadow(5.0, 5.0, 2.0);
    let cases: Vec<(Vec<Effect>, Option<Vec<usize>>)> = vec![
        (vec![rasterize(), hidden.clone()], None),
        (vec![rasterize(), off.clone()], None),
        (vec![rasterize(), hidden, off, scale()], Some(vec![3])),
        (vec![rasterize(), transform(json!({"moveH": 10})), scale()], Some(vec![1, 2])),
        // Effects before the first Transform effect go into the image; anything after it holds
        // the image back.
        (vec![rasterize(), roughen.clone(), shadow.clone(), scale()], Some(vec![3])),
        (vec![rasterize(), scale(), roughen], None),
        (vec![rasterize(), scale(), shadow], None),
        (vec![rasterize(), transform(json!({"scaleH": 200, "copies": 2}))], None),
        (vec![rasterize(), transform(json!({"scaleH": 200, "random": true}))], None),
        // The Transform effect before the Rasterize effect moves the art.
        (vec![scale(), rasterize()], None),
        (vec![scale(), rasterize(), rasterize(), scale()], Some(vec![3])),
    ];
    for (effects, want) in cases {
        let ids: Vec<&str> = effects.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(crate::rasterize_stage(&effects), want, "{ids:?}");
    }
    // What the image is made from hides those Transform effects; the rest stay as they are.
    let effects = [rasterize(), scale()];
    let staged = crate::staged_effects(&effects, &[1]);
    assert_eq!((staged[0].visible, staged[1].visible), (true, false));
}

/// The map that moves the image is each Transform effect's map about its reference point on the
/// bounds of the geometry it takes, in order.
#[test]
fn the_image_moves_by_each_transform_about_the_art_it_takes() {
    let staged = shapes::rectangle(Rect::new(10.0, 10.0, 30.0, 30.0));
    let effects = [fx(crate::RASTERIZE, json!({})), transform(json!({"scaleH": 200, "scaleV": 200})), transform(json!({"moveH": 5}))];
    let map = crate::stage_map(&effects, &[1, 2], &staged).unwrap();
    // Twice the size about the center (20, 20), then 5 pt to the right.
    let want = Affine::translate((5.0, 0.0)) * Affine::translate((20.0, 20.0)) * Affine::scale(2.0) * Affine::translate((-20.0, -20.0));
    assert!(map.as_coeffs().iter().zip(want.as_coeffs()).all(|(a, b)| (a - b).abs() < 1e-9), "{map:?}");
    // The second Transform effect turns about the center of what the first one made.
    let effects = [fx(crate::RASTERIZE, json!({})), transform(json!({"moveH": 100})), transform(json!({"rotate": 90}))];
    let map = crate::stage_map(&effects, &[1, 2], &staged).unwrap();
    let c = map * Point::new(20.0, 20.0);
    assert!((c.x - 120.0).abs() < 1e-9 && (c.y - 20.0).abs() < 1e-9, "{c:?}");
    assert_eq!(crate::stage_map(&effects, &[1, 2], &PathData::default()), None);
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

/// The light direction of the first pixel filter `effects` draw, if it has one.
fn drawn_light(effects: &[Effect]) -> Option<Light> {
    raster_effects(effects).into_iter().find_map(|r| match r {
        RasterFx::Pixel(
            PixelFx::Underpainting { light, .. }
            | PixelFx::RoughPastels { light, .. }
            | PixelFx::ConteCrayon { light, .. }
            | PixelFx::BasRelief { light, .. }
            | PixelFx::Plaster { light, .. }
            | PixelFx::Texturizer { light, .. },
        ) => Some(light),
        _ => None,
    })
}

/// The stroke direction of the first pixel filter `effects` draw, if it has one.
fn drawn_strokes(effects: &[Effect]) -> Option<StrokeDirection> {
    raster_effects(effects).into_iter().find_map(|r| match r {
        RasterFx::Pixel(PixelFx::SprayedStrokes { direction, .. } | PixelFx::GraphicPen { direction, .. }) => Some(direction),
        _ => None,
    })
}

/// The screen angles of the first Color Halftone `effects` draw.
fn drawn_screens(effects: &[Effect]) -> [f64; 4] {
    match raster_effects(effects).into_iter().next() {
        Some(RasterFx::Pixel(PixelFx::ColorHalftone { angles, .. })) => angles,
        other => panic!("expected a Color Halftone, got {other:?}"),
    }
}

/// A Bas Relief lit from `light`, then a Transform effect with `params`: the light it draws with.
fn relief_then(light: &str, params: Value) -> Option<Light> {
    drawn_light(&[fx("sketch.basRelief", json!({ "lightDirection": light })), transform(params)])
}

/// A light direction before a Transform effect turns and reflects with the art (y down on the
/// page), to the nearest of the eight directions, the clockwise one at a tie.
#[test]
fn bas_relief_light_follows_rotation_and_reflection() {
    assert_eq!(relief_then("top", json!({"rotate": 90})), Some(Light::Left));
    assert_eq!(relief_then("top", json!({"reflectY": true})), Some(Light::Bottom));
    assert_eq!(relief_then("top", json!({"reflectX": true})), Some(Light::Top));
    assert_eq!(relief_then("left", json!({"reflectX": true})), Some(Light::Right));
    // 22.5° clockwise lies halfway between Top and Top Right; 22.5° counterclockwise halfway
    // between Top and Top Left.
    assert_eq!(relief_then("top", json!({"rotate": -22.5})), Some(Light::TopRight));
    assert_eq!(relief_then("top", json!({"rotate": 22.5})), Some(Light::Top));
    assert_eq!(relief_then("top", json!({"rotate": 30})), Some(Light::TopLeft));
    // A turn of 90° counterclockwise, from each direction.
    for (from, to) in [
        ("bottom", Light::Right),
        ("bottomLeft", Light::BottomRight),
        ("left", Light::Bottom),
        ("topLeft", Light::BottomLeft),
        ("top", Light::Left),
        ("topRight", Light::TopLeft),
        ("right", Light::Top),
        ("bottomRight", Light::TopRight),
    ] {
        assert_eq!(relief_then(from, json!({"rotate": 90})), Some(to), "{from}");
    }
    // Every filter with a light direction, and Bas Relief's default light (Bottom).
    for id in ["artistic.underpainting", "artistic.roughPastels", "sketch.conteCrayon", "sketch.basRelief", "sketch.plaster", "texture.texturizer"] {
        let effects = [fx(id, json!({"lightDirection": "top"})), transform(json!({"rotate": 90}))];
        assert_eq!(drawn_light(&effects), Some(Light::Left), "{id}");
    }
    assert_eq!(drawn_light(&[fx("sketch.basRelief", json!({})), transform(json!({"rotate": 90}))]), Some(Light::Right));
    // The Transform effects after the filter turn it together, with one rounding at the end.
    let twice = [fx("sketch.basRelief", json!({"lightDirection": "top"})), transform(json!({"rotate": -22.5})), transform(json!({"rotate": -22.5}))];
    assert_eq!(drawn_light(&twice), Some(Light::TopRight));
}

/// A stroke direction before a Transform effect turns and reflects with the art, to the nearest
/// of the four, a stroke and its reverse being the same, the clockwise one at a tie.
#[test]
fn stroke_axis_follows_transform() {
    let strokes = |id: &str, way: Option<&str>, params: Value| {
        let p = way.map_or(json!({}), |w| json!({ "strokeDirection": w }));
        drawn_strokes(&[fx(id, p), transform(params)])
    };
    let sprayed = "brushStrokes.sprayedStrokes";
    let pen = "sketch.graphicPen";
    assert_eq!(strokes(sprayed, Some("horizontal"), json!({"rotate": 90})), Some(StrokeDirection::Vertical));
    assert_eq!(strokes(sprayed, Some("vertical"), json!({"rotate": 90})), Some(StrokeDirection::Horizontal));
    assert_eq!(strokes(pen, Some("rightDiagonal"), json!({"rotate": 90})), Some(StrokeDirection::LeftDiagonal));
    assert_eq!(strokes(pen, Some("rightDiagonal"), json!({"reflectX": true})), Some(StrokeDirection::LeftDiagonal));
    assert_eq!(strokes(pen, Some("leftDiagonal"), json!({"reflectY": true})), Some(StrokeDirection::RightDiagonal));
    assert_eq!(strokes(pen, None, json!({"rotate": 90})), Some(StrokeDirection::LeftDiagonal));
    assert_eq!(strokes(sprayed, Some("horizontal"), json!({"rotate": -22.5})), Some(StrokeDirection::LeftDiagonal));
    assert_eq!(strokes(sprayed, Some("horizontal"), json!({"rotate": 22.5})), Some(StrokeDirection::Horizontal));
    assert_eq!(strokes(sprayed, Some("horizontal"), json!({"rotate": -44})), Some(StrokeDirection::LeftDiagonal));
    assert_eq!(strokes(sprayed, Some("horizontal"), json!({"rotate": 180})), Some(StrokeDirection::Horizontal));
}

fn near4(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).all(|(p, q)| (p - q).abs() < 1e-6)
}

/// Color Halftone's screen angles before a Transform effect turn and reflect with the art.
#[test]
fn halftone_channels_follow_rotation_and_reflection() {
    let screens = |p: Value, transforms: &[Value]| {
        let mut effects = vec![fx("pixelate.colorHalftone", p)];
        effects.extend(transforms.iter().cloned().map(transform));
        drawn_screens(&effects)
    };
    let defaults = screens(json!({}), &[json!({"rotate": 90})]);
    assert!(near4(defaults, [198.0, 252.0, 180.0, 135.0]), "{defaults:?}");
    let reflected = screens(json!({"channel1": 45, "channel2": 30, "channel3": 0, "channel4": 270}), &[json!({"reflectX": true})]);
    assert!(near4(reflected, [135.0, 150.0, 180.0, 270.0]), "{reflected:?}");
    let flipped = screens(json!({"channel1": 45, "channel2": 30, "channel3": 0, "channel4": 270}), &[json!({"reflectY": true})]);
    assert!(near4(flipped, [315.0, 330.0, 0.0, 90.0]), "{flipped:?}");
    // One Transform effect reflects, then turns; two apply in the order they are listed.
    let p = json!({"channel1": 10.5, "channel2": -30, "channel3": 0, "channel4": 359.25});
    let one = screens(p.clone(), &[json!({"rotate": 30, "reflectX": true})]);
    assert!(near4(one, [199.5, 240.0, 210.0, 210.75]), "{one:?}");
    let two = screens(p, &[json!({"rotate": 30}), json!({"reflectX": true})]);
    assert!(near4(two, [139.5, 180.0, 150.0, 150.75]), "{two:?}");
}

/// Only the visible Transform effects listed after a filter that move the art once turn its
/// direction; a hidden filter turns too, and a zero scale leaves it as it is.
#[test]
fn only_earlier_visible_single_map_effects_follow() {
    let relief = fx("sketch.basRelief", json!({"lightDirection": "top"}));
    assert_eq!(drawn_light(&[transform(json!({"rotate": 90})), relief.clone()]), Some(Light::Top));
    for params in [
        json!({"rotate": 90, "objects": false}),
        json!({"rotate": 90, "copies": 1}),
        json!({"rotate": 90, "random": true}),
        json!({"scaleH": 0, "scaleV": 0}),
    ] {
        assert_eq!(drawn_light(&[relief.clone(), transform(params.clone())]), Some(Light::Top), "{params}");
    }
    let mut hidden = transform(json!({"rotate": 90}));
    hidden.visible = false;
    assert_eq!(drawn_light(&[relief.clone(), hidden]), Some(Light::Top));
    let zero = [fx("pixelate.colorHalftone", json!({"channel1": 20})), transform(json!({"scaleH": 0, "scaleV": 0}))];
    assert!(near4(drawn_screens(&zero), [20.0, 162.0, 90.0, 45.0]), "{:?}", drawn_screens(&zero));
    // A hidden filter kept when the Transform effect is applied draws turned once it is shown.
    let mut off = relief;
    off.visible = false;
    let kept = crate::raster::without_geometry(&[off, transform(json!({"rotate": 90}))]);
    assert_eq!(kept.len(), 1);
    let mut shown = kept[0].clone();
    shown.visible = true;
    assert_eq!(drawn_light(&[shown]), Some(Light::Left));
}

/// The light direction of every Bas Relief drawn on `n` and the nodes under it, from the object's
/// effects and from each fill's and stroke's.
fn lights_in(n: &Node) -> Vec<Light> {
    let mut out = vec![];
    n.walk(&mut |m| {
        let lists = std::iter::once(&m.appearance.effects).chain(m.appearance.items.iter().map(|i| i.effects()));
        for list in lists {
            out.extend(drawn_light(list));
        }
    });
    out
}

/// The canvas, a fill's own effects, a group's members and export turn the light the same way.
#[test]
fn nested_and_baked_directions_match_canvas() {
    let relief = || fx("sketch.basRelief", json!({"lightDirection": "top"}));
    let turn = || transform(json!({"rotate": 90}));
    // A fill's own Transform effect, and the object's Transform effect over its fill.
    let fill = fill_square(31, vec![relief(), turn()]);
    let item = fill.appearance.items.first().unwrap();
    assert_eq!(drawn_light(item.effects()), Some(Light::Left));
    let mut over = fill_square(32, vec![relief()]);
    over.appearance.effects = vec![turn()];
    let item = over.appearance.items.first().unwrap();
    assert_eq!(drawn_light(&crate::item_effects_in(&over.appearance.effects, item.effects())), Some(Light::Left));
    // A fill's own turn of 30° and the object's turn of 30° make 60°, rounded once: Top Left, not
    // Left (Top Left after the first turn, then Left after the second).
    let thirty = || transform(json!({"rotate": 30}));
    let mut both = fill_square(37, vec![relief(), thirty()]);
    both.appearance.effects = vec![thirty()];
    let item = both.appearance.items.first().unwrap();
    assert_eq!(drawn_light(&crate::item_effects_in(&both.appearance.effects, item.effects())), Some(Light::TopLeft));
    // A group's Transform effect over a member's own filter and its fill's.
    let g = group(33, vec![square(34, vec![relief()]), fill_square(35, vec![relief()])], vec![turn()]);
    let art = crate::evaluate_container(None, &g).unwrap();
    assert_eq!(lights_in(&art), vec![Light::Left; 2]);
    // Baked for export: the Transform effect is applied and the filter keeps the turned light.
    let mut d = Document::new(800.0, 800.0);
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, square(36, vec![relief(), turn()])).unwrap();
    d.insert(Some(layer), 1, g).unwrap();
    let baked = bake_document(&d).unwrap();
    let b = baked.node(NodeId(36)).unwrap();
    assert!(!b.appearance.effects.iter().any(|e| e.id == "distort.transform"), "{:?}", b.appearance.effects);
    assert_eq!(drawn_light(&b.appearance.effects), Some(Light::Left));
    for id in [34, 35] {
        assert_eq!(lights_in(baked.node(NodeId(id)).unwrap()), vec![Light::Left], "{id}");
    }
}

/// A member's light under two nested groups that each turn 30° turns 60° in all, rounded once:
/// Top Left, not Left (Top Left after the inner group's turn, then Left after the outer one's).
#[test]
fn nested_group_turns_round_the_light_once() {
    let relief = || fx("sketch.basRelief", json!({"lightDirection": "top"}));
    let thirty = || transform(json!({"rotate": 30}));
    let inner = group(41, vec![square(42, vec![relief()]), fill_square(43, vec![relief()])], vec![thirty()]);
    let outer = group(40, vec![inner], vec![thirty()]);
    let art = crate::evaluate_container(None, &outer).unwrap();
    assert_eq!(lights_in(&art), vec![Light::TopLeft; 2]);
    let mut d = Document::new(800.0, 800.0);
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, outer).unwrap();
    let baked = bake_document(&d).unwrap();
    for id in [42, 43] {
        assert_eq!(lights_in(baked.node(NodeId(id)).unwrap()), vec![Light::TopLeft], "{id}");
    }
    // Three groups make 90°: Left, not Bottom Left.
    let three = group(44, vec![group(45, vec![group(46, vec![square(47, vec![relief()])], vec![thirty()])], vec![thirty()])], vec![thirty()]);
    let art = crate::evaluate_container(None, &three).unwrap();
    assert_eq!(lights_in(&art), vec![Light::Left]);
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
/// they are.
#[test]
fn a_groups_copying_transform_leaves_its_members_shadows() {
    for params in [json!({"scaleH": 200, "scaleV": 200, "copies": 1}), json!({"scaleH": 200, "scaleV": 200, "random": true})] {
        let g = group(25, vec![square(26, vec![shadow(10.0, 10.0, 0.0)]), square(27, vec![])], vec![transform(params.clone())]);
        let art = crate::evaluate_container(None, &g).unwrap();
        let drawn = shadows_in(&art);
        assert!(!drawn.is_empty() && near(&drawn, &vec![(10.0, 10.0); drawn.len()]), "{params}: {drawn:?}");
    }
}

/// A layer of `members` with `effects` of its own.
fn layer(id: u64, members: Vec<Node>, effects: Vec<Effect>) -> Node {
    let mut l = Node::layer(NodeId(id), "Layer", vectorcraft_doc::LayerColor::Preset(0));
    if let Some(ch) = l.children_mut() {
        *ch = members.into_iter().map(std::sync::Arc::new).collect();
    }
    l.appearance.effects = effects;
    l
}

/// The offset and blur of the Drop Shadow that each member of `art` draws of its own, in order.
fn copy_shadows(art: &Node) -> Vec<(f64, f64, f64)> {
    art.children().into_iter().flatten().map(|c| drawn_shadow(&c.appearance.effects)).collect()
}

fn close_all(a: &[(f64, f64, f64)], b: &[(f64, f64, f64)]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| close(*x, *y))
}

/// A group's or layer's Transform effect that makes copies gives each copy the shadow listed
/// before it, moved, turned, reflected and scaled with that copy, the last copy lowest: drawn and
/// baked. Each copy is a group holding all the members, which carry no shadow of their own, and
/// the art paints none around all the copies. With Scale Strokes & Effects on, the shadow is not
/// scaled a second time. A layer's art stays a layer.
#[test]
fn each_copy_of_a_groups_or_layers_transform_effect_carries_the_shadow_listed_before_it() {
    let cases = [
        (json!({"moveH": 100, "copies": 2}), vec![(10.0, 0.0, 4.0); 3]),
        // A turn of 90° counterclockwise on screen takes an offset to the right to one upward.
        (json!({"rotate": 90, "copies": 2}), vec![(-10.0, 0.0, 4.0), (0.0, -10.0, 4.0), (10.0, 0.0, 4.0)]),
        (json!({"reflectX": true, "moveH": 100, "copies": 1}), vec![(-10.0, 0.0, 4.0), (10.0, 0.0, 4.0)]),
        (json!({"scaleH": 200, "scaleV": 200, "moveH": 100, "copies": 1}), vec![(20.0, 0.0, 8.0), (10.0, 0.0, 4.0)]),
        (json!({"scaleH": 200, "scaleV": 200, "moveH": 100, "copies": 1, "strokes": true}), vec![(20.0, 0.0, 8.0), (10.0, 0.0, 4.0)]),
    ];
    for (params, want) in cases {
        let offsets: Vec<(f64, f64)> = want.iter().map(|w| (w.0, w.1)).collect();
        for container in ["group", "layer"] {
            let members = vec![square(101, vec![]), square(102, vec![])];
            let effects = vec![shadow(10.0, 0.0, 4.0), transform(params.clone())];
            let n = if container == "group" { group(100, members, effects) } else { layer(100, members, effects) };
            assert!(crate::outer_effects(&n).iter().all(|e| e.id != "stylize.dropShadow"), "{params}: {container}");
            let art = crate::evaluate_container(None, &n).unwrap();
            assert_eq!(matches!(art.kind, vectorcraft_doc::NodeKind::Layer { .. }), container == "layer", "{params}: {container}");
            assert!(raster_effects(&art.appearance.effects).is_empty(), "{params}: {container} {:?}", art.appearance.effects);
            let drawn = copy_shadows(&art);
            assert!(close_all(&drawn, &want), "{params}: {container} {drawn:?}");
            assert_eq!(shadows_in(&art).len(), want.len(), "{params}: {container}: the members draw none");
            for c in art.children().into_iter().flatten() {
                assert!(matches!(c.kind, vectorcraft_doc::NodeKind::Group { .. }) && paths_in(c) == 2, "{params}: {container} {c:?}");
            }
            let b = baked(n);
            assert!(near(&shadows_in(&b), &offsets), "{params}: {container} baked {:?}", shadows_in(&b));
        }
    }
}

/// A group's copying Transform effect leaves its members' own shadows on each copy as they are,
/// and a shadow listed after it paints once around all the copies. A clip group's copies each clip
/// and carry the shadow, and the art clips nothing. A Transform effect that makes no copies, or has
/// Transform Objects off, leaves the group's shadows on the group.
#[test]
fn a_groups_copying_transform_leaves_member_and_later_shadows_where_they_paint() {
    let g = group(
        110,
        vec![square(111, vec![shadow(5.0, 5.0, 0.0)]), square(112, vec![])],
        vec![shadow(10.0, 0.0, 0.0), transform(json!({"rotate": 90, "copies": 1})), shadow(0.0, 5.0, 0.0)],
    );
    assert_eq!(crate::outer_effects(&g).len(), 2);
    let art = crate::evaluate_container(None, &g).unwrap();
    assert!(near(&shadows_in(&art), &[(0.0, 5.0), (0.0, -10.0), (5.0, 5.0), (10.0, 0.0), (5.0, 5.0)]), "{:?}", shadows_in(&art));

    let mut clip = square(121, vec![]);
    if let vectorcraft_doc::NodeKind::Path { clipping, .. } = &mut clip.kind {
        *clipping = true;
    }
    let mut g = group(120, vec![clip, square(122, vec![])], vec![shadow(10.0, 0.0, 0.0), transform(json!({"moveH": 100, "copies": 1}))]);
    if let vectorcraft_doc::NodeKind::Group { clip, .. } = &mut g.kind {
        *clip = true;
    }
    let art = crate::evaluate_container(None, &g).unwrap();
    assert!(!art.clips());
    assert!(art.children().into_iter().flatten().all(|c| c.clips()), "{art:?}");
    assert!(close_all(&copy_shadows(&art), &[(10.0, 0.0, 0.0), (10.0, 0.0, 0.0)]), "{:?}", copy_shadows(&art));

    for params in [json!({"rotate": 90}), json!({"rotate": 90, "copies": 1, "objects": false})] {
        let g = group(130, vec![square(131, vec![]), square(132, vec![])], vec![shadow(10.0, 0.0, 0.0), transform(params.clone())]);
        assert_eq!(crate::outer_effects(&g).len(), 2, "{params}");
        let art = crate::evaluate_container(None, &g).unwrap();
        assert_eq!(raster_effects(&art.appearance.effects).len(), 1, "{params}: the group keeps its shadow");
    }
}

/// Copies of a group that stop at the effect budget each keep their shadow: no copy goes without
/// one, and no shadow is left without its copy.
#[test]
fn a_groups_copies_past_the_budget_each_keep_their_shadow() {
    let g = group(140, vec![path_node(141, wavy(30_000), vec![])], vec![shadow(1.0, 0.5, 0.0), transform(json!({"moveV": 50, "copies": 1000}))]);
    let art = crate::evaluate_container(None, &g).unwrap();
    let paths = paths_in(&art);
    assert!(paths > 1 && paths < 1001, "{paths}");
    assert_eq!(shadows_in(&art).len(), paths);
}

/// A layer's ruler guides stay once above the copies of its Transform effect, and carry no shadow.
#[test]
fn a_layers_guides_stay_once_above_copies_that_carry_its_shadow() {
    let guide = Node::new(NodeId(152), vectorcraft_doc::NodeKind::RulerGuide { vertical: true, pos: 3.0, artboard: None });
    let l = layer(150, vec![square(151, vec![]), guide], vec![shadow(10.0, 0.0, 0.0), transform(json!({"moveH": 100, "copies": 2}))]);
    let art = crate::evaluate_container(None, &l).unwrap();
    let ch = art.children().unwrap();
    assert_eq!(ch.len(), 4, "three copies, then the guide");
    assert!(ch[3].is_ruler_guide() && ch.iter().filter(|c| c.is_ruler_guide()).count() == 1);
    assert_eq!(shadows_in(&art).len(), 3);
}

/// The bounds of every node with `id` under `art`, in order.
fn bounds_of(art: &Node, id: u64) -> Vec<Rect> {
    let mut out = vec![];
    art.walk(&mut |m| {
        if m.id == NodeId(id)
            && let Some(b) = m.geometric_bounds()
        {
            out.push(b);
        }
    });
    out
}

fn rects_near(a: &[Rect], b: &[Rect]) -> bool {
    let near = |p: f64, q: f64| (p - q).abs() < 1e-6;
    a.len() == b.len() && a.iter().zip(b).all(|(p, q)| near(p.x0, q.x0) && near(p.y0, q.y0) && near(p.x1, q.x1) && near(p.y1, q.y1))
}

/// A group with a fill of its own and only a shadow before its copying Transform effect applies
/// its members' own effects before it copies them, as a group without a fill does: a member's own
/// move is turned with each copy.
#[test]
fn a_painted_groups_copies_take_their_members_own_effects_first() {
    let members = || vec![square(171, vec![transform(json!({"moveH": 40}))]), square(172, vec![])];
    let effects = || vec![shadow(10.0, 0.0, 0.0), transform(json!({"rotate": 90, "copies": 1}))];
    let plain = crate::evaluate_container(None, &group(170, members(), effects())).unwrap();
    let mut painted = group(170, members(), effects());
    *painted.appearance = Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0);
    painted.appearance.effects = effects();
    let art = crate::evaluate_container(None, &painted).unwrap();
    let (want, got) = (bounds_of(&plain, 171), bounds_of(&art, 171));
    assert!(!want.is_empty());
    assert!(rects_near(&got, &want), "{got:?} {want:?}");
}

/// A layer's copies are made about the bounds of its visible members, as the layer measures them:
/// a hidden member does not move the center that a reflected copy turns about.
#[test]
fn a_layers_copies_leave_its_hidden_members_out_of_their_reference() {
    let mut hidden = path_node(182, shapes::rectangle(Rect::new(1000.0, 0.0, 1050.0, 50.0)), vec![]);
    hidden.visible = false;
    let l = layer(180, vec![square(181, vec![]), hidden], vec![shadow(10.0, 0.0, 0.0), transform(json!({"reflectX": true, "copies": 1}))]);
    let art = crate::evaluate_container(None, &l).unwrap();
    let got = bounds_of(&art, 181);
    assert!(rects_near(&got, &[Rect::new(0.0, 0.0, 50.0, 50.0); 2]), "{got:?}");
}

/// A shadow carried by the copies of an earlier copying Transform effect of a group turns with each
/// copy of a later one, drawn and baked.
#[test]
fn a_groups_later_copying_transform_turns_the_shadows_of_earlier_copies() {
    let g = || {
        group(
            190,
            vec![square(191, vec![])],
            vec![shadow(10.0, 0.0, 0.0), transform(json!({"moveH": 100, "copies": 1})), transform(json!({"rotate": 90, "copies": 1}))],
        )
    };
    let want = [(0.0, -10.0), (0.0, -10.0), (10.0, 0.0), (10.0, 0.0)];
    let art = crate::evaluate_container(None, &g()).unwrap();
    assert!(near(&shadows_in(&art), &want), "{:?}", shadows_in(&art));
    let b = baked(g());
    assert!(near(&shadows_in(&b), &want), "baked {:?}", shadows_in(&b));
}

/// A group holding type whose Transform effect only moves its copies, baked for export, gives each
/// copy the shadow listed before that effect, as the canvas draws it, rather than keeping the type
/// as type under one shadow around all the copies. Built without fonts, type lays out no glyphs
/// and the test is skipped.
#[test]
fn a_baked_group_with_type_keeps_a_shadow_on_each_copy() {
    use vectorcraft_doc::text::{CharStyle, TextObject};
    let style = CharStyle { size: 40.0, fill: Paint::solid(Color::BLACK), ..Default::default() };
    let words = Node::new(NodeId(161), vectorcraft_doc::NodeKind::Text(Box::new(TextObject::point(Point::new(5.0, 60.0), "IIII", style))));
    if !crate::outline_text(&words).is_some_and(|o| paths_in(&o) > 0) {
        eprintln!("skipped: built without fonts (enable vectorcraft-text's bundled-fonts to run it)");
        return;
    }
    let g = group(160, vec![words, square(162, vec![])], vec![shadow(10.0, 0.0, 0.0), transform(json!({"moveH": 150, "copies": 1}))]);
    let b = baked(g);
    assert!(near(&shadows_in(&b), &[(10.0, 0.0), (10.0, 0.0)]), "{:?}", shadows_in(&b));
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
/// art, whatever its options: drawn, drawn through the art an option makes, and baked. For one that
/// makes copies, see [`each_copy_of_an_objects_transform_carries_its_fills_or_strokes_shadow`].
#[test]
fn an_objects_transform_moves_its_fills_shadows() {
    let cases = [
        (json!({"scaleH": 200, "scaleV": 100}), (20.0, 10.0)),
        (json!({"scaleH": 200, "scaleV": 100, "strokes": true}), (20.0, 10.0)),
        (json!({"rotate": 90, "patterns": true}), (10.0, -10.0)),
        (json!({"rotate": 90, "strokes": true}), (10.0, -10.0)),
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

/// `n` with only its fill (or only a 4 pt blue stroke, with `stroke`), whose own effects are
/// `effects`.
fn one_row(mut n: Node, stroke: bool, effects: Vec<Effect>) -> Node {
    let mut item = if stroke {
        vectorcraft_doc::AppearanceItem::Stroke(vectorcraft_doc::StrokeLayer::new(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 4.0))
    } else {
        n.appearance.items.remove(0)
    };
    *item.effects_mut() = effects;
    n.appearance.items = vec![item];
    n
}

/// A case of [`copying_cases`]: the params, each copy's offset and each copy's blur.
type CopyingCase = (Value, Vec<(f64, f64)>, Vec<f64>);

/// The cases of a Transform effect that makes copies with a drop shadow (10, 0) and a blur of 4
/// before it: the params, each copy's offset and each copy's blur, the last copy first.
fn copying_cases() -> Vec<CopyingCase> {
    vec![
        (json!({"moveH": 100, "copies": 2}), vec![(10.0, 0.0), (10.0, 0.0), (10.0, 0.0)], vec![4.0, 4.0, 4.0]),
        // A turn of 90° counterclockwise on screen takes an offset to the right to one upward.
        (json!({"rotate": 90, "copies": 2}), vec![(-10.0, 0.0), (0.0, -10.0), (10.0, 0.0)], vec![4.0, 4.0, 4.0]),
        (json!({"reflectX": true, "moveH": 100, "copies": 1}), vec![(-10.0, 0.0), (10.0, 0.0)], vec![4.0, 4.0]),
        (json!({"scaleH": 200, "scaleV": 200, "moveH": 100, "copies": 1}), vec![(20.0, 0.0), (10.0, 0.0)], vec![8.0, 4.0]),
        // Scale Strokes & Effects does not scale the shadow a second time, nor leave it unturned.
        (
            json!({"scaleH": 200, "scaleV": 200, "rotate": 90, "moveH": 100, "copies": 1, "strokes": true}),
            vec![(0.0, -20.0), (10.0, 0.0)],
            vec![8.0, 4.0],
        ),
        (
            json!({"scaleH": 200, "scaleV": 200, "rotate": 90, "moveH": 100, "copies": 1, "patterns": true}),
            vec![(0.0, -20.0), (10.0, 0.0)],
            vec![8.0, 4.0],
        ),
    ]
}

/// Check that `n` is drawn, expanded and baked as `want.len()` paths, each with one shadow at the
/// offset and blur `want` and `blurs` give it, the last copy first.
fn assert_copies_carry(n: &Node, want: &[(f64, f64)], blurs: &[f64], what: &str) {
    assert!(crate::has_transform_art(n), "{what}");
    for (how, art) in
        [("drawn", crate::reshape(None, n, None).unwrap()), ("expanded", expand_outlined(None, n, None).unwrap()), ("baked", baked(n.clone()))]
    {
        assert_eq!(paths_in(&art), want.len(), "{what}: {how} {art:?}");
        let drawn = shadows_in(&art);
        assert!(near(&drawn, want), "{what}: {how} {drawn:?}");
        let b = shadow_blurs_in(&art);
        assert!(b.len() == blurs.len() && b.iter().zip(blurs).all(|(x, y)| (x - y).abs() < 1e-9), "{what}: {how} blurs {b:?}");
    }
}

/// A fill's or stroke's own Transform effect that makes copies gives each copy the shadow listed
/// before it in that fill's or stroke's effects, moved, turned, reflected and scaled with that copy,
/// the last copy lowest: drawn, expanded and baked. The original keeps the shadow as it is.
#[test]
fn each_copy_of_a_fills_or_strokes_own_transform_carries_the_shadow_before_it() {
    for stroke in [false, true] {
        for (params, want, blurs) in copying_cases() {
            let n = one_row(square(95, vec![]), stroke, vec![shadow(10.0, 0.0, 4.0), transform(params.clone())]);
            assert_copies_carry(&n, &want, &blurs, &format!("stroke {stroke}: {params}"));
        }
        // A Transform effect that makes copies before the shadow: its copies make one piece, which
        // the Transform effect after the shadow copies, each copy with its own shadow.
        let effects =
            vec![transform(json!({"moveV": 100, "copies": 1})), shadow(10.0, 0.0, 4.0), transform(json!({"rotate": 90, "moveH": 200, "copies": 1}))];
        let n = one_row(square(95, vec![]), stroke, effects);
        assert!(crate::has_transform_art(&n), "stroke {stroke}");
        for (how, art) in
            [("drawn", crate::reshape(None, &n, None).unwrap()), ("expanded", expand_outlined(None, &n, None).unwrap()), ("baked", baked(n.clone()))]
        {
            let drawn = shadows_in(&art);
            assert!(near(&drawn, &[(0.0, -10.0), (10.0, 0.0)]), "stroke {stroke}: two copying Transform effects, {how} {drawn:?}");
        }
    }
}

/// A Transform effect that makes copies, listed after a fill's or stroke's first Transform effect
/// with Transform Patterns on, gives each copy the shadow listed before it, as the first one's
/// copies do, with or without options of its own: drawn, expanded and baked.
#[test]
fn each_copy_of_a_later_copying_transform_carries_the_shadow_before_it() {
    for stroke in [false, true] {
        for (params, want, blurs) in copying_cases() {
            let effects = vec![transform(json!({"patterns": true})), shadow(10.0, 0.0, 4.0), transform(params.clone())];
            let n = one_row(square(102, vec![]), stroke, effects);
            assert_copies_carry(&n, &want, &blurs, &format!("stroke {stroke}: {params}"));
        }
    }
}

/// A fill's or stroke's shadow under its object's Transform effect that makes copies is carried by
/// each copy, moved, turned, reflected and scaled with it: for a single fill, a single stroke, and
/// a fill and a stroke that paint each copy below the one before. Drawn, expanded and baked.
#[test]
fn each_copy_of_an_objects_transform_carries_its_fills_or_strokes_shadow() {
    for (params, want, blurs) in copying_cases() {
        for stroke in [false, true] {
            let mut n = one_row(square(96, vec![]), stroke, vec![shadow(10.0, 0.0, 4.0)]);
            n.appearance.effects = vec![transform(params.clone())];
            assert_copies_carry(&n, &want, &blurs, &format!("stroke {stroke}: {params}"));
        }
        // Both a fill and a stroke: only the fill has the shadow.
        let mut n = square(97, vec![transform(params.clone())]);
        if let Some(vectorcraft_doc::AppearanceItem::Stroke(s)) = n.appearance.items.get_mut(1) {
            s.paint = Paint::solid(Color::BLACK);
            s.width = 2.0;
        }
        *n.appearance.items[0].effects_mut() = vec![shadow(10.0, 0.0, 4.0)];
        assert!(crate::has_transform_art(&n), "{params}");
        for (how, art) in
            [("drawn", crate::reshape(None, &n, None).unwrap()), ("expanded", expand_outlined(None, &n, None).unwrap()), ("baked", baked(n.clone()))]
        {
            let drawn = shadows_in(&art);
            assert!(near(&drawn, &want), "layered {params}: {how} {drawn:?}");
        }
    }
}

/// A shadow listed after a fill's own Transform effect that makes copies paints once around all of
/// them, and one listed before it on each copy. A Transform effect that makes no copies, or has
/// Transform Objects off, draws the fill as before, without art.
#[test]
fn a_fills_shadow_after_its_own_copying_transform_paints_once_around_the_copies() {
    let effects = vec![shadow(10.0, 0.0, 0.0), transform(json!({"rotate": 90, "copies": 1})), shadow(0.0, 5.0, 0.0)];
    let n = one_row(square(98, vec![]), false, effects);
    for (how, art) in
        [("drawn", crate::reshape(None, &n, None).unwrap()), ("expanded", expand_outlined(None, &n, None).unwrap()), ("baked", baked(n.clone()))]
    {
        assert!(near(&shadows_in(&art), &[(0.0, 5.0), (0.0, -10.0), (10.0, 0.0)]), "{how}: {:?}", shadows_in(&art));
        assert_eq!(paths_in(&art), 2, "{how}");
    }
    for params in [json!({"rotate": 90}), json!({"rotate": 90, "copies": 1, "objects": false})] {
        let n = one_row(square(99, vec![]), false, vec![shadow(10.0, 0.0, 0.0), transform(params.clone())]);
        assert!(!crate::has_transform_art(&n), "{params}");
        assert!(!crate::has_copied_row_raster(&n), "{params}");
    }
    // A shadow between two Transform effects that make copies paints on what the second one
    // copies, so the fill is drawn as before, without art.
    let copies = transform(json!({"rotate": 90, "copies": 1}));
    let n = one_row(square(99, vec![]), false, vec![shadow(10.0, 0.0, 0.0), copies.clone(), shadow(0.0, 5.0, 0.0), copies]);
    assert!(!crate::has_copied_row_raster(&n));
    assert!(!crate::has_transform_art(&n));
}

/// Copies of a fill's own Transform effect that stop at the effect budget each keep their shadow,
/// and so do those of an object's Transform effect for its fill's shadow.
#[test]
fn row_copies_past_the_budget_each_keep_their_shadow() {
    let copies = transform(json!({"moveV": 50, "copies": 1000}));
    let own = one_row(path_node(100, wavy(30_000), vec![]), false, vec![shadow(1.0, 0.5, 0.0), copies.clone()]);
    let mut owner = one_row(path_node(101, wavy(30_000), vec![]), false, vec![shadow(1.0, 0.5, 0.0)]);
    owner.appearance.effects = vec![copies];
    for n in [own, owner] {
        let art = crate::reshape(None, &n, None).unwrap();
        let paths = paths_in(&art);
        assert!(paths > 1 && paths < 1001, "{paths}");
        assert_eq!(shadows_in(&art).len(), paths);
    }
}

/// Black 40 pt "IIII" with its baseline at (5, 60) and `effects`.
fn type_node(id: u64, effects: Vec<Effect>) -> Node {
    use vectorcraft_doc::text::{CharStyle, TextObject};
    let style = CharStyle { size: 40.0, fill: Paint::solid(Color::BLACK), ..Default::default() };
    let mut n = Node::new(NodeId(id), vectorcraft_doc::NodeKind::Text(Box::new(TextObject::point(Point::new(5.0, 60.0), "IIII", style))));
    n.appearance.effects = effects;
    n
}

/// An instance at (50, 50) with `effects` of a symbol, and the symbol's art: a black 20 pt square
/// centered on its origin.
fn instance(id: u64, effects: Vec<Effect>) -> (Node, Node) {
    let art = Node::path(
        NodeId(900),
        shapes::rectangle(Rect::new(-10.0, -10.0, 10.0, 10.0)),
        Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
    );
    let mut n = Node::new(NodeId(id), vectorcraft_doc::NodeKind::SymbolInstance { symbol: "Square".into(), xf: Affine::translate((50.0, 50.0)) });
    n.appearance.effects = effects;
    (n, art)
}

/// Type, an image and a symbol instance with a Transform effect that makes copies give each copy
/// the shadow listed before it, turned and scaled with that copy, the last copy lowest: drawn and
/// expanded. The object paints none around all of them, and a shadow listed after the Transform
/// effect still paints once around them. Built without fonts, type lays out no glyphs and only the
/// image and the instance are checked.
#[test]
fn each_copy_of_type_an_image_or_a_symbol_instance_carries_the_shadow_listed_before_it() {
    let fonts = crate::outline_text(&type_node(95, vec![])).is_some_and(|o| paths_in(&o) > 0);
    if !fonts {
        eprintln!("type skipped: built without fonts (enable vectorcraft-text's bundled-fonts to run it)");
    }
    let objects = |effects: Vec<Effect>| {
        let (inst, art) = instance(96, effects.clone());
        let mut out = vec![("image", image(97, effects.clone()), None), ("instance", inst, Some(art))];
        if fonts {
            out.push(("type", type_node(95, effects), None));
        }
        out
    };
    let cases = [
        (json!({"rotate": 90, "copies": 2}), vec![(-10.0, 0.0), (0.0, -10.0), (10.0, 0.0)], vec![4.0, 4.0, 4.0]),
        (json!({"scaleH": 200, "scaleV": 200, "moveH": 100, "copies": 1}), vec![(20.0, 0.0), (10.0, 0.0)], vec![8.0, 4.0]),
    ];
    for (params, want, blurs) in cases {
        for (kind, n, symbol) in objects(vec![shadow(10.0, 0.0, 4.0), transform(params.clone())]) {
            assert!(crate::outer_effects(&n).iter().all(|e| e.id != "stylize.dropShadow"), "{kind} {params}");
            let symbol = symbol.as_ref();
            for (how, drawn) in [("drawn", crate::reshape(None, &n, symbol)), ("expanded", expand_outlined(None, &n, symbol))] {
                let drawn = drawn.unwrap_or_else(|| panic!("{kind} {params}: {how}"));
                assert_eq!(drawn.children().map_or(0, Vec::len), want.len(), "{kind} {params}: {how} copies");
                assert!(drawn.appearance.effects.is_empty(), "{kind} {params}: {how} {:?}", drawn.appearance.effects);
                assert!(near(&shadows_in(&drawn), &want), "{kind} {params}: {how} {:?}", shadows_in(&drawn));
                let b = shadow_blurs_in(&drawn);
                assert!(b.len() == blurs.len() && b.iter().zip(&blurs).all(|(x, y)| (x - y).abs() < 1e-9), "{kind} {params}: {how} blurs {b:?}");
            }
        }
    }
    let effects = vec![shadow(10.0, 0.0, 0.0), transform(json!({"rotate": 90, "copies": 1})), shadow(0.0, 5.0, 0.0)];
    for (kind, n, symbol) in objects(effects) {
        assert_eq!(crate::outer_effects(&n).len(), 2, "{kind}");
        let drawn = crate::reshape(None, &n, symbol.as_ref()).unwrap();
        assert!(near(&shadows_in(&drawn), &[(0.0, 5.0), (0.0, -10.0), (10.0, 0.0)]), "{kind}: {:?}", shadows_in(&drawn));
    }
}

/// Type whose Transform effect only moves its copies, baked for export, gives each copy the shadow
/// listed before that effect, as the canvas draws it, rather than staying type under one shadow.
#[test]
fn baked_type_whose_transform_moves_its_copies_keeps_a_shadow_on_each() {
    if !crate::outline_text(&type_node(99, vec![])).is_some_and(|o| paths_in(&o) > 0) {
        eprintln!("skipped: built without fonts (enable vectorcraft-text's bundled-fonts to run it)");
        return;
    }
    let n = type_node(99, vec![shadow(10.0, 0.0, 0.0), transform(json!({"moveH": 150, "copies": 1}))]);
    let drawn = baked(n);
    assert!(near(&shadows_in(&drawn), &[(10.0, 0.0), (10.0, 0.0)]), "{:?}", shadows_in(&drawn));
}

/// Type with a fill whose own geometry effects reshape it keeps one set of raster effects around
/// all the copies.
#[test]
fn type_with_a_reshaped_fill_keeps_one_shadow_around_its_copies() {
    let effects = vec![shadow(10.0, 0.0, 0.0), transform(json!({"moveH": 100, "copies": 1}))];
    let mut filled = type_node(98, effects.clone());
    filled.appearance.items.push(vectorcraft_doc::AppearanceItem::Fill(vectorcraft_doc::FillLayer {
        effects: vec![fx("path.offsetPath", json!({"offset": 2}))],
        ..vectorcraft_doc::FillLayer::new(Paint::solid(Color::BLACK))
    }));
    assert_eq!(crate::outer_effects(&filled).len(), 2);
}

/// Type whose own fill's Transform effect makes copies is drawn as its outlines, and each copy
/// carries the shadow listed before that Transform effect, as a path's copies do: for the fill's
/// first Transform effect and for one listed after a Transform effect with Transform Patterns on.
/// Drawn, expanded and baked.
#[test]
fn type_carries_its_fills_shadow_on_each_copy() {
    let filled = |mut n: Node, effects: Vec<Effect>| {
        n.appearance.items.push(vectorcraft_doc::AppearanceItem::Fill(vectorcraft_doc::FillLayer {
            effects,
            ..vectorcraft_doc::FillLayer::new(Paint::solid(Color::BLACK))
        }));
        n
    };
    let copying = vec![shadow(10.0, 0.0, 0.0), transform(json!({"moveH": 100, "copies": 1}))];
    if !crate::outline_text(&type_node(95, vec![])).is_some_and(|o| paths_in(&o) > 0) {
        eprintln!("type skipped: built without fonts (enable vectorcraft-text's bundled-fonts to run it)");
        return;
    }
    let later = vec![transform(json!({"patterns": true})), shadow(10.0, 0.0, 0.0), transform(json!({"moveH": 100, "copies": 1}))];
    for (what, effects) in [("first", copying), ("later", later)] {
        let n = filled(type_node(95, vec![]), effects);
        for (how, art) in
            [("drawn", crate::reshape(None, &n, None)), ("expanded", expand_outlined(None, &n, None)), ("baked", Some(baked(n.clone())))]
        {
            let art = art.unwrap_or_else(|| panic!("{what}: {how}"));
            assert!(near(&shadows_in(&art), &[(10.0, 0.0), (10.0, 0.0)]), "{what}: {how} {:?}", shadows_in(&art));
            // The two shadows are on two objects 100 pt apart: the type and its copy.
            let mut lefts = vec![];
            art.walk(&mut |m| {
                let lists = std::iter::once(&m.appearance.effects).chain(m.appearance.items.iter().map(|i| i.effects()));
                let shadows = lists.map(|l| l.iter().filter(|e| e.id == "stylize.dropShadow").count()).sum::<usize>();
                if let Some(b) = m.geometric_bounds() {
                    lefts.extend(std::iter::repeat_n(b.x0, shadows));
                }
            });
            lefts.sort_by(f64::total_cmp);
            assert!(lefts.len() == 2 && (lefts[1] - lefts[0] - 100.0).abs() < 1e-6, "{what}: {how} {lefts:?}");
        }
    }
}
