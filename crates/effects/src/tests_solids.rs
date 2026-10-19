use serde_json::json;
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_geom::{Affine, BezPath, Point, Shape};

use super::*;

const RED: Color = Color::Rgb { r: 0.8, g: 0.2, b: 0.1 };

fn square_node() -> Node {
    let mut b = BezPath::new();
    b.move_to((100.0, 100.0));
    b.line_to((200.0, 100.0));
    b.line_to((200.0, 200.0));
    b.line_to((100.0, 200.0));
    b.close_path();
    Node::path(NodeId(1), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(RED), Paint::solid(Color::BLACK), 4.0))
}

fn fx(id: &str, params: Value) -> Effect {
    crate::new_effect(id, &params).unwrap()
}

/// The shaded faces of every surface of `art`, bottom first.
fn faces(art: &Node) -> Vec<&Node> {
    art.children().unwrap().iter().flat_map(|s| s.children().unwrap().iter().map(|f| &**f)).collect()
}

fn fill_rgb(n: &Node) -> [f32; 3] {
    match &n.appearance.items[0] {
        AppearanceItem::Fill(f) => match &f.paint {
            Paint::Gradient(g) => g.gradient.sample(0.5).0.to_rgb(),
            p => p.color().unwrap().to_rgb(),
        },
        _ => panic!("a fill first"),
    }
}

#[test]
fn every_solid_draws_shaded_faces_and_leaves_the_source_editable() {
    for id in SOLIDS {
        let mut n = square_node();
        let source = n.path_data().unwrap().clone();
        n.appearance.effects.push(fx(id, json!({})));
        let art = solid_art(&n).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(n.path_data(), Some(&source), "{id}");
        assert!(!faces(&art).is_empty(), "{id}");
        // The faces are the fill's red or the stroke's black, shaded: never brighter, never another
        // hue.
        for f in faces(&art) {
            let [r, g, b] = fill_rgb(f);
            assert!(r <= 0.8 + 1e-6 && (g * 4.0 - r).abs() < 1e-4 && (b * 8.0 - r).abs() < 1e-4 || id == ROTATE, "{id}: {r} {g} {b}");
        }
        // The art follows the source when it moves.
        let before = art.visual_bounds().unwrap();
        n.transform(Affine::translate((50.0, -30.0)), false);
        let after = solid_art(&n).unwrap().visual_bounds().unwrap();
        assert!((after.x0 - before.x0 - 50.0).abs() < 1e-6 && (after.y0 - before.y0 + 30.0).abs() < 1e-6, "{id}");
        // Hidden, the effect draws nothing of its own.
        n.appearance.effects[0].visible = false;
        assert!(solid_art(&n).is_none(), "{id}");
    }
}

#[test]
fn rotate_turns_every_fill_and_stroke_and_the_solids_use_the_top_fill() {
    let mut n = square_node();
    n.appearance.effects.push(fx(ROTATE, json!({})));
    let art = solid_art(&n).unwrap();
    // The fill's plane and the stroke's outline, each in its own paint.
    let surfaces = art.children().unwrap();
    assert_eq!(surfaces.len(), 2);
    let paint = |s: &Node| fill_rgb(&s.children().unwrap()[0]);
    assert!(paint(&surfaces[0])[0] > paint(&surfaces[0])[1], "the red fill below");
    assert!(paint(&surfaces[1]).iter().all(|c| *c < 0.05), "the black stroke above");
    // Turned 60° about y with no perspective, the stroke's outline spans half its width plus the
    // stroke's own outset, foreshortened as well.
    n.appearance.effects[0].params = json!({"rotationX": 0, "rotationY": 60, "rotationZ": 0});
    let b = solid_art(&n).unwrap().children().unwrap()[1].visual_bounds().unwrap();
    assert!((b.width() - 104.0 * 0.5 - 0.75).abs() < 1e-6, "{b:?}");
    // Extrude & Bevel draws the fill's solid and the stroke's, both opaque, as one surface.
    let mut n = square_node();
    n.appearance.effects.push(fx(EXTRUDE, json!({})));
    assert_eq!(solid_art(&n).unwrap().children().unwrap().len(), 1);
}

const BLUE: Color = Color::Rgb { r: 0.1, g: 0.3, b: 0.9 };

