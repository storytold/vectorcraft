use serde_json::json;
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_geom::{Affine, BezPath};

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
        // The faces are the fill's red, shaded: never brighter, never another hue.
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
    // Extrude & Bevel paints one surface with the top fill.
    let mut n = square_node();
    n.appearance.effects.push(fx(EXTRUDE, json!({})));
    assert_eq!(solid_art(&n).unwrap().children().unwrap().len(), 1);
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
    // Straight on, the extrusion shows its front: the offset square.
    let b = solid_art(&n).unwrap().visual_bounds().unwrap();
    assert!((b.width() - 140.0 - 0.75).abs() < 1e-6, "{b:?}");
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
    assert_eq!(clips, 1);
    assert_eq!(groups, 7, "group count must not grow with triangle count");
}
