use serde_json::json;
use vectorcraft_doc::{LiveShape, NodeId, NodeKind};
use vectorcraft_geom::{Affine, Rect, shapes};
use vectorcraft_tools::addons::box3d::CustomShape;

use crate::Session;

fn new_session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s
}

#[test]
fn default_shapes_do_not_alter_standard_documents() {
    let mut s = new_session();

    // Default rectangle: standard LiveShape::Rectangle, no vc-shape-tools attribute.
    let rect_id =
        NodeId(s.execute("shape.rectangle", &json!({"x": 10.0, "y": 20.0, "width": 100.0, "height": 50.0})).unwrap()["id"].as_u64().unwrap());
    let n = s.doc().unwrap().doc.node(rect_id).unwrap();
    assert!(n.extras().attrs.is_none());
    let NodeKind::Path { path, live: Some(live), .. } = &n.kind else { panic!("expected live path") };
    assert_eq!(
        **live,
        LiveShape::Rectangle {
            w: 100.0,
            h: 50.0,
            radii: [0.0; 4],
            kinds: Default::default(),
            radii_relative: false,
            xf: Affine::translate((10.0, 20.0))
        }
    );
    assert_eq!(path, &shapes::rectangle(Rect::new(10.0, 20.0, 110.0, 70.0)));

    // Default ellipse: standard LiveShape::Ellipse, no vc-shape-tools attribute.
    let ell_id = NodeId(s.execute("shape.ellipse", &json!({"x": 0.0, "y": 0.0, "width": 80.0, "height": 60.0})).unwrap()["id"].as_u64().unwrap());
    let n = s.doc().unwrap().doc.node(ell_id).unwrap();
    assert!(n.extras().attrs.is_none());
    let NodeKind::Path { path, live: Some(live), .. } = &n.kind else { panic!("expected live path") };
    assert_eq!(**live, LiveShape::Ellipse { w: 80.0, h: 60.0, pie: (0.0, 360.0), xf: Affine::IDENTITY });
    assert_eq!(path, &shapes::ellipse(Rect::new(0.0, 0.0, 80.0, 60.0)));
}