/// `b` with a red fill and a 6 pt blue stroke above it, on the path.
fn stroked(b: &BezPath) -> Node {
    Node::path(NodeId(1), PathData::from_bezpath(b), Appearance::basic(Paint::solid(RED), Paint::solid(BLUE), 6.0))
}

/// A `width` × 100 pt rectangle at (100, 100), stroked.
fn stroked_rect(width: f64) -> Node {
    let mut b = BezPath::new();
    b.move_to((100.0, 100.0));
    b.line_to((100.0 + width, 100.0));
    b.line_to((100.0 + width, 200.0));
    b.line_to((100.0, 200.0));
    b.close_path();
    stroked(&b)
}

/// Is `face` in the stroke's blue (rather than the fill's red)?
fn blue(face: &Node) -> bool {
    let [r, _, b] = fill_rgb(face);
    b > r
}

/// The bounds of the faces' own paths (without their seam strokes).
fn extent(faces: &[&Node]) -> Rect {
    faces.iter().filter_map(|f| f.path_data()?.bounds()).reduce(|a, b| a.union(b)).unwrap()
}

fn assert_rect(r: Rect, x0: f64, y0: f64, x1: f64, y1: f64) {
    assert!([r.x0 - x0, r.y0 - y0, r.x1 - x1, r.y1 - y1].iter().all(|d| d.abs() < 1e-6), "{r:?}");
}

/// Extrude & Bevel and Inflate build a solid from each stroke's outline as well as from the path
/// for the fill: seen straight on, the stroke's faces reach 3 pt (half its width) outside the
/// fill's, and they are drawn in the stroke's paint.
#[test]
fn a_stroke_builds_a_solid_from_its_outline_beside_the_fill() {
    for id in [EXTRUDE, INFLATE] {
        let mut n = stroked_rect(100.0);
        n.appearance.effects.push(fx(id, json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0})));
        let art = solid_art(&n).unwrap();
        let (strokes, fills): (Vec<&Node>, Vec<&Node>) = faces(&art).into_iter().partition(|f| blue(f));
        assert!(!strokes.is_empty(), "{id}: the stroke draws");
        assert_rect(extent(&fills), 100.0, 100.0, 200.0, 200.0);
        assert_rect(extent(&strokes), 97.0, 97.0, 203.0, 203.0);
        // Its own blue, shaded: never brighter, never another hue.
        for f in &strokes {
            let [r, g, b] = fill_rgb(f);
            assert!(b <= 0.9 + 1e-6 && (r * 9.0 - b).abs() < 1e-4 && (g * 3.0 - b).abs() < 1e-4, "{id}: {r} {g} {b}");
        }
    }
}

/// Turned, the extrusion's walls show in both paints, and the front caps, which nothing of either
/// solid can hide, are drawn last: the fill's, then the stroke's above it, equally lit. Without
/// depth the two flat shapes are drawn in the same order. A stroke below the fill has its cap
/// drawn below the fill's.
#[test]
fn the_front_caps_are_drawn_last_in_the_order_of_the_appearance() {
    // A triangle wider than deep, so the near ends of the walls lie nearer than the middle of the
    // caps; the mean of its outline's points is not the mean of the stroke's.
    let mut b = BezPath::new();
    b.move_to((100.0, 100.0));
    b.line_to((400.0, 100.0));
    b.line_to((100.0, 200.0));
    b.close_path();
    for params in [json!({}), json!({"depth": 120, "bevel": "rounded"}), json!({"depth": 0})] {
        let mut n = stroked(&b);
        n.appearance.effects.push(fx(EXTRUDE, params.clone()));
        n.appearance.items.reverse();
        let art = solid_art(&n).unwrap();
        let [.., stroke_cap, fill_cap] = faces(&art)[..] else { panic!("{params}: two caps") };
        assert!(blue(stroke_cap) && !blue(fill_cap), "{params}: the stroke below");
        n.appearance.items.reverse();
        let art = solid_art(&n).unwrap();
        let faces = faces(&art);
        let flat = params["depth"] == json!(0);
        assert!(if flat { faces.len() == 2 } else { faces.len() > 4 }, "{params}: {} faces", faces.len());
        let [.., fill_cap, stroke_cap] = faces.as_slice() else { panic!("{params}: two caps") };
        assert!(!blue(fill_cap) && blue(stroke_cap), "{params}");
        // The stroke's cap is a ring: its outline and the hole the fill shows through.
        assert_eq!(stroke_cap.path_data().unwrap().subpaths.len(), 2, "{params}");
        assert_eq!(fill_cap.path_data().unwrap().subpaths.len(), 1, "{params}");
        let shade = |f: &Node, i: usize, of: f32| fill_rgb(f)[i] / of;
        assert!((shade(fill_cap, 0, 0.8) - shade(stroke_cap, 2, 0.9)).abs() < 1e-5, "{params}: one plane, one light");
        if !flat {
            assert!(faces.iter().filter(|f| blue(f)).count() > 2 && faces.iter().filter(|f| !blue(f)).count() > 1, "{params}: walls of both");
        }
    }
}

