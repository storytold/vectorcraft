use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_render::effects;

use crate::{NodeId, Session};

fn session_with_rect() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    (s, NodeId(r["id"].as_u64().unwrap()))
}

fn node(s: &Session, id: NodeId) -> vectorcraft_doc::Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

#[test]
fn revolve_commands_keep_profile_live_edit_options_expand_and_undo() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":300,"height":300})).unwrap();
    let r = s.execute("path.create", &json!({"d":"M 150 40 L 180 60 L 180 140 L 150 160"})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    let source = node(&s, id).path_data().unwrap().clone();
    s.execute("effect.apply", &json!({"effect":"threeD.revolve","ids":[id.0],"item":null})).unwrap();
    let before = effects_for_test(&s, id);
    assert_eq!(node(&s, id).path_data(), Some(&source));
    let doc = &s.doc().unwrap().doc;
    let restored = vectorcraft_format::load(&vectorcraft_format::save(doc, false)).unwrap();
    assert_eq!(restored.node(id).unwrap().path_data(), Some(&source));
    assert_eq!(effects::revolve_art(restored.node(id).unwrap()).unwrap(), before);
    let pdf = s.execute("document.serialize", &json!({"format":"pdf"})).unwrap();
    let bytes = vectorcraft_format::base64_decode(pdf["dataBase64"].as_str().unwrap()).unwrap();
    assert!(bytes.starts_with(b"%PDF"));
    assert!(s.execute("effect.apply", &json!({"effect":"threeD.revolve","ids":[id.0],"item":null})).is_err());
    assert!(s.execute("effect.duplicate", &json!({"index":0,"ids":[id.0],"item":null})).is_err());
    s.execute("effect.setParams", &json!({"index":0,"params":{"angle":180,"offset":10},"ids":[id.0],"item":null})).unwrap();
    assert_ne!(effects_for_test(&s, id), before);
    assert!(s.execute("effect.setParams", &json!({"index":0,"params":{"edge":"invalid"},"ids":[id.0],"item":null})).is_err());
    s.execute("effect.expandAppearance", &json!({"ids":[id.0]})).unwrap();
    assert!(matches!(node(&s, id).kind, NodeKind::Group { .. }));
    assert!(!effects::has_revolve(&node(&s, id)));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, id).path_data(), Some(&source));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(effects_for_test(&s, id), before);
}

/// Extrude & Bevel, Inflate and Rotate: applied by command, saved and reopened, edited,
/// exported, expanded and undone; one 3D effect per object.
#[test]
fn solid_effects_apply_save_edit_export_expand_and_undo() {
    for effect in effects::SOLIDS {
        let (mut s, id) = session_with_rect();
        let source = node(&s, id).path_data().unwrap().clone();
        s.execute("effect.apply", &json!({"effect": effect, "ids": [id.0], "item": null})).unwrap();
        let art = effects::solid_art(&node(&s, id)).unwrap();
        assert_eq!(node(&s, id).path_data(), Some(&source), "{effect}: the source stays editable");
        let doc = &s.doc().unwrap().doc;
        let restored = vectorcraft_format::load(&vectorcraft_format::save(doc, false)).unwrap();
        assert_eq!(effects::solid_art(restored.node(id).unwrap()).unwrap(), art, "{effect}: reopens the same");
        // One 3D effect per object, whichever kind.
        for other in ["threeD.revolve", "threeD.extrude", "threeD.rotate"] {
            assert!(s.execute("effect.apply", &json!({"effect": other, "ids": [id.0], "item": null})).is_err(), "{effect} + {other}");
        }
        s.execute("effect.setParams", &json!({"index": 0, "params": {"rotationY": 50, "rotationX": 10}, "ids": [id.0], "item": null})).unwrap();
        assert_ne!(effects::solid_art(&node(&s, id)).unwrap(), art, "{effect}: options change the art");
        assert!(s.execute("effect.setParams", &json!({"index": 0, "params": {"bevel": "nope"}, "ids": [id.0], "item": null})).is_err());
        let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap();
        let svg = svg["data"].as_str().or_else(|| svg["text"].as_str()).map(str::to_string).unwrap_or_else(|| svg.to_string());
        assert!(vectorcraft_testkit::svg_shapes(&svg) >= 2, "{effect}: exported as vector faces");
        s.execute("effect.expandAppearance", &json!({"ids": [id.0]})).unwrap();
        assert!(matches!(node(&s, id).kind, NodeKind::Group { .. }), "{effect}");
        assert!(!effects::has_solid(&node(&s, id)));
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(node(&s, id).appearance.effects.len(), 0, "{effect}: back to the plain rectangle");
    }
    // Type has no outline to extrude.
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let t = s.execute("text.create", &json!({"x": 50, "y": 50, "text": "3D"})).unwrap();
    let err = s.execute("effect.apply", &json!({"effect": "threeD.extrude", "ids": [t["id"]], "item": null})).unwrap_err();
    assert!(err.to_string().contains("requires a path"), "{err}");
}

