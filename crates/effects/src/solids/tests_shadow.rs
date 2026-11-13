use serde_json::json;
use vectorcraft_doc::color::Color;
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_geom::{Affine, BezPath};

use super::*;

const RED: Color = Color::Rgb { r: 0.8, g: 0.2, b: 0.1 };

/// The 3D and Materials effects that cast shadows.
const CASTING: [&str; 4] = [EXTRUDE, INFLATE, ROTATE, crate::REVOLVE];

fn square_node(id: u64, dx: f64) -> Node {
    let mut b = BezPath::new();
    b.move_to((100.0 + dx, 100.0));
    b.line_to((200.0 + dx, 100.0));
    b.line_to((200.0 + dx, 200.0));
    b.line_to((100.0 + dx, 200.0));
    b.close_path();
    Node::path(NodeId(id), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(RED), Paint::None, 0.0))
}

fn art(n: &Node, id: &str) -> Option<Node> {
    if id == crate::REVOLVE { crate::revolve_art(n) } else { solid_art(n) }
}

/// The opacity of shadow `n`: a path filled black and nothing else; `None` for any other node.
fn shadow_opacity(n: &Node) -> Option<f32> {
    let [AppearanceItem::Fill(fill)] = n.appearance.items.as_slice() else { return None };
    (matches!(n.kind, NodeKind::Path { .. }) && fill.paint.color() == Some(Color::BLACK)).then_some(fill.opacity)
}

/// Whether `n` is the shadow of opaque art: a path filled black at 50 %, nothing else.
fn is_shadow(n: &Node) -> bool {
    shadow_opacity(n) == Some(0.5)
}

fn shadows(n: &Node) -> usize {
    let mut count = 0;
    n.walk(&mut |c| count += usize::from(is_shadow(c)));
    count
}

#[test]
fn each_3d_effect_draws_its_shadow_behind_its_solid_when_shadows_are_on() {
    for id in CASTING {
        let mut n = square_node(1, 0.0);
        n.appearance.effects.push(crate::new_effect(id, &json!({})).unwrap());
        let plain = art(&n, id).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(shadows(&plain), 0, "{id}: off by default");
        n.appearance.effects[0].params = json!({"shadow": true, "shadowDistance": 50});
        let lit = art(&n, id).unwrap_or_else(|| panic!("{id}"));
        let children = lit.children().unwrap();
        assert!(is_shadow(&children[0]), "{id}: the shadow is drawn first");
        assert_eq!(shadows(&lit), 1, "{id}");
        // The default light is up and to the left: the shadow on a plane away from the object
        // reaches past the solid down and right.
        let shadow = children[0].visual_bounds().unwrap();
        let solid = children[1..].iter().filter_map(|c| c.visual_bounds()).reduce(|a, b| a.union(b)).unwrap();
        assert!(shadow.x1 > solid.x1 && shadow.y1 > solid.y1, "{id}: {shadow:?} {solid:?}");
        // The art's bounds take the shadow in.
        assert!(lit.visual_bounds().unwrap().contains_rect(shadow), "{id}");
    }
}

#[test]
fn the_shadow_moves_with_its_settings() {
    for id in CASTING {
        let bounds = |params: Value| {
            let mut n = square_node(1, 0.0);
            n.appearance.effects.push(crate::new_effect(id, &params).unwrap());
            let lit = art(&n, id).unwrap();
            let shadow = &lit.children().unwrap()[0];
            assert!(is_shadow(shadow), "{id} {params}");
            shadow.visual_bounds().unwrap()
        };
        let near = bounds(json!({"shadow": true}));
        let far = bounds(json!({"shadow": true, "shadowDistance": 100}));
        assert!(far.x1 > near.x1 + 1.0, "{id}: {near:?} {far:?}");
        // With the light straight in front, the outline falls about the plane's center; at 10 %
        // the plane cuts it to a square 0.4 times the object's size across.
        let tight = bounds(json!({"shadow": true, "shadowBounds": 10, "lightHeight": 90}));
        let wide = bounds(json!({"shadow": true, "shadowBounds": 400, "lightHeight": 90}));
        assert!(wide.width() > tight.width() * 2.0, "{id}: {tight:?} {wide:?}");
        let below = bounds(json!({"shadow": true, "shadowPosition": "below", "rotationX": -30}));
        assert_ne!(below, near, "{id}");
    }
}