/// A stroked open path builds a ribbon, the stroke's outline, not the area the path would enclose
/// if it were closed: for an L of two 100 pt legs, a 6 pt band with butt ends and a mitered corner.
#[test]
fn a_stroked_open_path_builds_a_ribbon() {
    let mut b = BezPath::new();
    b.move_to((100.0, 100.0));
    b.line_to((200.0, 100.0));
    b.line_to((200.0, 200.0));
    for id in [EXTRUDE, INFLATE] {
        let mut n = Node::path(NodeId(1), PathData::from_bezpath(&b), Appearance::basic(Paint::None, Paint::solid(BLUE), 6.0));
        n.appearance.effects.push(fx(id, json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0, "volume": 0})));
        let art = solid_art(&n).unwrap();
        let faces = faces(&art);
        assert!(faces.iter().all(|f| blue(f)), "{id}");
        assert_rect(extent(&faces), 100.0, 97.0, 203.0, 200.0);
        let area: f64 = faces.iter().map(|f| f.path_data().unwrap().to_bezpath().area().abs()).sum();
        // 6 × 200 less the 3 × 3 the legs share, plus the 3 × 3 miter.
        assert!((area - 1200.0).abs() < 1e-6, "{id}: {area}");
    }
}

/// A stroke whose outline takes more points than a solid takes is left out, and the fill's solid
/// still draws.
#[test]
fn a_stroke_past_the_limits_is_left_out() {
    // 1,500 corners: within the 2,048 points of the fill's solid, but not the stroke's outline.
    let points: Vec<Point> = (0..1500)
        .map(|k| {
            let (s, c) = (k as f64 * std::f64::consts::TAU / 1500.0).sin_cos();
            let r = if k % 2 == 0 { 100.0 } else { 96.0 };
            Point::new(200.0 + r * c, 200.0 + r * s)
        })
        .collect();
    let path = PathData::single(vectorcraft_geom::SubPath::polyline(&points, true));
    for id in [EXTRUDE, INFLATE] {
        let mut n = Node::path(NodeId(1), path.clone(), Appearance::basic(Paint::solid(RED), Paint::solid(BLUE), 2.0));
        n.appearance.effects.push(fx(id, json!({})));
        let art = solid_art(&n).unwrap_or_else(|| panic!("{id}: the fill's solid draws"));
        assert!(!faces(&art).is_empty() && faces(&art).iter().all(|f| !blue(f)), "{id}");
    }
}

/// At no volume Inflate's front is flat: turned, the fill's cells are all drawn before the stroke's,
/// whatever their depths, as the extrusion's caps are. When the fill puffs, a stroke too narrow to
/// puff on the grid stays flat and keeps its depths: the fill's puff is drawn over its far side.
#[test]
fn a_flat_inflate_draws_the_strokes_front_above_the_fills() {
    // On the 100 pt square's grid (cells 101 / 48 pt apart from 99.5) no grid point falls inside a
    // 1 pt stroke, so the stroke stays flat.
    for (volume, width, length) in [(0.0, 6.0, 300.0), (50.0, 1.0, 100.0)] {
        let mut n = stroked_rect(length);
        if let Some(AppearanceItem::Stroke(s)) = n.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Stroke(_))) {
            s.width = width;
        }
        n.appearance.effects.push(fx(INFLATE, json!({ "volume": volume })));
        let art = solid_art(&n).unwrap();
        let faces = faces(&art);
        let first_blue = faces.iter().position(|f| blue(f)).unwrap();
        let last_red = faces.iter().rposition(|f| !blue(f)).unwrap();
        assert_eq!(last_red < first_blue, volume == 0.0, "{volume}: red at {last_red}, blue from {first_blue} of {}", faces.len());
    }
}

