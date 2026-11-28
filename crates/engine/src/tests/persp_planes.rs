//! Perspective planes: offsets, plane widgets (alone, Shift with the objects, Alt with copies),
//! Move Plane to Match Object, and grid edits with attached art.

use serde_json::json;
use vectorcraft_geom::{Point, Vec2};
use vectorcraft_tools::Mods;
use vectorcraft_tools::distort::perspective::{PerspectiveGrid, Plane};

use crate::tests_persp_select::{attached, drag, grid, persp_session, plane_pts, undo_len};
use crate::*;

fn path(s: &Session, id: NodeId) -> vectorcraft_geom::PathData {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().clone()
}

#[test]
fn moving_a_plane_alone_leaves_its_objects() {
    let (mut s, id) = persp_session();
    let before = path(&s, id);
    let n = undo_len(&s);
    let r = s.execute("perspective.plane.move", &json!({"plane": "right", "offset": 50})).unwrap();
    assert_eq!((r["plane"].as_str(), r["offset"].as_f64()), (Some("right"), Some(50.0)));
    assert_eq!(undo_len(&s), n + 1);
    let g = grid(&s);
    assert_eq!(g.right_offset, 50.0);
    // The plane is drawn where it was moved: its origin corner lies 50 pt along its normal.
    let o = g.homography(Plane::Right).unwrap().apply(Point::ZERO).unwrap();
    assert!((o - g.homography_at(Plane::Right, 50.0).unwrap().apply(Point::ZERO).unwrap()).hypot() < 1e-9);
    assert!((o - Point::new(g.origin[0], g.origin[1])).hypot() > 1.0);
    assert_eq!(path(&s, id), before);
    assert_eq!(attached(&s, id), Some((Plane::Right, 0.0)));
    // Relative moves; new objects attach where the plane is.
    s.execute("perspective.plane.move", &json!({"plane": "right", "by": -20})).unwrap();
    assert_eq!(grid(&s).right_offset, 30.0);
    let r = s.execute("shape.rectangle", &json!({"x": 470, "y": 300, "width": 30, "height": 30})).unwrap();
    let b = NodeId(r["id"].as_u64().unwrap());
    s.execute("perspective.attach", &json!({"ids": [b.0], "plane": "right"})).unwrap();
    assert_eq!(attached(&s, b), Some((Plane::Right, 30.0)));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(grid(&s).right_offset, 50.0);
    assert!(s.execute("perspective.plane.move", &json!({"plane": "right"})).is_err());
    assert!(s.execute("perspective.plane.move", &json!({"plane": "right", "offset": 1, "objects": "all"})).is_err());
    assert!(s.execute("perspective.plane.move", &json!({"plane": "right", "offset": -1e9})).is_err(), "behind the viewer");
}

#[test]
fn moving_a_plane_with_its_objects_or_copies() {
    let (mut s, id) = persp_session();
    // A second object moved off the plane along its normal stays put.
    let r = s.execute("shape.rectangle", &json!({"x": 560, "y": 330, "width": 30, "height": 30})).unwrap();
    let off = NodeId(r["id"].as_u64().unwrap());
    s.execute("perspective.attach", &json!({"ids": [off.0], "plane": "right"})).unwrap();
    s.execute("perspective.transform", &json!({"ids": [off.0], "matrix": [1, 0, 0, 1, 0, 0], "depth": 20})).unwrap();
    let off_path = path(&s, off);
    let q = plane_pts(&s, id, Plane::Right, 0.0);
    let r = s.execute("perspective.plane.move", &json!({"plane": "right", "offset": 35, "objects": "move"})).unwrap();
    assert_eq!(r["ids"], json!([id.0]));
    assert_eq!(attached(&s, id), Some((Plane::Right, 35.0)));
    for (a, b) in q.iter().zip(plane_pts(&s, id, Plane::Right, 35.0)) {
        assert!((*a - b).hypot() < 1e-6, "keeps its place on the plane");
    }
    assert_eq!(path(&s, off), off_path);
    // Copies: the originals stay, the copies go with the plane.
    let r = s.execute("perspective.plane.move", &json!({"plane": "right", "offset": 60, "objects": "copy"})).unwrap();
    let copy = NodeId(r["ids"][0].as_u64().unwrap());
    assert_ne!(copy, id);
    assert_eq!(attached(&s, id), Some((Plane::Right, 35.0)));
    assert_eq!(attached(&s, copy), Some((Plane::Right, 60.0)));
    for (a, b) in q.iter().zip(plane_pts(&s, copy, Plane::Right, 60.0)) {
        assert!((*a - b).hypot() < 1e-6);
    }
}