#[test]
fn shadow_settings_out_of_range_or_of_the_wrong_kind_are_refused() {
    for id in CASTING {
        for (params, error) in [
            (json!({"shadow": "yes"}), "shadow must be true or false"),
            (json!({"shadowPosition": "under"}), "shadowPosition must be behind or below"),
            (json!({"shadowPosition": 1}), "shadowPosition must be behind or below"),
            (json!({"shadowDistance": 101}), "shadowDistance must be a number from 0 to 100"),
            (json!({"shadowDistance": -1}), "shadowDistance must be a number from 0 to 100"),
            (json!({"shadowBounds": 5}), "shadowBounds must be a number from 10 to 400"),
            (json!({"shadowBounds": "wide"}), "shadowBounds must be a number from 10 to 400"),
        ] {
            let err = validate_three_d_params(id, &params).unwrap_err();
            assert!(err.contains(error), "{id} {params}: {err}");
        }
        let ok = json!({"shadow": true, "shadowPosition": "below", "shadowDistance": 100, "shadowBounds": 10});
        assert!(validate_three_d_params(id, &ok).is_ok(), "{id}");
    }
}

#[test]
fn a_group_casts_one_shadow_for_all_its_members() {
    for id in CASTING {
        let mut g = Node::group(NodeId(9), vec![Arc::new(square_node(1, 0.0)), Arc::new(square_node(2, 200.0))]);
        g.appearance.effects.push(crate::new_effect(id, &json!({"shadow": true, "lightHeight": 90})).unwrap());
        let lit = art(&g, id).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(shadows(&lit), 1, "{id}");
        let shadow = lit.children().unwrap()[0].clone();
        assert!(is_shadow(&shadow), "{id}");
        // With the light straight in front, the shadow behind spans both members.
        let b = shadow.visual_bounds().unwrap();
        assert!(b.x0 < 200.0 && b.x1 > 300.0, "{id}: {b:?}");
    }
}

/// A member's opacity mask applies to the part of the shadow it casts, as to its solid: that part
/// is drawn in the mask, the other member's outside it, both on one plane.
#[test]
fn a_masked_member_casts_its_part_of_the_shadow_in_its_mask() {
    for id in CASTING {
        let mut masked = square_node(2, 200.0);
        let mut cover = square_node(3, 200.0);
        cover.appearance.items = Appearance::basic(Paint::solid(Color::WHITE), Paint::None, 0.0).items;
        masked.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(cover, true)));
        let mut g = Node::group(NodeId(9), vec![Arc::new(square_node(1, 0.0)), Arc::new(masked)]);
        g.appearance.effects.push(crate::new_effect(id, &json!({"shadow": true})).unwrap());
        let lit = art(&g, id).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(shadows(&lit), 2, "{id}: one piece for each member");
        let mut in_mask = 0;
        lit.walk(&mut |c| {
            if c.extras().mask.is_some() {
                c.walk(&mut |s| in_mask += usize::from(is_shadow(s)));
            }
        });
        assert_eq!(in_mask, 1, "{id}");
    }
}