/// With Perspective one eye sees the fill's puff and the stroke's, however much higher one rises:
/// seen from behind, the back of a stroke aligned outside meets the fill's back exactly.
#[test]
fn inflate_sees_the_fill_and_the_stroke_from_one_eye() {
    let mut n = stroked_rect(100.0);
    if let Some(AppearanceItem::Stroke(s)) = n.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Stroke(_))) {
        s.align = vectorcraft_doc::StrokeAlign::Outside;
    }
    let params = json!({"volume": 100, "depth": 100, "perspectiveAngle": 120, "rotationX": 15, "rotationY": 160, "rotationZ": 0});
    n.appearance.effects.push(fx(INFLATE, params));
    let art = solid_art(&n).unwrap();
    let [.., fill_back, stroke_back] = faces(&art)[..] else { panic!("two backs") };
    assert!(!blue(fill_back) && blue(stroke_back));
    let fill = fill_back.path_data().unwrap().bounds().unwrap();
    // The ring's inner contour: the smaller of its two.
    let rims = stroke_back.path_data().unwrap().subpaths.iter().filter_map(|s| PathData::new(vec![s.clone()]).bounds());
    let hole = rims.reduce(|a, b| a.intersect(b)).unwrap();
    assert_rect(hole, fill.x0, fill.y0, fill.x1, fill.y1);
}

/// A stroke with its own transparency or blending is a surface of its own above the fill's, so its
/// opacity applies to the stroke as a whole.
#[test]
fn a_translucent_stroke_is_a_surface_of_its_own() {
    for (opacity, blend) in [(0.5, BlendMode::Normal), (1.0, BlendMode::Multiply)] {
        let mut n = stroked_rect(100.0);
        if let Some(AppearanceItem::Stroke(s)) = n.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Stroke(_))) {
            s.opacity = opacity;
            s.blend = blend;
        }
        n.appearance.effects.push(fx(EXTRUDE, json!({})));
        let art = solid_art(&n).unwrap();
        let surfaces = art.children().unwrap();
        assert_eq!(surfaces.len(), 2);
        assert_eq!((surfaces[0].opacity, surfaces[0].blend), (1.0, BlendMode::Normal));
        assert_eq!((surfaces[1].opacity, surfaces[1].blend), (opacity, blend));
        assert!(surfaces[0].children().unwrap().iter().all(|f| !blue(f)) && surfaces[1].children().unwrap().iter().all(|f| blue(f)));
    }
}

/// With a light that is not white, a fill in a gradient with transparent stops is drawn with an
/// opacity mask over its whole surface: the fill's solid and the stroke's stay surfaces of their
/// own, and the mask stays on the fill's. Under a white light the same object is one surface.
#[test]
fn a_masked_surface_under_a_colored_light_stays_whole() {
    use vectorcraft_doc::color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let mut half = GradientStop::new(1.0, RED);
    half.opacity = 0.5;
    let gradient = GradientPaint::new(Gradient { kind: GradientKind::Linear, stops: vec![GradientStop::new(0.0, RED), half] });
    for (lights, surfaces) in [(json!({"lights": [{"color": "#ff8080"}]}), 2), (json!({}), 1)] {
        let mut n = stroked_rect(100.0);
        if let Some(AppearanceItem::Fill(f)) = n.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Fill(_))) {
            f.paint = Paint::Gradient(Box::new(gradient.clone()));
        }
        n.appearance.effects.push(fx(EXTRUDE, lights.clone()));
        let art = solid_art(&n).unwrap();
        let drawn = art.children().unwrap();
        assert_eq!(drawn.len(), surfaces, "{lights}");
        assert_eq!(drawn[0].extras().mask.is_some(), surfaces == 2, "{lights}");
    }
}