#[test]
fn addon_shapes_commands_and_format_roundtrip_preserve_paths() {
    let mut s = new_session();

    // Create customizable shapes through addon.shapes.* commands.
    let box_id = NodeId(
        s.execute(
            "addon.shapes.box3d",
            &json!({"x": 10.0, "y": 10.0, "width": 100.0, "height": 80.0, "angleX": 30.0, "angleY": 35.0, "angleZ": 0.0}),
        )
        .unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    let n = s.doc().unwrap().doc.node(box_id).unwrap();
    let NodeKind::Path { path, .. } = &n.kind else { panic!("expected path") };
    assert_eq!(path.subpaths.len(), 3);
    assert!(
        matches!(CustomShape::read(n), Some(CustomShape::Box3d { angle_x, angle_y, .. }) if (angle_x - 30.0).abs() < 1e-6 && (angle_y - 35.0).abs() < 1e-6)
    );

    s.execute("addon.shapes.configure", &json!({"angleX": 45.0, "parallelX": true})).unwrap();
    let n = s.doc().unwrap().doc.node(box_id).unwrap();
    assert!(matches!(CustomShape::read(n), Some(CustomShape::Box3d { angle_x, parallel_x: true, .. }) if (angle_x - 45.0).abs() < 1e-6));

    let ell_id = NodeId(
        s.execute(
            "addon.shapes.ellipse",
            &json!({"x": 0.0, "y": 0.0, "width": 100.0, "height": 100.0, "arcMode": "arc", "startAngle": 30.0, "endAngle": 210.0}),
        )
        .unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    let n = s.doc().unwrap().doc.node(ell_id).unwrap();
    let NodeKind::Path { path, .. } = &n.kind else { panic!("expected path") };
    assert!(!path.subpaths[0].closed);

    s.execute("addon.shapes.configure", &json!({"arcMode": "chord"})).unwrap();
    let n = s.doc().unwrap().doc.node(ell_id).unwrap();
    let NodeKind::Path { path, .. } = &n.kind else { panic!("expected path") };
    assert!(path.subpaths[0].closed);

    let rect_id = NodeId(
        s.execute("addon.shapes.rectangle", &json!({"x": 0.0, "y": 0.0, "width": 100.0, "height": 80.0, "radii": [10.0, 20.0, 5.0, 0.0], "arcMode": "pie", "startAngle": 0.0, "endAngle": 180.0}))
            .unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    let n = s.doc().unwrap().doc.node(rect_id).unwrap();
    assert!(
        matches!(CustomShape::read(n), Some(CustomShape::Rectangle { radii, arc_mode: shapes::EllipseMode::Slice, .. }) if radii == [10.0, 20.0, 5.0, 0.0])
    );

    let poly_id = NodeId(
        s.execute("addon.shapes.polygon", &json!({"cx": 50.0, "cy": 50.0, "radius": 40.0, "sides": 5, "rounded": 0.25, "randomized": 0.1})).unwrap()
            ["id"]
            .as_u64()
            .unwrap(),
    );
    s.execute("addon.shapes.configure", &json!({"isStar": true, "spokeRatio": 0.4})).unwrap();
    let n = s.doc().unwrap().doc.node(poly_id).unwrap();
    assert!(
        matches!(CustomShape::read(n), Some(CustomShape::Polygon { spoke_ratio: Some(sr), rounded, .. }) if (sr - 0.4).abs() < 1e-6 && (rounded - 0.25).abs() < 1e-6)
    );

    let spiral_id = NodeId(
        s.execute("addon.shapes.spiral", &json!({"cx": 60.0, "cy": 60.0, "radius": 45.0, "turns": 4.0, "divergence": 1.4, "innerRadius": 0.2}))
            .unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    let n = s.doc().unwrap().doc.node(spiral_id).unwrap();
    assert!(
        matches!(CustomShape::read(n), Some(CustomShape::Spiral { turns, divergence, .. }) if (turns - 4.0).abs() < 1e-6 && (divergence - 1.4).abs() < 1e-6)
    );

    // Verify that saving and reopening a document containing add-on shapes preserves both the
    // baked PathData/LiveShape (for a build that ignores the add-on attribute) and the stored
    // extras attribute across a save/load roundtrip.
    let doc_before = s.doc().unwrap().doc.clone();
    let bytes = vectorcraft_format::save(&doc_before, false);
    let doc_after = vectorcraft_format::load(&bytes).unwrap();
    let re_saved = vectorcraft_format::save(&doc_after, false);
    assert_eq!(bytes, re_saved);
    for id in [box_id, ell_id, rect_id, poly_id, spiral_id] {
        let a = doc_before.node(id).unwrap();
        let b = doc_after.node(id).unwrap();
        assert_eq!(a.kind, b.kind);
        assert_eq!(CustomShape::read(a), CustomShape::read(b));
    }
}

fn id_of(s: &mut Session, cmd: &str, p: serde_json::Value) -> NodeId {
    NodeId(s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))["id"].as_u64().unwrap())
}

fn shape_of(s: &Session, id: NodeId) -> CustomShape {
    CustomShape::read(s.doc().unwrap().doc.node(id).unwrap()).unwrap()
}

/// Runs `cmd` with `p`, which must fail with `msg` and leave the document and its history as they
/// were.
fn refused(s: &mut Session, cmd: &str, p: serde_json::Value, msg: &str) {
    let saved = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    let undo = s.doc().unwrap().history.undo.len();
    let e = s.execute(cmd, &p).map(|v| v.to_string()).unwrap_err().to_string();
    assert!(e.contains(msg), "{cmd} {p}: {e}");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "{cmd} {p}");
    assert_eq!(vectorcraft_format::save(&s.doc().unwrap().doc, false), saved, "{cmd} {p}");
}

#[test]
fn polygon_and_star_refuse_counts_that_are_not_whole_numbers_from_3_to_1000() {
    let mut s = new_session();
    for n in [json!(2.5), json!(2), json!(0), json!(-6), json!(1000.5), json!(5000), json!(4_294_967_299u64)] {
        let x = n.as_f64().unwrap();
        let p = json!({"cx": 0.0, "cy": 0.0, "radius": 10.0, "sides": n});
        refused(&mut s, "addon.shapes.polygon", p, &format!("`sides` must be a whole number between 3 and 1000 (got {x})"));
        let p = json!({"cx": 0.0, "cy": 0.0, "radius1": 10.0, "points": n});
        refused(&mut s, "addon.shapes.star", p, &format!("`points` must be a whole number between 3 and 1000 (got {x})"));
    }
    refused(&mut s, "addon.shapes.polygon", json!({"cx": 0.0, "cy": 0.0, "radius": 10.0, "sides": "6"}), "`sides` must be a number");
    refused(&mut s, "addon.shapes.star", json!({"cx": 0.0, "cy": 0.0, "radius1": 10.0, "points": [5]}), "`points` must be a number");

    // The ends of the range are taken as given, and an omitted or null count is the default.
    for (n, want) in [(json!(3), 3), (json!(1000), 1000), (json!(7.0), 7), (json!(null), 6)] {
        let id = id_of(&mut s, "addon.shapes.polygon", json!({"cx": 0.0, "cy": 0.0, "radius": 10.0, "sides": n}));
        assert!(matches!(shape_of(&s, id), CustomShape::Polygon { sides, spoke_ratio: None, .. } if sides == want), "{n}");
    }
    let id = id_of(&mut s, "addon.shapes.polygon", json!({"cx": 0.0, "cy": 0.0, "radius": 10.0}));
    assert!(matches!(shape_of(&s, id), CustomShape::Polygon { sides: 6, .. }));
    for (n, want) in [(json!(3), 3), (json!(1000), 1000), (json!(null), 5)] {
        let id = id_of(&mut s, "addon.shapes.star", json!({"cx": 0.0, "cy": 0.0, "radius1": 10.0, "points": n}));
        assert!(matches!(shape_of(&s, id), CustomShape::Polygon { sides, spoke_ratio: Some(_), .. } if sides == want), "{n}");
    }
}

#[test]
fn shapes_refuse_numbers_outside_their_ranges() {
    let mut s = new_session();
    let poly = json!({"cx": 0.0, "cy": 0.0, "radius": 10.0});
    let star = json!({"cx": 0.0, "cy": 0.0, "radius1": 10.0});
    let spiral = json!({"cx": 0.0, "cy": 0.0, "radius": 10.0});
    let with = |base: &serde_json::Value, key: &str, v: f64| {
        let mut p = base.clone();
        p[key] = json!(v);
        p
    };
    for (cmd, base, key, bad, good) in [
        ("addon.shapes.polygon", &poly, "rounded", [-10.5, 10.5], [-10.0, 10.0]),
        ("addon.shapes.polygon", &poly, "randomized", [-11.0, 10.01], [-10.0, 10.0]),
        ("addon.shapes.star", &star, "rounded", [-10.5, 10.5], [-10.0, 10.0]),
        ("addon.shapes.star", &star, "randomized", [-11.0, 10.01], [-10.0, 10.0]),
        ("addon.shapes.star", &star, "radius2", [0.05, 11.0], [0.1, 10.0]),
        ("addon.shapes.spiral", &spiral, "turns", [0.0, 1024.5], [0.01, 1024.0]),
        ("addon.shapes.spiral", &spiral, "divergence", [-0.1, 1000.5], [0.0, 1000.0]),
        ("addon.shapes.spiral", &spiral, "innerRadius", [-0.1, 1.0], [0.0, 0.999]),
    ] {
        let (min, max) = (good[0], good[1]);
        for x in bad {
            refused(&mut s, cmd, with(base, key, x), &format!("`{key}` must be between {min} and {max} (got {x})"));
        }
        for x in good {
            s.execute(cmd, &with(base, key, x)).unwrap_or_else(|e| panic!("{cmd} {key} {x}: {e}"));
        }
    }

    // Sizes are not negative.
    for (cmd, p, key) in [
        ("addon.shapes.polygon", json!({"cx": 0.0, "cy": 0.0, "radius": -10.0}), "radius"),
        ("addon.shapes.star", json!({"cx": 0.0, "cy": 0.0, "radius1": -10.0}), "radius1"),
        ("addon.shapes.spiral", json!({"cx": 0.0, "cy": 0.0, "radius": -10.0}), "radius"),
        ("addon.shapes.rectangle", json!({"x": 0.0, "y": 0.0, "width": -10.0, "height": 10.0}), "width"),
        ("addon.shapes.rectangle", json!({"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0, "radius": -1.0}), "radius"),
        ("addon.shapes.ellipse", json!({"x": 0.0, "y": 0.0, "width": 10.0, "height": -10.0}), "height"),
        ("addon.shapes.box3d", json!({"x": 0.0, "y": 0.0, "width": -10.0, "height": 10.0}), "width"),
    ] {
        refused(&mut s, cmd, p, &format!("`{key}` must be a number not below 0"));
    }

    // Corner radii are four numbers that are not negative.
    for radii in [json!([1.0, 2.0, 3.0]), json!([1.0, 2.0, 3.0, -4.0]), json!([1.0, 2.0, 3.0, "4"]), json!(4.0)] {
        let p = json!({"x": 0.0, "y": 0.0, "width": 100.0, "height": 80.0, "radii": radii});
        refused(&mut s, "addon.shapes.rectangle", p, "`radii` must be four numbers not below 0: [tl, tr, br, bl]");
    }

    // An arc mode is one of the names the commands take.
    for mode in [json!("slices"), json!(""), json!(3)] {
        let p = json!({"x": 0.0, "y": 0.0, "width": 100.0, "height": 80.0, "arcMode": mode});
        refused(&mut s, "addon.shapes.rectangle", p.clone(), "`arcMode` must be one of whole, pie, chord, arc");
        refused(&mut s, "addon.shapes.ellipse", p, "`arcMode` must be one of whole, pie, chord, arc");
    }
    for mode in ["whole", "pie", "slice", "chord", "arc", "open"] {
        s.execute("addon.shapes.ellipse", &json!({"x": 0.0, "y": 0.0, "width": 100.0, "height": 80.0, "arcMode": mode})).unwrap();
    }
}

#[test]
fn configure_refuses_values_outside_their_ranges_before_changing_any_shape() {
    let mut s = new_session();
    let rect = id_of(&mut s, "addon.shapes.rectangle", json!({"x": 0.0, "y": 0.0, "width": 100.0, "height": 80.0, "radius": 4.0}));
    let poly = id_of(&mut s, "addon.shapes.polygon", json!({"cx": 50.0, "cy": 50.0, "radius": 40.0, "sides": 5}));
    let spiral = id_of(&mut s, "addon.shapes.spiral", json!({"cx": 60.0, "cy": 60.0, "radius": 45.0}));
    let ids = json!([rect.0, poly.0, spiral.0]);
    let cfg = |key: &str, v: serde_json::Value| json!({"ids": ids.clone(), key: v});
    let c = "addon.shapes.configure";
    for n in [json!(2.5), json!(2), json!(-1), json!(5000), json!(4_294_967_299u64)] {
        let x = n.as_f64().unwrap();
        refused(&mut s, c, cfg("sides", n), &format!("`sides` must be a whole number between 3 and 1000 (got {x})"));
    }
    for (key, bad, min, max) in [
        ("spokeRatio", [0.0, 1.5], 0.01, 1.0),
        ("rounded", [-10.5, 11.0], -10.0, 10.0),
        ("randomized", [-11.0, 10.5], -10.0, 10.0),
        ("turns", [0.0, 2000.0], 0.01, 1024.0),
        ("divergence", [-1.0, 1000.5], 0.0, 1000.0),
        ("innerRadius", [-0.5, 1.0], 0.0, 0.999),
    ] {
        for x in bad {
            refused(&mut s, c, cfg(key, json!(x)), &format!("`{key}` must be between {min} and {max} (got {x})"));
        }
    }
    refused(&mut s, c, cfg("radius", json!(-1.0)), "`radius` must be a number not below 0");
    for radii in [json!([1.0, 2.0]), json!([1.0, 2.0, 3.0, -1.0]), json!("4")] {
        refused(&mut s, c, cfg("radii", radii), "`radii` must be four numbers not below 0: [tl, tr, br, bl]");
    }
    refused(&mut s, c, cfg("arcMode", json!("bogus")), "`arcMode` must be one of whole, pie, chord, arc");
    // A valid value next to a refused one changes no shape either.
    refused(&mut s, c, json!({"ids": ids.clone(), "sides": 8, "turns": 0.0}), "`turns` must be between 0.01 and 1024 (got 0)");

    // In range, the values are taken as given; a radius larger than the rectangle takes is fitted to it.
    s.execute(c, &json!({"ids": [poly.0], "sides": 1000, "rounded": -10.0, "randomized": 10.0})).unwrap();
    assert!(matches!(shape_of(&s, poly), CustomShape::Polygon { sides: 1000, rounded, randomized, .. } if rounded == -10.0 && randomized == 10.0));
    s.execute(c, &json!({"ids": [poly.0], "sides": 3})).unwrap();
    assert!(matches!(shape_of(&s, poly), CustomShape::Polygon { sides: 3, .. }));
    s.execute(c, &json!({"ids": [spiral.0], "turns": 1024.0, "divergence": 0.0, "innerRadius": 0.999})).unwrap();
    assert!(
        matches!(shape_of(&s, spiral), CustomShape::Spiral { turns, divergence, inner_radius, .. } if turns == 1024.0 && divergence == 0.0 && inner_radius == 0.999)
    );
    s.execute(c, &json!({"ids": [rect.0], "radius": 1000.0})).unwrap();
    assert!(matches!(shape_of(&s, rect), CustomShape::Rectangle { radii, .. } if radii == [40.0; 4]));
}

#[test]
fn configure_records_no_undo_step_when_nothing_changes() {
    let mut s = new_session();
    let poly = id_of(&mut s, "addon.shapes.polygon", json!({"cx": 50.0, "cy": 50.0, "radius": 40.0, "sides": 5, "rounded": 0.5}));
    let undo = s.doc().unwrap().history.undo.len();
    let r = s.execute("addon.shapes.configure", &json!({"ids": [poly.0], "sides": 5, "rounded": 0.5})).unwrap();
    assert_eq!(r["updated"], 0);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);

    let r = s.execute("addon.shapes.configure", &json!({"ids": [poly.0], "sides": 6})).unwrap();
    assert_eq!(r["updated"], 1);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert!(matches!(shape_of(&s, poly), CustomShape::Polygon { sides: 6, .. }));
}
