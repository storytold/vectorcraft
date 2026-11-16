use serde_json::json;
use vectorcraft_doc::color::Color;
use vectorcraft_doc::{Document, NodeId, StrokeLayer};
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

/// The radius of the Gaussian Blur on shadow `n`; `None` without one.
fn shadow_blur(n: &Node) -> Option<f64> {
    match n.appearance.effects.as_slice() {
        [] => None,
        [e] if e.id == "blur.gaussian" && e.visible => e.params["radius"].as_f64(),
        other => panic!("unexpected effects on the shadow: {other:?}"),
    }
}

/// The first light's Softness blurs the shadow, more with Softness and with Distance from Object;
/// at 0 % it stays the hard-edged path. A list of lights takes the first light's Softness.
#[test]
fn the_first_lights_softness_blurs_the_shadow() {
    for id in CASTING {
        let blur = |params: Value| {
            let mut n = square_node(1, 0.0);
            n.appearance.effects.push(crate::new_effect(id, &params).unwrap());
            let lit = art(&n, id).unwrap();
            let shadow = &lit.children().unwrap()[0];
            assert!(is_shadow(shadow), "{id} {params}");
            shadow_blur(shadow)
        };
        assert_eq!(blur(json!({"shadow": true})), None, "{id}: hard at 0 %");
        let soft = blur(json!({"shadow": true, "lightSoftness": 50})).unwrap_or_else(|| panic!("{id}"));
        let softer = blur(json!({"shadow": true, "lightSoftness": 100})).unwrap();
        let far = blur(json!({"shadow": true, "lightSoftness": 100, "shadowDistance": 100})).unwrap();
        assert!(soft > 0.0 && softer > soft && far > softer, "{id}: {soft} {softer} {far}");
        let first = json!({"shadow": true, "lightSoftness": 0, "lights": [{"softness": 100}, {"softness": 0}]});
        assert_eq!(blur(first), Some(softer), "{id}: the first light's");
        let second = json!({"shadow": true, "lightSoftness": 100, "lights": [{"softness": 0}, {"softness": 100}]});
        assert_eq!(blur(second), None, "{id}: only the first light casts");
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

/// A fill's own 3D effect blurs its shadow by the light's Softness too, and the effects after it
/// in the fill's list follow the blur.
#[test]
fn a_fills_own_3d_effect_keeps_the_blur_below_its_later_effects() {
    for id in CASTING {
        for later in [None, Some(crate::new_effect("distort.roughen", &json!({})).unwrap())] {
            let mut n = square_node(1, 0.0);
            let fill = n.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Fill(_))).unwrap();
            fill.effects_mut().push(crate::new_effect(id, &json!({"shadow": true, "lightSoftness": 100})).unwrap());
            fill.effects_mut().extend(later.clone());
            let lit = art(&n, id).unwrap_or_else(|| panic!("{id}"));
            let shadow = &lit.children().unwrap()[0];
            assert!(is_shadow(shadow), "{id}");
            let ids: Vec<&str> = shadow.appearance.effects.iter().map(|e| e.id.as_str()).collect();
            let want: Vec<&str> = std::iter::once("blur.gaussian").chain(later.as_ref().map(|e| e.id.as_str())).collect();
            assert_eq!(ids, want, "{id}");
        }
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

/// A square filled red and stroked blue, its stroke as opaque as `stroke`.
fn stroked_square(id: u64, dx: f64, stroke: f32) -> Node {
    let mut n = square_node(id, dx);
    n.appearance = Appearance::basic(Paint::solid(RED), Paint::solid(Color::rgb(0.1, 0.2, 0.9)), 6.0).into();
    for item in &mut n.appearance.items {
        if let AppearanceItem::Stroke(s) = item {
            s.opacity = stroke;
        }
    }
    n
}

/// Objects with 3D effect `id` and its shadow on (`params`): a filled and stroked square, one whose
/// stroke is half transparent, one whose stroke is fully transparent, and a group of two members,
/// one of them at half opacity.
fn shadowed(id: &str, params: &Value) -> Vec<Node> {
    let with = |mut n: Node| {
        n.appearance.effects.push(crate::new_effect(id, params).unwrap());
        n
    };
    let mut half = stroked_square(2, 200.0, 1.0);
    half.opacity = 0.5;
    let group = Node::group(NodeId(9), vec![Arc::new(stroked_square(1, 0.0, 1.0)), Arc::new(half)]);
    vec![with(stroked_square(1, 0.0, 1.0)), with(stroked_square(1, 0.0, 0.5)), with(stroked_square(1, 0.0, 0.0)), with(group)]
}

/// The settings the shadows of effect `id` are compared with: the defaults, turned with
/// perspective, and under a colored light, below the object for Extrude & Bevel, Inflate and
/// Rotate and behind it for Revolve.
fn shadow_settings(id: &str) -> [Value; 3] {
    let position = if id == crate::REVOLVE { "behind" } else { "below" };
    [
        json!({"shadow": true}),
        json!({"shadow": true, "rotationX": -25, "rotationY": 35, "perspective": 50, "shadowDistance": 40}),
        json!({"shadow": true, "shadowPosition": position, "shadowDistance": 30, "lightColor": "#ff8040", "twist": 30}),
    ]
}

/// How many solids drawing the art of `n` with effect `id` builds.
fn solids_built(n: &Node, id: &str) -> usize {
    let start = SOLIDS_BUILT.with(std::cell::Cell::get);
    assert!(art(n, id).is_some(), "{id}");
    SOLIDS_BUILT.with(std::cell::Cell::get) - start
}

/// Drawing a 3D object with its shadow on builds each of its solids once: the shadow is cast from
/// the faces the solids drawn kept, so it builds no more solids than the art without a shadow.
#[test]
fn a_shadowed_3d_object_builds_each_solid_once() {
    for id in CASTING {
        for params in shadow_settings(id) {
            for (lit, mut plain) in shadowed(id, &params).into_iter().zip(shadowed(id, &params)) {
                plain.appearance.effects.iter_mut().for_each(|e| e.params["shadow"] = json!(false));
                let mut cast = 0;
                art(&lit, id).unwrap().walk(&mut |c| cast += usize::from(shadow_opacity(c).is_some()));
                assert!(cast > 0, "{id} {params}");
                let built = solids_built(&plain, id);
                assert!(built > 0, "{id}");
                assert_eq!(solids_built(&lit, id), built, "{id} {params}: the shadow builds no solid of its own");
            }
        }
    }
}

/// The shadow of the faces Extrude & Bevel, Inflate and Rotate kept when they drew their solids,
/// to the bit: the shadow [`cast_shadow`] casts from solids built for the shadow alone.
#[test]
fn the_shadow_cast_from_the_solids_drawn_is_the_shadow_of_solids_built_for_it() {
    for id in [EXTRUDE, INFLATE, ROTATE] {
        for params in shadow_settings(id) {
            for n in shadowed(id, &params) {
                let (_, fx) = solid(&n.appearance.effects).unwrap();
                let shapes = if is_container(&n) {
                    members(&n, "").unwrap()
                } else {
                    let (path, rule) = profile(&n).unwrap();
                    vec![Profile { path, rule, items: solid_paint_sources(&n).1, opacity: 1.0, scopes: vec![] }]
                };
                let (drawn, _) = art_for(None, &n, fx, &shapes).unwrap();
                let p = crate::merged_params(&fx.id, &fx.params);
                let (caster, settings, cast) = cast_shadow(&fx.id, &p, &shapes).unwrap().unwrap();
                let alone = shadow::scoped_art(n.id, &caster, settings, cast, has_masks(&shapes));
                assert!(!alone.is_empty(), "{id} {params}");
                assert_eq!(format!("{drawn:?}"), format!("{alone:?}"), "{id} {params}");
            }
        }
    }
}

/// A stroke's solid left out of the art (past its own limits) keeps no faces, and the shadow cast
/// from the faces the solids kept leaves it out without building it: it casts nothing, as it draws
/// nothing, and the fill's solid still casts.
#[test]
fn a_stroke_left_out_of_the_art_casts_no_shadow() {
    for id in [EXTRUDE, INFLATE] {
        let n = stroked_square(1, 0.0, 1.0);
        let (path, rule) = profile(&n).unwrap();
        let shapes = [Profile { path, rule, items: solid_paints(&n), opacity: 1.0, scopes: vec![] }];
        let p = crate::merged_params(id, &json!({"shadow": true}));
        let (parts, bounds) = solid_parts(&shapes, union_bounds(&shapes).unwrap());
        let mut built: Vec<Option<SolidFaces>> =
            built_solids_cast(id, &p, &parts, bounds, MAX_OBJECT_FACES, true, None).map(|r| r.unwrap().and_then(|(_, faces)| faces)).collect();
        let stroke = parts.iter().position(|(item, ..)| matches!(item, AppearanceItem::Stroke(_))).unwrap();
        assert!(built.len() == 2 && built.iter().all(Option::is_some), "{id}");
        let (both, _, cast) = cast_built(id, &p, &shapes, built.clone()).unwrap().unwrap();
        assert_eq!(cast.len(), 2, "{id}: the fill and the stroke cast");
        built[stroke] = None;
        let start = SOLIDS_BUILT.with(std::cell::Cell::get);
        let (fill, _, cast) = cast_built(id, &p, &shapes, built).unwrap().unwrap();
        assert_eq!(SOLIDS_BUILT.with(std::cell::Cell::get), start, "{id}: no solid is built for the shadow");
        assert_eq!(cast.len(), 1, "{id}: the stroke left out casts nothing");
        assert!(!fill.is_empty() && fill.len() < both.len(), "{id}: only the fill's faces are cast");
    }
}

/// A command accepts each shadowed object that draws with its shadow, and with Rotate builds the
/// plane of each visible fill and stroke once for the shadow: a fully transparent one casts nothing.
#[test]
fn the_check_accepts_the_shadowed_objects_that_draw() {
    for id in [EXTRUDE, INFLATE, ROTATE] {
        for params in shadow_settings(id) {
            for n in shadowed(id, &params) {
                let mut drawn = 0;
                art(&n, id).unwrap().walk(&mut |c| drawn += usize::from(shadow_opacity(c).is_some()));
                assert!(drawn > 0, "{id} {params}");
                assert_eq!(validate_solid(&n, id, &params), Ok(()), "{id} {params}");
            }
        }
    }
    let shadow = json!({"shadow": true});
    for (stroke, planes) in [(1.0, 2), (0.5, 2), (0.0, 1)] {
        let n = stroked_square(1, 0.0, stroke);
        let start = SOLIDS_BUILT.with(std::cell::Cell::get);
        assert_eq!(validate_solid(&n, ROTATE, &shadow), Ok(()), "{stroke}");
        assert_eq!(SOLIDS_BUILT.with(std::cell::Cell::get) - start, planes, "Rotate, a stroke at {stroke}");
    }
}

/// In wireframe mode no shadow is drawn, so the check casts none: with Shadows on, Rotate builds no
/// plane for a shadow, and Extrude & Bevel and Inflate build each solid once, as with the shadow off.
#[test]
fn the_check_casts_no_shadow_in_wireframe_mode() {
    let n = stroked_square(1, 0.0, 1.0);
    let wire = json!({"shadow": true, "render": {"mode": "wireframe"}});
    for (id, solids) in [(ROTATE, 0), (EXTRUDE, 2), (INFLATE, 2)] {
        let start = SOLIDS_BUILT.with(std::cell::Cell::get);
        assert_eq!(validate_solid(&n, id, &wire), Ok(()), "{id}");
        assert_eq!(SOLIDS_BUILT.with(std::cell::Cell::get) - start, solids, "{id}");
    }
}

/// The opacity of each part of the shadow of `n` (its 3D effect's shadow on) and the shadow's art,
/// as [`cast_shadow`] casts it from solids built for the shadow alone and as [`cast_built`] casts
/// it from the faces the solids drawn kept.
fn cast_parts(n: &Node) -> [(Vec<f32>, String); 2] {
    let (_, fx) = solid(&n.appearance.effects).unwrap();
    let shapes = if is_container(n) {
        members(n, "").unwrap()
    } else {
        let (path, rule) = profile(n).unwrap();
        vec![Profile { path, rule, items: solid_paint_sources(n).1, opacity: 1.0, scopes: vec![] }]
    };
    let p = crate::merged_params(&fx.id, &fx.params);
    let (_, built) = surfaces_for(None, n, fx, &shapes, true).unwrap();
    [cast_shadow(&fx.id, &p, &shapes), cast_built(&fx.id, &p, &shapes, built)].map(|c| {
        let (caster, settings, cast) = c.unwrap().unwrap();
        let inks = cast.iter().map(|(_, ink, _)| ink.opacity).collect();
        (inks, format!("{:?}", shadow::scoped_art(n.id, &caster, settings, cast, has_masks(&shapes))))
    })
}

/// A fill, a stroke or a member whose opacity is NaN or infinite casts no part of the shadow, as one
/// at 0 % does, whether the shadow is cast from solids built for it or from the solids drawn; the
/// art beside it still casts its part.
#[test]
fn art_whose_opacity_is_not_a_finite_number_casts_no_shadow() {
    // The art with opacity `opacity` in one place: a square's stroke, the fill of a group's second
    // member, and a group's second member.
    let objects = |opacity: f32| {
        let mut fill = square_node(2, 200.0);
        for item in &mut fill.appearance.items {
            if let AppearanceItem::Fill(f) = item {
                f.opacity = opacity;
            }
        }
        let mut member = square_node(2, 200.0);
        member.opacity = opacity;
        [
            stroked_square(1, 0.0, opacity),
            Node::group(NodeId(9), vec![Arc::new(square_node(1, 0.0)), Arc::new(fill)]),
            Node::group(NodeId(9), vec![Arc::new(square_node(1, 0.0)), Arc::new(member)]),
        ]
    };
    for id in [EXTRUDE, INFLATE, ROTATE] {
        for bad in [f32::NAN, f32::INFINITY] {
            for (i, (mut n, mut clear)) in objects(bad).into_iter().zip(objects(0.0)).enumerate() {
                n.appearance.effects.push(crate::new_effect(id, &json!({"shadow": true})).unwrap());
                clear.appearance.effects.push(crate::new_effect(id, &json!({"shadow": true})).unwrap());
                let expected = cast_parts(&clear);
                for (k, (inks, shadow)) in cast_parts(&n).into_iter().enumerate() {
                    assert_eq!(inks, vec![1.0], "{id} {bad} object {i} cast {k}: only the opaque art casts");
                    assert_eq!(shadow, expected[k].1, "{id} {bad} object {i} cast {k}");
                }
                let mut opacities = vec![];
                art(&n, id).unwrap().walk(&mut |c| opacities.extend(shadow_opacity(c)));
                assert_eq!(opacities, vec![0.5], "{id} {bad} object {i}");
            }
        }
    }
}