#[test]
fn transparency_stays_on_the_whole_surface_and_items_take_their_own_effect() {
    let mut n = square_node();
    n.opacity = 0.5;
    if let AppearanceItem::Fill(f) = &mut n.appearance.items[0] {
        f.opacity = 0.4;
        f.effects.push(fx(INFLATE, json!({})));
    }
    let art = solid_art(&n).unwrap();
    assert_eq!(art.opacity, 0.5);
    let surfaces = art.children().unwrap();
    // The fill inflated, the stroke drawn flat as it was.
    assert_eq!(surfaces.len(), 2);
    assert_eq!(surfaces[0].opacity, 0.4);
    assert!(surfaces[0].children().unwrap().iter().all(|c| c.opacity == 1.0));
    assert!(matches!(surfaces[1].kind, NodeKind::Path { .. }));
}

#[test]
fn export_and_expand_bake_faces_with_their_own_ids() {
    for id in SOLIDS {
        let mut d = Document::new(400.0, 400.0);
        let mut n = square_node();
        n.id = d.alloc_id();
        n.appearance.effects.push(fx(id, json!({"bevel": "rounded"})));
        assert_eq!(crate::bake_appearance(&n), solid_art(&n), "{id}");
        let mut stroke = |_: &mut Document, _: &PathData, _: FillRule, _: &StrokeLayer| None;
        let art = crate::expand_leaf(&mut d, &n, &mut stroke).unwrap();
        let mut ids = std::collections::HashSet::new();
        fn visit(n: &Node, ids: &mut std::collections::HashSet<NodeId>) {
            assert!(ids.insert(n.id), "{:?} twice", n.id);
            assert!(!has_solid(n));
            for c in n.children().into_iter().flatten() {
                visit(c, ids);
            }
        }
        visit(&art, &mut ids);
        assert!(ids.len() > 2, "{id}");
    }
}

/// A twisted, tapered, hollow Inflate (both sides) exports and expands like the others: faces with
/// their own ids, no 3D effect left on them.
#[test]
fn a_twisted_tapered_hollow_inflate_exports_and_expands() {
    let mut d = Document::new(400.0, 400.0);
    let mut n = square_node();
    n.id = d.alloc_id();
    let params = json!({"twist": 120, "taper": 50, "cap": false, "bothSides": true, "rotationX": -20, "rotationY": 35});
    n.appearance.effects.push(fx(INFLATE, params));
    let art = solid_art(&n).unwrap();
    assert_eq!(crate::bake_appearance(&n), Some(art.clone()));
    assert!(faces(&art).len() > 20, "{} faces", faces(&art).len());
    let mut stroke = |_: &mut Document, _: &PathData, _: FillRule, _: &StrokeLayer| None;
    let expanded = crate::expand_leaf(&mut d, &n, &mut stroke).unwrap();
    let mut ids = std::collections::HashSet::new();
    fn visit(n: &Node, ids: &mut std::collections::HashSet<NodeId>) {
        assert!(ids.insert(n.id), "{:?} twice", n.id);
        assert!(!has_solid(n));
        for c in n.children().into_iter().flatten() {
            visit(c, ids);
        }
    }
    visit(&expanded, &mut ids);
    assert!(ids.len() > 20, "{} nodes", ids.len());
}

/// Files and scripts from before the Perspective angle give `perspective` as a 0–100 % strength:
/// each 3D effect draws it exactly as the angle that stands for it, an angle given too wins, and
/// the current parameters carry only the angle.
#[test]
fn an_earlier_perspective_strength_draws_as_its_angle() {
    let art = |n: &Node, id: &str| if id == crate::REVOLVE { crate::revolve_art(n) } else { solid_art(n) };
    for id in SOLIDS.iter().copied().chain([crate::REVOLVE]) {
        let view = |extra: Value| {
            let mut p = json!({"rotationX": -25, "rotationY": 35, "rotationZ": 0, "depth": 80});
            p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            p
        };
        let with = |params: Value| {
            let mut n = square_node();
            // As read from a file: the stored parameters, not a fresh effect's.
            n.appearance.effects.push(Effect { id: id.into(), params, visible: true });
            art(&n, id).unwrap().visual_bounds().unwrap()
        };
        let angle = vectorcraft_three_d::perspective_from_strength(0.6);
        let old = with(view(json!({"perspective": 60})));
        let new = with(view(json!({"perspectiveAngle": angle})));
        let none = with(view(json!({})));
        for (a, b) in [(old.x0, new.x0), (old.y0, new.y0), (old.x1, new.x1), (old.y1, new.y1)] {
            assert!((a - b).abs() < 1e-9, "{id}: {old:?} against {new:?}, none {none:?}");
        }
        assert!((old.width() - none.width()).abs() > 1.0 || (old.height() - none.height()).abs() > 1.0, "{id}: perspective shows");
        assert_eq!(with(view(json!({"perspective": 60, "perspectiveAngle": 0}))), none, "{id}: the angle wins");
        let merged = crate::merged_params(id, &view(json!({"perspective": 60})));
        assert!(merged.get("perspective").is_none() && (merged["perspectiveAngle"].as_f64().unwrap() - angle).abs() < 1e-12, "{id}");
    }
}