/// Scripts from before the Perspective angle give `perspective` as a 0–100 % strength: it is
/// stored as the angle that shows the same view, and a later angle replaces an earlier strength.
#[test]
fn an_earlier_perspective_strength_is_set_as_its_angle() {
    let (mut s, id) = session_with_rect();
    s.execute("effect.apply", &json!({"effect": effects::EXTRUDE, "ids": [id.0], "item": null, "params": {"perspective": 60}})).unwrap();
    let params = |s: &Session| node(s, id).appearance.effects[0].params.clone();
    let angle_of = |pct: f64| effects::merged_params(effects::EXTRUDE, &json!({"perspective": pct}))["perspectiveAngle"].as_f64().unwrap();
    let angle = angle_of(60.0);
    assert!((angle - 30.12).abs() < 0.01, "{angle}");
    assert!(params(&s).get("perspective").is_none() && (params(&s)["perspectiveAngle"].as_f64().unwrap() - angle).abs() < 1e-12, "{}", params(&s));
    s.execute("effect.setParams", &json!({"index": 0, "params": {"perspective": 100}, "ids": [id.0], "item": null})).unwrap();
    assert!((params(&s)["perspectiveAngle"].as_f64().unwrap() - angle_of(100.0)).abs() < 1e-12);
    // A file of then: the strength stored; setting an angle replaces it.
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().appearance.effects[0].params =
        json!({"depth": 50, "perspective": 60});
    s.execute("effect.setParams", &json!({"index": 0, "params": {"perspectiveAngle": 120}, "ids": [id.0], "item": null})).unwrap();
    assert_eq!(params(&s), json!({"depth": 50, "perspectiveAngle": 120}));
}

/// Scripts from before Rotation and Height give the light by `lightAzimuth` and
/// `lightElevation`: stored as the same light; a later Height keeps that light's Rotation.
#[test]
fn an_earlier_light_direction_is_set_as_rotation_and_height() {
    let (mut s, id) = session_with_rect();
    s.execute("effect.apply", &json!({"effect": effects::ROTATE, "ids": [id.0], "item": null, "params": {"lightAzimuth": 180, "lightElevation": 0}}))
        .unwrap();
    let params = |s: &Session| node(s, id).appearance.effects[0].params.clone();
    // Straight from behind the object, level with it.
    assert!(params(&s).get("lightAzimuth").is_none() && params(&s).get("lightElevation").is_none(), "{}", params(&s));
    assert_eq!((params(&s)["lightHeight"].as_f64().unwrap().round(), &params(&s)["lightBehind"]), (90.0, &json!(true)));
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().appearance.effects[0].params =
        json!({"lightAzimuth": 90, "lightElevation": 0});
    s.execute("effect.setParams", &json!({"index": 0, "params": {"lightHeight": 60}, "ids": [id.0], "item": null})).unwrap();
    assert_eq!(params(&s), json!({"lightRotation": 0.0, "lightHeight": 60, "lightBehind": false}));
}

