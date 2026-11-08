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