/// Effects from before Rotation and Height set the light by azimuth and elevation: each 3D effect
/// shades its faces exactly as with the Rotation, Height and side that stand for it.
#[test]
fn an_earlier_light_direction_shades_as_its_rotation_and_height() {
    fn colours(n: &Node, out: &mut Vec<[f32; 3]>) {
        if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first() {
            if let Paint::Gradient(g) = &f.paint {
                out.extend(g.gradient.stops.iter().map(|s| s.color.to_rgb()));
            } else if let Some(c) = f.paint.color() {
                out.push(c.to_rgb());
            }
        }
        for c in n.children().into_iter().flatten() {
            colours(c, out);
        }
    }
    for id in SOLIDS.iter().copied().chain([crate::REVOLVE]) {
        for (azimuth, elevation) in [(-45.0, 45.0), (150.0, -30.0), (60.0, 80.0), (-120.0, 10.0)] {
            let with = |light: Value| {
                let mut p = json!({"rotationX": -25, "rotationY": 35, "rotationZ": 0, "depth": 60, "lightSoftness": 0});
                p.as_object_mut().unwrap().extend(light.as_object().unwrap().clone());
                let mut n = square_node();
                n.appearance.effects.push(Effect { id: id.into(), params: p, visible: true });
                let art = if id == crate::REVOLVE { crate::revolve_art(&n) } else { solid_art(&n) }.unwrap();
                let mut out = vec![];
                colours(&art, &mut out);
                out
            };
            let (r, h, behind) = vectorcraft_three_d::light_from_azimuth_elevation(azimuth, elevation);
            let old = with(json!({"lightAzimuth": azimuth, "lightElevation": elevation}));
            let new = with(json!({"lightRotation": r, "lightHeight": h, "lightBehind": behind}));
            assert!(!old.is_empty() && old.len() == new.len(), "{id}");
            for (a, b) in old.iter().zip(&new) {
                assert!(a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-6), "{id} {azimuth} {elevation}: {a:?} against {b:?}");
            }
        }
        let merged = crate::merged_params(id, &json!({"lightAzimuth": 90, "lightElevation": 0}));
        assert!(
            merged.get("lightAzimuth").is_none() && merged["lightHeight"].as_f64().unwrap().abs() < 1e-9 && merged["lightBehind"] == json!(false),
            "{id}: {merged}"
        );
    }
}

#[test]
fn effects_before_it_shape_the_solid_and_effects_after_it_bend_the_result() {
    let mut n = square_node();
    n.appearance.effects.push(fx("path.offsetPath", json!({"offset": 20})));
    n.appearance.effects.push(fx(EXTRUDE, json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0, "perspectiveAngle": 0})));
    // Straight on, the extrusion shows its front: the offset square and its 4 pt stroke.
    let b = solid_art(&n).unwrap().visual_bounds().unwrap();
    assert!((b.width() - 144.0 - 0.75).abs() < 1e-6, "{b:?}");
    n.appearance.effects.push(fx("distort.twist", json!({"angle": 30})));
    let art = solid_art(&n).unwrap();
    assert!(art.appearance.effects.iter().any(|e| e.id == "distort.twist"), "applied to the result as a whole");
    assert!(art.appearance.effects.iter().all(|e| e.id != "path.offsetPath" && !is_three_d(&e.id)));
}