/// Scripts from before Scribble's separate variations give one `variation`, which is stored as the
/// Path Overlap and Spacing variations. An angle is stored above -180° and up to 180°.
#[test]
fn scribble_stores_an_earlier_variation_as_two_and_its_angle_within_180() {
    let (mut s, id) = session_with_rect();
    let p = json!({"effect": "stylize.scribble", "ids": [id.0], "item": null, "params": {"angle": 270, "variation": 2}});
    s.execute("effect.apply", &p).unwrap();
    let params = |s: &Session| node(s, id).appearance.effects[0].params.clone();
    assert_eq!((params(&s)["angle"].as_f64(), params(&s).get("variation")), (Some(-90.0), None), "{}", params(&s));
    assert_eq!((&params(&s)["overlapVariation"], &params(&s)["spacingVariation"]), (&json!(2), &json!(2)));
    s.execute("effect.setParams", &json!({"index": 0, "params": {"angle": -200}, "ids": [id.0], "item": null})).unwrap();
    assert_eq!(params(&s)["angle"].as_f64(), Some(160.0));
    // A file of then stores its variation; setting one of the two keeps it for the other.
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().appearance.effects[0].params = json!({"variation": 3});
    s.execute("effect.setParams", &json!({"index": 0, "params": {"spacingVariation": 1}, "ids": [id.0], "item": null})).unwrap();
    assert_eq!(params(&s), json!({"overlapVariation": 3, "spacingVariation": 1}));
}

fn effects_for_test(s: &Session, id: NodeId) -> vectorcraft_doc::Node {
    effects::revolve_art(&node(s, id)).unwrap()
}

#[test]
fn smooth_revolve_commands_validate_save_expand_and_undo() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":300,"height":300})).unwrap();
    let r = s.execute("path.create", &json!({"d":"M150 40 C190 60 190 140 150 160"})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("effect.apply", &json!({"effect":"threeD.revolve","ids":[id.0],"item":null,"params":{"offset":20,"segments":16}})).unwrap();
    let initial = node(&s, id);
    let source = initial.path_data().unwrap().clone();
    assert!(!initial.appearance.effects[0].params.as_object().unwrap().contains_key("smoothShading"));
    assert!(initial.appearance.effects[0].params["cap"].as_bool().unwrap());
    let mut gradients = 0;
    effects_for_test(&s, id).walk(&mut |face| {
        gradients += face.appearance.items.iter().filter(|item| {
            matches!(item, vectorcraft_doc::AppearanceItem::Fill(fill) if matches!(fill.paint, vectorcraft_doc::color::Paint::Gradient(_)))
        }).count();
    });
    assert!(gradients > 0, "commands use smooth vector shading by default");
    s.execute("effect.setParams", &json!({"index":0,"ids":[id.0],"item":null,"params":{"lightAzimuth":30}})).unwrap();
    let smooth = node(&s, id);
    assert_eq!(smooth.path_data(), Some(&source));
    assert_ne!(effects_for_test(&s, id), effects::revolve_art(&initial).unwrap());
    let restored = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(effects::revolve_art(restored.node(id).unwrap()), Some(effects_for_test(&s, id)));
    s.execute("effect.expandAppearance", &json!({"ids":[id.0]})).unwrap();
    assert!(!effects::has_revolve(&node(&s, id)));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, id), smooth);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, id), initial);
}

#[test]
fn revolve_caps_save_export_expand_and_undo_without_changing_the_source() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":300,"height":300})).unwrap();
    let r = s.execute("path.create", &json!({"d":"M150 40 L150 160"})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    let source = node(&s, id).path_data().unwrap().clone();
    s.execute(
        "effect.apply",
        &json!({"effect":"threeD.revolve","ids":[id.0],"item":null,
        "params":{"offset":30,"rotationX":-35,"segments":16,"expandVisibleOnly":false,"cap":false}}),
    )
    .unwrap();
    let uncapped = node(&s, id);
    assert_eq!(uncapped.appearance.effects[0].params["cap"], json!(false));
    let count = |n: &vectorcraft_doc::Node| n.children().unwrap()[0].children().unwrap().len();
    let open_count = count(&effects_for_test(&s, id));
    s.execute("effect.setParams", &json!({"index":0,"ids":[id.0],"item":null,"params":{"cap":true}})).unwrap();
    let capped = node(&s, id);
    let art = effects_for_test(&s, id);
    assert_eq!(count(&art), open_count + 2, "each endpoint adds one continuous vector cap");
    assert_eq!(capped.path_data(), Some(&source));
    assert!(s.execute("effect.setParams", &json!({"index":0,"ids":[id.0],"item":null,"params":{"cap":"yes"}})).is_err());
    assert_eq!(node(&s, id), capped, "a bad boolean does not mutate the document");
    let restored = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(restored.node(id).unwrap().appearance.effects[0].params["cap"], json!(true));
    assert_eq!(effects::revolve_art(restored.node(id).unwrap()), Some(art.clone()));
    let pdf = s.execute("document.serialize", &json!({"format":"pdf"})).unwrap();
    assert!(vectorcraft_format::base64_decode(pdf["dataBase64"].as_str().unwrap()).unwrap().starts_with(b"%PDF"));
    s.execute("effect.expandAppearance", &json!({"ids":[id.0]})).unwrap();
    assert_eq!(count(&node(&s, id)), count(&art));
    assert!(!effects::has_revolve(&node(&s, id)));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, id), capped);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, id), uncapped);
}