#[test]
fn transparent_art_and_a_light_that_lights_nothing_cast_no_shadow() {
    for id in CASTING {
        let mut n = square_node(1, 0.0);
        n.appearance.effects.push(crate::new_effect(id, &json!({"shadow": true, "shade": false})).unwrap());
        assert_eq!(shadows(&art(&n, id).unwrap()), 0, "{id}: Shading off");
        n.appearance.effects[0].params = json!({"shadow": true, "lightIntensity": 0});
        assert_eq!(shadows(&art(&n, id).unwrap()), 0, "{id}: no intensity");
        // A member at 0 % opacity casts nothing; the other member still casts its shadow.
        let mut clear = square_node(2, 200.0);
        clear.opacity = 0.0;
        let mut g = Node::group(NodeId(9), vec![Arc::new(square_node(1, 0.0)), Arc::new(clear)]);
        g.appearance.effects.push(crate::new_effect(id, &json!({"shadow": true, "lightHeight": 90})).unwrap());
        let lit = art(&g, id).unwrap_or_else(|| panic!("{id}"));
        let shadow = lit.children().unwrap()[0].clone();
        assert!(is_shadow(&shadow), "{id}");
        let b = shadow.visual_bounds().unwrap();
        assert!(b.x1 < 300.0, "{id}: only the visible member casts: {b:?}");
        // Art at 50 % casts a shadow half as dark.
        let mut half = square_node(1, 0.0);
        for item in &mut half.appearance.items {
            if let AppearanceItem::Fill(fill) = item {
                fill.opacity = 0.5;
            }
        }
        half.appearance.effects.push(crate::new_effect(id, &json!({"shadow": true})).unwrap());
        let lit = art(&half, id).unwrap();
        assert_eq!(shadow_opacity(&lit.children().unwrap()[0]), Some(0.25), "{id}");
        // Art in a blending mode casts its shadow in that mode.
        for item in &mut half.appearance.items {
            if let AppearanceItem::Fill(fill) = item {
                fill.blend = BlendMode::Screen;
            }
        }
        let lit = art(&half, id).unwrap();
        let AppearanceItem::Fill(fill) = &lit.children().unwrap()[0].appearance.items[0] else { panic!("{id}: a fill") };
        assert_eq!(fill.blend, BlendMode::Screen, "{id}");
    }
}

/// With a 3D effect on each of two fills, both fills' shadows are drawn behind both fills' solids.
#[test]
fn the_shadows_of_every_fill_go_behind_every_fill_s_solid() {
    for id in CASTING {
        let mut n = square_node(1, 0.0);
        let fill = n.appearance.items.iter().find(|i| matches!(i, AppearanceItem::Fill(_))).unwrap().clone();
        n.appearance.items = vec![fill.clone(), fill];
        for item in &mut n.appearance.items {
            item.effects_mut().push(crate::new_effect(id, &json!({"shadow": true})).unwrap());
        }
        let lit = art(&n, id).unwrap_or_else(|| panic!("{id}"));
        let children = lit.children().unwrap();
        assert!(children.len() >= 4, "{id}");
        assert!(is_shadow(&children[0]) && is_shadow(&children[1]), "{id}: both shadows first");
        assert!(children[2..].iter().all(|c| !is_shadow(c)), "{id}");
    }
}

#[test]
fn expand_appearance_keeps_the_shadow_as_a_path() {
    for id in CASTING {
        let mut d = Document::new(400.0, 400.0);
        let mut n = square_node(1, 0.0);
        n.id = d.alloc_id();
        n.appearance.effects.push(crate::new_effect(id, &json!({"shadow": true})).unwrap());
        let mut stroke = |_: &mut Document, _: &PathData, _: FillRule, _: &StrokeLayer| None;
        let expanded = crate::expand_leaf(&mut d, &n, &mut stroke).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(shadows(&expanded), 1, "{id}");
        let mut effects = 0;
        expanded.walk(&mut |c| effects += c.appearance.effects.len());
        assert_eq!(effects, 0, "{id}");
        // It moves with the object, as the solid does.
        let before = art(&n, id).unwrap().children().unwrap()[0].visual_bounds().unwrap();
        n.transform(Affine::translate((40.0, 0.0)), false);
        let after = art(&n, id).unwrap().children().unwrap()[0].visual_bounds().unwrap();
        assert!((after.x0 - before.x0 - 40.0).abs() < 1e-6, "{id}: {before:?} {after:?}");
    }
}