#[test]
fn parameters_are_validated_and_lengths_scale_with_the_object() {
    let n = square_node();
    assert!(validate_solid(&n, EXTRUDE, &json!({"bevel": "rounded", "depth": 80})).is_ok());
    assert!(validate_solid(&n, EXTRUDE, &json!({"bevel": "fancy"})).unwrap_err().contains("bevel must be"));
    let text = Node::group(NodeId(2), vec![]);
    assert!(validate_solid(&text, INFLATE, &json!({})).unwrap_err().contains("requires a path"));
    for (id, key) in [(EXTRUDE, "depth"), (EXTRUDE, "bevelWidth"), (EXTRUDE, "bevelHeight"), (INFLATE, "depth")] {
        assert!(crate::is_length(id, key, false), "{id} {key}");
    }
    assert!(!crate::is_length(EXTRUDE, "twist", false));
    for id in SOLIDS {
        assert!(!crate::is_geometry(id) && is_three_d(id));
        assert!(crate::default_params(id).is_some());
    }
    // Options read from parameters, with the documented ranges' defaults.
    let o = extrude_options(&json!({"taper": 40, "bevel": "step", "bevelSpace": 30, "bevelRepeat": 99}));
    assert_eq!(o.taper, 0.4);
    let b = o.bevel.unwrap();
    assert_eq!((b.shape, b.space, b.repeat), (BevelShape::Step, 0.3, 32));
    assert!(extrude_options(&json!({})).bevel.is_none());
}

#[test]
fn inflate_reads_twist_taper_and_cap() {
    let o = inflate_options(&json!({"twist": 90, "taper": 40, "cap": false}));
    assert_eq!((o.twist, o.taper, o.cap), (90.0, 0.4, false));
    let d = inflate_options(&json!({}));
    assert_eq!((d.twist, d.taper, d.cap), (0.0, 1.0, true));
    assert_eq!(crate::default_params(INFLATE).unwrap()["cap"], json!(true));
}

#[test]
fn legacy_white_inflate_keeps_polygon_faces_and_its_seam_range() {
    let mut n = square_node();
    n.appearance.effects.push(fx(INFLATE, json!({})));
    let art = solid_art(&n).unwrap();
    let original = faces(&art);
    assert!(original.len() > 1000);
    assert!(original.iter().any(|f| f.path_data().unwrap().subpaths.iter().any(|p| p.anchors.len() > 3)), "legacy cells remain polygons");
    for face in original {
        assert!(matches!(face.kind, NodeKind::Path { .. }));
        let AppearanceItem::Stroke(stroke) = &face.appearance.items[1] else { panic!("seam") };
        assert!((0.05..=0.75).contains(&stroke.width));
        assert_eq!(stroke.join, vectorcraft_doc::LineJoin::Miter);
    }
    // A tiny shape exercises the lower bound, which otherwise disappears at grazing angles.
    n.transform(Affine::scale(0.001), false);
    let art = solid_art(&n).unwrap();
    assert!(faces(&art).iter().any(|face| matches!(&face.appearance.items[1], AppearanceItem::Stroke(s) if s.width == 0.05)));
    // An explicit white light overrides a stale legacy color without switching shading paths.
    n.appearance.effects[0].params = json!({"lightColor":"#ff0000","lights":[{}]});
    let art = solid_art(&n).unwrap();
    assert!(faces(&art).iter().all(|face| matches!(face.kind, NodeKind::Path { .. })));
}

#[test]
fn colored_solid_inflate_batches_channels_and_clips_once_per_surface() {
    let mut n = square_node();
    n.appearance.effects.push(fx(INFLATE, json!({"lights":[{"color":"#ff0000"},{"color":"#0000ff"}]})));
    let art = solid_art(&n).unwrap();
    fn count(n: &Node) -> (usize, usize, usize) {
        let (mut groups, mut clips, mut paths) = match &n.kind {
            NodeKind::Group { clip, .. } => (1, usize::from(*clip), 0),
            NodeKind::Path { .. } => (0, 0, 1),
            _ => (0, 0, 0),
        };
        for child in n.children().into_iter().flatten() {
            let (g, c, p) = count(child);
            groups += g;
            clips += c;
            paths += p;
        }
        (groups, clips, paths)
    }
    let (groups, clips, paths) = count(&art);
    assert!(paths > 1000, "exercise a tessellated surface");
    // The fill's solid and the stroke's, each one composite with its own clip.
    let surfaces = art.children().unwrap().len();
    assert_eq!(surfaces, 2);
    assert_eq!(clips, surfaces);
    assert_eq!(groups, 1 + 6 * surfaces, "group count must not grow with triangle count");
}