#[test]
fn revolve_visibility_parameter_saves_expands_and_undoes_without_other_appearance_changes() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":300,"height":300})).unwrap();
    let r = s.execute("path.create", &json!({"d":"M 150 40 L 180 60 L 180 140 L 150 160"})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("effect.apply", &json!({"effect":"threeD.revolve","ids":[id.0],"item":null})).unwrap();
    let live = node(&s, id);
    let restored = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(restored.node(id).unwrap().appearance.effects[0].params["expandVisibleOnly"], json!(true));
    let count = |n: &vectorcraft_doc::Node| n.children().unwrap()[0].children().unwrap().len();
    s.execute("effect.expandAppearance", &json!({"ids":[id.0]})).unwrap();
    let visible = count(&node(&s, id));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, id), live);
    s.execute("effect.setParams", &json!({"index":0,"item":null,"ids":[id.0],"params":{"expandVisibleOnly":false}})).unwrap();
    s.execute("effect.expandAppearance", &json!({"ids":[id.0]})).unwrap();
    assert!(count(&node(&s, id)) > visible);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, id).appearance.effects[0].params["expandVisibleOnly"], json!(false));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, id), live);
    // The shared command still expands ordinary multiple-fill artwork as before.
    let r = s.execute("shape.rectangle", &json!({"x":10,"y":10,"width":20,"height":20})).unwrap();
    let ordinary = NodeId(r["id"].as_u64().unwrap());
    s.execute("effect.apply", &json!({"effect":"distort.twist","ids":[ordinary.0],"params":{"angle":15}})).unwrap();
    s.execute("effect.expandAppearance", &json!({"ids":[ordinary.0]})).unwrap();
    assert!(node(&s, ordinary).appearance.effects.is_empty());
}

#[test]
fn apply_uses_catalog_defaults_and_undoes() {
    let (mut s, id) = session_with_rect();
    let r = s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "params": {"x": 3}})).unwrap();
    assert_eq!(r["index"], json!(0));
    let n = node(&s, id);
    assert_eq!(n.appearance.effects.len(), 1);
    assert_eq!(n.appearance.effects[0].params["x"], json!(3));
    assert_eq!(n.appearance.effects[0].params["opacity"], json!(75.0));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(node(&s, id).appearance.effects.is_empty());
    assert!(s.execute("effect.apply", &json!({"effect": "nope"})).is_err());
}

#[test]
fn warp_options_style_change_preserves_slot_and_visibility() {
    let (mut s, id) = session_with_rect();
    for effect in ["path.offsetPath", "warp.arc", "stylize.roundCorners"] {
        s.execute("effect.apply", &json!({"effect": effect, "item": 1})).unwrap();
    }
    s.execute("effect.setParams", &json!({"index": 1, "item": 1, "visible": false})).unwrap();
    s.doc_mut().unwrap().mark_saved();
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let result = s.execute("effect.setParams", &json!({"index": 1, "item": 1, "warpStyle": "flag", "params": {"bend": 25.0}}));
    assert!(result.is_ok(), "an applied warp can change style in place: {result:?}");
    let old = before.node(id).unwrap().appearance.effects_at(Some(1)).unwrap();
    let n = node(&s, id);
    let new = n.appearance.effects_at(Some(1)).unwrap();
    assert_eq!(new.len(), 3);
    assert_eq!(new[0], old[0]);
    assert_eq!(new[2], old[2]);
    assert_eq!(new[1].id, "warp.flag");
    assert_eq!(new[1].params["bend"], 25.0);
    assert!(!new[1].visible);
    assert!(n.appearance.effects.is_empty());
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    let after = s.doc().unwrap().doc.clone();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, before);
    assert!(!s.doc().unwrap().is_dirty());
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, after);
}