#[test]
fn plane_widgets_drag_with_shift_and_alt() {
    let (mut s, id) = persp_session();
    s.execute("perspective.grid.show", &json!({"visible": true})).unwrap();
    for tool in ["perspectiveGrid", "perspectiveSelection"] {
        s.select_tool(tool, ViewInfo::default()).unwrap();
        let g = grid(&s);
        let (_, w) = g.plane_widgets().into_iter().find(|(p, _)| *p == Plane::Right).unwrap();
        let from = g.offset(Plane::Right);
        // Where the widget lands 15 pt further along the plane's normal.
        let to = g
            .homography_at(Plane::Right, from + 15.0)
            .unwrap()
            .apply(g.homography(Plane::Right).unwrap().inverse().unwrap().apply(w).unwrap())
            .unwrap();
        let n = undo_len(&s);
        drag(&mut s, &[w, to, to], Mods { shift: true, ..Default::default() });
        assert_eq!(undo_len(&s), n + 1, "{tool}");
        let (cmd, p) = s.journal.last().unwrap().clone();
        assert_eq!((cmd.as_str(), p["objects"].as_str()), ("perspective.plane.move", Some("move")));
        assert!((grid(&s).right_offset - from - 15.0).abs() < 1e-6, "{tool}: {}", grid(&s).right_offset);
        assert!((attached(&s, id).unwrap().1 - from - 15.0).abs() < 1e-6, "{tool}: the object went along");
    }
    // Alt copies the objects; a plain drag moves the plane alone.
    let g = grid(&s);
    let (_, w) = g.plane_widgets().into_iter().find(|(p, _)| *p == Plane::Right).unwrap();
    let count = |s: &Session| s.doc().unwrap().doc.layers[0].children().unwrap().len();
    let k = count(&s);
    drag(&mut s, &[w, w + Vec2::new(-12.0, 0.0), w + Vec2::new(-12.0, 0.0)], Mods { alt: true, ..Default::default() });
    assert_eq!(count(&s), k + 1);
    let g = grid(&s);
    let (_, w) = g.plane_widgets().into_iter().find(|(p, _)| *p == Plane::Right).unwrap();
    let depth = attached(&s, id).unwrap().1;
    drag(&mut s, &[w, w + Vec2::new(-12.0, 0.0), w + Vec2::new(-12.0, 0.0)], Mods::default());
    assert_eq!(attached(&s, id).unwrap().1, depth);
    assert_eq!(count(&s), k + 1);
}

#[test]
fn double_clicking_a_plane_widget_asks_for_its_options() {
    let (mut s, _) = persp_session();
    s.execute("perspective.grid.show", &json!({"visible": true})).unwrap();
    let v = ViewInfo::default();
    let (_, w) = grid(&s).plane_widgets().into_iter().find(|(p, _)| *p == Plane::Ground).unwrap();
    for tool in ["perspectiveGrid", "perspectiveSelection"] {
        s.select_tool(tool, v).unwrap();
        let r = s.pointer(&vectorcraft_tools::PointerEvent::new(vectorcraft_tools::PointerKind::DoubleClick, w.x, w.y), v).unwrap();
        assert_eq!(r, vec![crate::UiRequest::Dialog("perspectivePlane".into(), json!({"plane": "ground"}))], "{tool}");
    }
}

#[test]
fn move_plane_to_match_object() {
    let (mut s, id) = persp_session();
    let q0 = plane_pts(&s, id, Plane::Right, 0.0);
    s.execute("perspective.transform", &json!({"matrix": [1, 0, 0, 1, 0, 0], "depth": -25})).unwrap();
    s.execute("perspective.plane.set", &json!({"plane": "ground"})).unwrap();
    let r = s.execute("perspective.plane.matchObject", &json!({})).unwrap();
    assert_eq!(r, json!({"plane": "right", "offset": -25.0}));
    let g = grid(&s);
    assert_eq!((g.right_offset, g.plane), (-25.0, Plane::Right));
    // The object now lies on the plane where it is drawn.
    let h = g.homography(Plane::Right).unwrap();
    for (q, a) in q0.iter().zip(s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().anchors()) {
        assert!((h.apply(*q).unwrap() - a.2.p).hypot() < 1e-6);
    }
    s.execute("perspective.release", &json!({"ids": [id.0]})).unwrap();
    assert!(s.execute("perspective.plane.matchObject", &json!({"id": id.0})).is_err());
}

#[test]
fn grid_edits_keep_attached_art_in_place_unless_reprojected() {
    let (mut s, id) = persp_session();
    let before = path(&s, id);
    let q = plane_pts(&s, id, Plane::Right, 0.0);
    let vr = grid(&s).vp_right;
    s.execute("perspective.grid.set", &json!({"vpRight": vr + 150.0})).unwrap();
    assert_eq!(path(&s, id), before, "as in the reference app, art stays where it is");
    s.execute("perspective.grid.set", &json!({"vpRight": vr - 100.0, "reproject": true})).unwrap();
    assert_ne!(path(&s, id), before);
    // Re-projected from the previous grid: the same place on the plane of the new one.
    let old = {
        let mut g = grid(&s);
        g.vp_right = vr + 150.0;
        g
    };
    let hi = old.homography(Plane::Right).unwrap().inverse().unwrap();
    let back: Vec<Point> = before.anchors().map(|(_, _, a)| hi.apply(a.p).unwrap()).collect();
    for (a, b) in back.iter().zip(plane_pts(&s, id, Plane::Right, 0.0)) {
        assert!((*a - b).hypot() < 1e-6, "{a:?} {b:?}");
    }
    assert!(q.len() == back.len());
}

#[test]
fn plane_offsets_round_trip_and_old_grids_have_none() {
    let (mut s, _) = persp_session();
    s.execute("perspective.grid.set", &json!({"leftOffset": 12, "groundOffset": -8})).unwrap();
    let doc = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let g = PerspectiveGrid::from_doc(&doc).unwrap();
    assert_eq!((g.left_offset, g.right_offset, g.ground_offset), (12.0, 0.0, -8.0));
    let mut stored = doc.unknown["perspectiveGrid"].clone();
    assert!(stored.get("rightOffset").is_none(), "zero offsets aren't written");
    for k in ["leftOffset", "groundOffset"] {
        stored.as_object_mut().unwrap().remove(k);
    }
    let old: PerspectiveGrid = serde_json::from_value(stored).unwrap();
    assert_eq!((old.left_offset, old.ground_offset), (0.0, 0.0));
}