#[test]
fn warp_options_non_warp_target_rejection_is_atomic() {
    let (mut s, first) = session_with_rect();
    s.execute("effect.apply", &json!({"effect": "warp.arc", "item": null})).unwrap();
    let second = s.execute("shape.rectangle", &json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].clone();
    s.execute("effect.apply", &json!({"effect": "distort.twist", "item": null})).unwrap();
    s.execute("select.set", &json!({"ids": [first.0, second]})).unwrap();
    s.doc_mut().unwrap().mark_saved();
    let before = s.doc().unwrap().doc.clone();
    let selection = s.doc().unwrap().selection.clone();
    let undo = s.doc().unwrap().history.undo.len();
    assert!(s.execute("effect.setParams", &json!({"index": 0, "item": null, "warpStyle": "flag"})).is_err());
    let state = s.doc().unwrap();
    assert_eq!(state.doc, before);
    assert_eq!(state.selection, selection);
    assert_eq!(state.history.undo.len(), undo);
    assert!(!state.is_dirty());
}

#[test]
fn list_remove_and_set_params() {
    let (mut s, id) = session_with_rect();
    s.execute("effect.apply", &json!({"effect": "distort.twist"})).unwrap();
    s.execute("effect.apply", &json!({"effect": "stylize.outerGlow"})).unwrap();
    let l = s.execute("effect.list", &json!({})).unwrap();
    assert!(l["catalog"].as_array().unwrap().len() >= 34);
    assert_eq!(l["applied"][0]["effects"].as_array().unwrap().len(), 2);
    s.execute("effect.setParams", &json!({"index": 0, "params": {"angle": 45}, "visible": false})).unwrap();
    let e = &node(&s, id).appearance.effects[0];
    assert_eq!(e.params["angle"], json!(45));
    assert!(!e.visible);
    s.execute("effect.remove", &json!({"index": 0})).unwrap();
    let n = node(&s, id);
    assert_eq!(n.appearance.effects.len(), 1);
    assert_eq!(n.appearance.effects[0].id, "stylize.outerGlow");
    assert!(s.execute("effect.remove", &json!({"index": 5})).is_err());
}

#[test]
fn expand_appearance_bakes_geometry() {
    let (mut s, id) = session_with_rect();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("effect.apply", &json!({"effect": "path.offsetPath", "params": {"offset": 10}})).unwrap();
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    // One fill: the path itself, reshaped.
    let n = node(&s, id);
    let b = n.geometric_bounds().unwrap();
    assert!((b.width() - 120.0).abs() < 0.1, "{b:?}");
    assert!(n.appearance.effects.is_empty());
    assert!(matches!(n.kind, NodeKind::Path { live: None, .. }));
    // Nothing left to expand.
    assert!(s.execute("effect.expandAppearance", &json!({})).is_err());
}

#[test]
fn expand_transform_copies_makes_compound() {
    let (mut s, id) = session_with_rect();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("effect.apply", &json!({"effect": "distort.transform", "params": {"moveH": 120, "copies": 1}})).unwrap();
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    let n = node(&s, id);
    assert!(matches!(n.kind, NodeKind::Compound { .. }));
    let b = n.geometric_bounds().unwrap();
    assert!((b.x1 - 320.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn pathfinder_effect_on_loose_objects_groups_them_first() {
    let (mut s, a) = session_with_rect();
    let b = NodeId(s.execute("shape.rectangle", &json!({"x": 150, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let r = s.execute("effect.apply", &json!({"effect": "pathfinder.add"})).unwrap();
    assert_eq!(r["grouped"], true);
    let g = NodeId(r["ids"][0].as_u64().unwrap());
    let gn = node(&s, g);
    assert_eq!(gn.children().unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), [a, b], "stacking order kept");
    assert!(vectorcraft_render::effects::has_pathfinder(&gn));
    assert_eq!(s.doc().unwrap().selection.objects, [g]);
    // Live: one united shape, so expanding leaves a single path.
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    assert_eq!(node(&s, g).children().unwrap().len(), 1);
    // Grouping and applying are one undo step.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.node(g).is_none());
    assert!(d.node(a).is_some_and(|n| n.appearance.effects.is_empty()) && d.parent_of(a) == d.parent_of(b));
    // A selection holding a group applies to each object as before; so does a single object.
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let r = s.execute("effect.apply", &json!({"effect": "pathfinder.add"})).unwrap();
    assert!(r.get("grouped").is_none());
    assert_eq!(node(&s, a).appearance.effects.len(), 1);
}

#[test]
fn pathfinder_effect_on_a_group_with_type_cuts_the_glyphs_out() {
    let (mut s, a) = session_with_rect();
    let t = NodeId(s.execute("text.create", &json!({"x": 110, "y": 180, "text": "H", "size": 100})).unwrap()["id"].as_u64().unwrap());
    let glyphs = effects::outline_text(&node(&s, t)).unwrap();
    let b = glyphs.geometric_bounds().unwrap();
    assert!(b.x0 > 105.0 && b.y0 > 105.0 && b.x1 < 195.0 && b.y1 < 195.0, "the type lies inside the square: {b:?}");
    let area = |n: &vectorcraft_doc::Node| effects::member_shapes(None, n).iter().map(|(p, r)| vectorcraft_pathops::area(p, *r)).sum::<f64>();
    let glyph_area = area(&glyphs);
    s.execute("select.set", &json!({"ids": [a.0, t.0]})).unwrap();
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("effect.apply", &json!({"effect": "pathfinder.subtract"})).unwrap();
    // Live: the renderer leaves the glyph's place in the square empty.
    let doc = s.doc().unwrap().doc.clone();
    let img = vectorcraft_render::Renderer::new().render(&doc, 400, 400, vectorcraft_geom::Affine::IDENTITY, &Default::default());
    let empty = (105..195).flat_map(|y| (105..195).map(move |x| (x, y))).filter(|&(x, y)| img.pixel(x, y)[3] == 0).count();
    assert!(empty as f64 > glyph_area / 2.0, "{empty} empty pixels for a glyph area of {glyph_area}");
    // Expanded: the group holds the square minus the glyph.
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    let left = area(&node(&s, g));
    assert!((left - (10_000.0 - glyph_area)).abs() < 0.5, "{left} left of the square for a glyph area of {glyph_area}");
}

#[test]
fn pathfinder_effect_on_a_group_renders_live_and_expands() {
    let (mut s, a) = session_with_rect();
    let b = s.execute("shape.rectangle", &json!({"x": 150, "y": 100, "width": 100, "height": 100})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b["id"]]})).unwrap();
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("effect.apply", &json!({"effect": "pathfinder.subtract"})).unwrap();
    // Live: the renderer shows only the back square minus the front one.
    let doc = s.doc().unwrap().doc.clone();
    let img = vectorcraft_render::Renderer::new().render(&doc, 400, 400, vectorcraft_geom::Affine::IDENTITY, &Default::default());
    assert!(img.pixel(125, 150)[3] > 0);
    assert_eq!(img.pixel(200, 150)[3], 0);
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    let n = node(&s, g);
    assert!(n.appearance.effects.is_empty());
    let ch = n.children().unwrap();
    assert_eq!(ch.len(), 1);
    assert!((ch[0].geometric_bounds().unwrap().width() - 50.0).abs() < 1e-6);
}
#[test]
fn expand_appearance_gives_each_copy_of_a_transform_its_own_gradient() {
    use vectorcraft_color::{Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint};
    let (mut s, id) = session_with_rect();
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    s.edit("gradient", |d, _| {
        let n = d.node_mut(id).unwrap();
        n.appearance.set_fill(ramp);
        Ok(())
    })
    .unwrap();
    s.execute("effect.apply", &json!({"effect":"distort.transform","ids":[id.0],"item":null})).unwrap();
    s.execute("effect.setParams", &json!({"index":0,"params":{"moveH":150,"copies":2},"ids":[id.0],"item":null})).unwrap();
    s.execute("effect.expandAppearance", &json!({"ids":[id.0]})).unwrap();
    let mut boxes = vec![];
    node(&s, id).walk(&mut |n| {
        if matches!(n.kind, NodeKind::Path { .. }) && matches!(n.appearance.fill_paint(), Paint::Gradient(_)) {
            boxes.push(n.geometric_bounds().unwrap().width());
        }
    });
    assert_eq!(boxes, vec![100.0; 3], "three squares, each with its own gradient");
}
