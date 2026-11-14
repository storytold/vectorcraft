//! Automatic Plane Positioning: Shift held over an anchor point of art in perspective or over a
//! gridline intersection moves the active plane through it for now (`perspective.plane.infer`),
//! the two Perspective Grid Options that turn the targets on and off, and what ends it.

use serde_json::json;
use vectorcraft_geom::{Point, Vec2};
use vectorcraft_tools::distort::perspective::{PerspectiveGrid, Plane};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::tests_persp_select::{attached, bounds_of, drag, grid, persp_session, plane_pts, undo_len};
use crate::*;

fn shift() -> Mods {
    Mods { shift: true, ..Default::default() }
}

/// The top anchor of path `id` (on the right wall at depth 0): its page point and its height.
fn top_anchor(s: &Session, id: NodeId) -> (Point, f64) {
    let page: Vec<Point> = s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().anchors().map(|(_, _, a)| a.p).collect();
    let plane = plane_pts(s, id, Plane::Right, 0.0);
    page.into_iter().zip(plane).map(|(p, q)| (p, q.y)).max_by(|a, b| a.1.total_cmp(&b.1)).unwrap()
}

fn hover(s: &mut Session, p: Point, mods: Mods) {
    s.pointer(&PointerEvent::new(PointerKind::Move, p.x, p.y).with_mods(mods), ViewInfo::default()).unwrap();
}

fn revision(s: &Session) -> u64 {
    s.doc().unwrap().revision
}

#[test]
fn shift_over_an_anchor_holds_the_active_plane_at_its_height_until_an_object_is_drawn() {
    let (mut s, id) = persp_session();
    let v = ViewInfo::default();
    let (anchor, height) = top_anchor(&s, id);
    assert!(height > 10.0, "{height}");
    s.execute("perspective.plane.set", &json!({"plane": "ground"})).unwrap();
    s.select_tool("perspectiveSelection", v).unwrap();
    let steps = undo_len(&s);
    // Without Shift the pointer only hovers.
    hover(&mut s, anchor, Mods::default());
    assert_eq!(grid(&s).inferred, None);
    hover(&mut s, anchor, shift());
    let g = grid(&s);
    assert!(g.inferred.is_some());
    assert!((g.offset(Plane::Ground) - height).abs() < 1e-6, "{} {height}", g.offset(Plane::Ground));
    assert_eq!((g.ground_offset, g.shown_planes(), undo_len(&s)), (0.0, vec![Plane::Ground], steps));
    // Held there already, another hover over the anchor changes nothing.
    let rev = revision(&s);
    hover(&mut s, anchor, shift());
    assert_eq!(revision(&s), rev);
    // The Rectangle tool draws on the plane where it is held, and the plane stays put for the drag.
    s.select_tool("rectangle", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 420.0, 400.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 470.0, 430.0), v).unwrap();
    assert!(grid(&s).inferred.is_some(), "held while the object is drawn");
    s.pointer(&PointerEvent::new(PointerKind::Up, 470.0, 430.0), v).unwrap();
    let drawn = s.doc().unwrap().selection.objects[0];
    assert_ne!(drawn, id);
    let (plane, depth) = attached(&s, drawn).unwrap();
    assert!(plane == Plane::Ground && (depth - height).abs() < 1e-6, "{plane:?} {depth}");
    // Drawn, the plane is back in its place and the other planes show: one undo step for the drag.
    let g = grid(&s);
    assert_eq!((g.inferred, g.offset(Plane::Ground), g.shown_planes().len(), undo_len(&s)), (None, 0.0, 3, steps + 1));
    // Undoing the drag holds the plane again; Escape with the Perspective Selection tool ends that.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(grid(&s).inferred.is_some());
    s.select_tool("perspectiveSelection", v).unwrap();
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    assert_eq!(grid(&s).inferred, None);
}

#[test]
fn shift_during_a_drag_still_constrains_the_move() {
    let (mut s, id) = persp_session();
    let v = ViewInfo::default();
    s.select_tool("perspectiveSelection", v).unwrap();
    let before = plane_pts(&s, id, Plane::Right, 0.0);
    let h = grid(&s).homography(Plane::Right).unwrap();
    // From the middle of the object, along the wall and a little up.
    let start = bounds_of(&before).center();
    let (from, to) = (h.apply(start).unwrap(), h.apply(start + Vec2::new(80.0, 20.0)).unwrap());
    drag(&mut s, &[from, to, to], shift());
    assert_eq!(grid(&s).inferred, None, "with a button down Shift constrains the move");
    for (a, b) in before.iter().zip(plane_pts(&s, id, Plane::Right, 0.0)) {
        assert!((b - *a - Vec2::new(80.0, 0.0)).hypot() < 1e-6, "{a:?} → {b:?}");
    }
}

#[test]
fn a_gridline_intersection_brings_each_plane_made_active_through_it() {
    let (mut s, _) = persp_session();
    let (g, steps) = (grid(&s), undo_len(&s));
    s.execute("perspective.plane.set", &json!({"plane": "left"})).unwrap();
    let at = g.to_page(Plane::Left, Point::new(3.0 * g.cell, 2.0 * g.cell)).unwrap();
    let r = s.execute("perspective.plane.infer", &json!({"at": [at.x, at.y], "tolerance": 1})).unwrap();
    assert_eq!((r["active"].as_bool(), r["source"].as_str(), r["plane"].as_str()), (Some(true), Some("gridline"), Some("left")));
    for (got, want) in r["point"].as_array().unwrap().iter().zip([0.0, 2.0 * g.cell, 3.0 * g.cell]) {
        assert!((got.as_f64().unwrap() - want).abs() < 1e-6, "{r}");
    }
    for (plane, want) in [("ground", 2.0 * g.cell), ("right", 3.0 * g.cell), ("left", 0.0)] {
        s.execute("perspective.plane.set", &json!({"plane": plane})).unwrap();
        let r = s.execute("perspective.plane.infer", &json!({})).unwrap();
        assert!((r["offset"].as_f64().unwrap() - want).abs() < 1e-6 && r["active"] == json!(true) && r.get("source").is_none(), "{plane}: {r}");
    }
    let held = grid(&s);
    assert_eq!((held.left_offset, held.right_offset, held.ground_offset, undo_len(&s)), (0.0, 0.0, 0.0, steps));
    // Nothing within reach leaves the plane where it is held.
    let rev = revision(&s);
    let mid = held.to_page(Plane::Left, Point::new(3.5 * g.cell, 2.5 * g.cell)).unwrap();
    let r = s.execute("perspective.plane.infer", &json!({"at": [mid.x, mid.y], "tolerance": 0.5})).unwrap();
    assert!(r["active"] == json!(true) && r.get("source").is_none() && revision(&s) == rev, "{r}");
    // With no active plane nothing is held.
    s.execute("perspective.plane.set", &json!({"plane": "none"})).unwrap();
    assert_eq!(grid(&s).inferred, None);
    assert!(s.execute("perspective.plane.infer", &json!({"at": [at.x, at.y]})).is_err(), "no active plane");
    assert_eq!(s.execute("perspective.plane.infer", &json!({})).unwrap()["active"], json!(false));
}

#[test]
fn the_two_options_turn_the_targets_on_and_off_and_are_kept() {
    let (mut s, id) = persp_session();
    assert!(s.prefs.perspective_widget.anchor_point && s.prefs.perspective_widget.gridline_intersection, "both on by default");
    let (anchor, _) = top_anchor(&s, id);
    let infer = |s: &mut Session| s.execute("perspective.plane.infer", &json!({"at": [anchor.x, anchor.y], "tolerance": 0.01}));
    assert_eq!(infer(&mut s).unwrap()["source"], json!("anchor"));
    s.execute("perspective.plane.infer", &json!({"clear": true})).unwrap();
    let r = s.execute("perspective.widget.options", &json!({"anchorPoint": false})).unwrap();
    assert_eq!((r["anchorPoint"].as_bool(), r["gridlineIntersection"].as_bool(), r["show"].as_bool()), (Some(false), Some(true), Some(true)));
    assert!(infer(&mut s).unwrap().get("source").is_none(), "the anchor is no target now");
    assert_eq!(grid(&s).inferred, None);
    let back: Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert!(!back.perspective_widget.anchor_point && back.perspective_widget.gridline_intersection);
    s.execute("perspective.widget.options", &json!({"gridlineIntersection": false})).unwrap();
    assert!(infer(&mut s).is_err(), "both off");
    for bad in [json!({"anchorPoint": "yes"}), json!({"gridlineIntersection": 1})] {
        assert!(s.execute("perspective.widget.options", &bad).is_err(), "{bad}");
    }
    assert!(!s.prefs.perspective_widget.anchor_point, "a refused value changes nothing");
    s.execute("prefs.set", &json!({"key": "perspectiveWidget", "value": {"anchorPoint": true}})).unwrap();
    assert!(s.prefs.perspective_widget.anchor_point && !s.prefs.perspective_widget.gridline_intersection);
    // Preferences saved before the options existed read as both on.
    let old: vectorcraft_tools::distort::perspective::widget::WidgetOptions = serde_json::from_value(json!({"show": false})).unwrap();
    assert!(old.anchor_point && old.gridline_intersection && !old.show);
}

#[test]
fn bad_params_are_refused_and_leave_the_grid_as_it_is() {
    let (mut s, id) = persp_session();
    let (anchor, _) = top_anchor(&s, id);
    let rev = revision(&s);
    for bad in [
        json!({"at": "here"}),
        json!({"at": [1]}),
        json!({"at": [anchor.x, anchor.y], "tolerance": -1}),
        json!({"at": [anchor.x, anchor.y], "tolerance": "wide"}),
        json!({"at": [anchor.x, anchor.y], "clear": true}),
        json!({"clear": "yes"}),
    ] {
        assert!(s.execute("perspective.plane.infer", &bad).is_err(), "{bad}");
    }
    assert_eq!((grid(&s).inferred, revision(&s)), (None, rev));
    // Clearing when nothing is held changes nothing.
    assert_eq!(s.execute("perspective.plane.infer", &json!({"clear": true})).unwrap()["active"], json!(false));
    assert_eq!(revision(&s), rev);
    s.execute("perspective.grid.show", &json!({"visible": false})).unwrap();
    assert!(s.execute("perspective.plane.infer", &json!({"at": [anchor.x, anchor.y]})).is_err(), "hidden grid");
}

/// A session whose ground plane is held at the height of the attached rectangle's top anchor.
fn held_session() -> (Session, NodeId, f64) {
    let (mut s, id) = persp_session();
    let (anchor, height) = top_anchor(&s, id);
    s.execute("perspective.plane.set", &json!({"plane": "ground"})).unwrap();
    let r = s.execute("perspective.plane.infer", &json!({"at": [anchor.x, anchor.y], "tolerance": 0.01})).unwrap();
    assert_eq!(r["source"], json!("anchor"));
    (s, id, height)
}

#[test]
fn attaching_an_object_ends_it_and_the_object_sits_where_the_plane_was_held() {
    let (mut s, _, height) = held_session();
    let r = s.execute("shape.rectangle", &json!({"x": 420, "y": 400, "width": 40, "height": 30})).unwrap();
    let flat = NodeId(r["id"].as_u64().unwrap());
    s.execute("perspective.attach", &json!({"ids": [flat.0]})).unwrap();
    let (plane, depth) = attached(&s, flat).unwrap();
    assert!(plane == Plane::Ground && (depth - height).abs() < 1e-6, "{plane:?} {depth}");
    assert_eq!((grid(&s).inferred, grid(&s).ground_offset), (None, 0.0));
    // An attach of nothing, or to a plane it doesn't hold, leaves it held.
    let (mut s, ..) = held_session();
    s.execute("perspective.attach", &json!({"ids": []})).unwrap();
    assert!(grid(&s).inferred.is_some());
    let r = s.execute("shape.rectangle", &json!({"x": 420, "y": 400, "width": 40, "height": 30})).unwrap();
    let other = NodeId(r["id"].as_u64().unwrap());
    s.execute("perspective.attach", &json!({"ids": [other.0], "plane": "right"})).unwrap();
    assert_eq!(attached(&s, other).map(|a| a.0), Some(Plane::Right));
    assert!(grid(&s).inferred.is_some());
    // A move of an object already in perspective leaves it held; one brought into perspective ends it.
    let (mut s, id, height) = held_session();
    s.execute("perspective.move", &json!({"ids": [id.0], "from": [460, 400], "to": [470, 400]})).unwrap();
    assert!(grid(&s).inferred.is_some());
    let r = s.execute("shape.rectangle", &json!({"x": 420, "y": 400, "width": 40, "height": 30})).unwrap();
    let flat = NodeId(r["id"].as_u64().unwrap());
    s.execute("perspective.move", &json!({"ids": [flat.0], "from": [430, 410], "to": [440, 410]})).unwrap();
    let (plane, depth) = attached(&s, flat).unwrap();
    assert!(plane == Plane::Ground && (depth - height).abs() < 1e-6, "{plane:?} {depth}");
    assert_eq!(grid(&s).inferred, None);
}

#[test]
fn moving_a_plane_hiding_the_grid_and_reopening_the_file_end_it() {
    // A moved plane moves from where it is held and is placed there for good, in one undo step;
    // undoing the move holds it again.
    let (mut s, _, height) = held_session();
    let steps = undo_len(&s);
    s.execute("perspective.plane.move", &json!({"plane": "ground", "by": 10})).unwrap();
    let g = grid(&s);
    assert_eq!((g.inferred, undo_len(&s)), (None, steps + 1));
    assert!((g.ground_offset - (height + 10.0)).abs() < 1e-6, "{}", g.ground_offset);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((grid(&s).inferred.is_some(), grid(&s).ground_offset), (true, 0.0));
    // A refused move leaves it held.
    let (mut s, ..) = held_session();
    assert!(s.execute("perspective.plane.move", &json!({"plane": "ground", "offset": 1, "objects": "all"})).is_err());
    assert!(grid(&s).inferred.is_some());
    // A plane dragged while it is held, then the drag canceled, is held again.
    let (mut s, ..) = held_session();
    s.begin_interaction("Move Plane").unwrap();
    s.preview("perspective.plane.move", &json!({"plane": "ground", "by": 20})).unwrap();
    assert_eq!(grid(&s).inferred, None);
    s.cancel_interaction().unwrap();
    assert!(grid(&s).inferred.is_some());
    // Move Plane to Match Object.
    let (mut s, id, _) = held_session();
    s.execute("perspective.plane.matchObject", &json!({"id": id.0})).unwrap();
    assert_eq!((grid(&s).inferred, grid(&s).plane), (None, Plane::Right));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((grid(&s).inferred.is_some(), grid(&s).plane), (true, Plane::Ground), "undo holds it again");
    // With the plane placed at the object already, matching only puts the held plane back there.
    s.execute("perspective.plane.set", &json!({"plane": "right"})).unwrap();
    assert!(grid(&s).inferred.is_some());
    let steps = undo_len(&s);
    s.execute("perspective.plane.matchObject", &json!({"id": id.0})).unwrap();
    assert_eq!((grid(&s).inferred, grid(&s).offset(Plane::Right), undo_len(&s)), (None, 0.0, steps));
    // Hide Grid.
    let (mut s, ..) = held_session();
    s.execute("perspective.grid.show", &json!({"visible": false})).unwrap();
    assert_eq!(grid(&s).inferred, None);
    // The grid's definition changing leaves it held, and it is not `perspective.grid.set`'s to set.
    let (mut s, ..) = held_session();
    let held = grid(&s).inferred;
    s.execute("perspective.grid.set", &json!({"cell": 31, "inferred": [1, 2, 3]})).unwrap();
    assert_eq!(grid(&s).inferred, held);
    // `perspective.grid.set` hiding the grid or choosing no plane ends it.
    for patch in [json!({"visible": false}), json!({"plane": "none"})] {
        let (mut s, ..) = held_session();
        s.execute("perspective.grid.set", &patch).unwrap();
        assert_eq!(grid(&s).inferred, None, "{patch}");
    }
    // Another preset leaves it held, as other changes to the grid's definition do.
    let (mut preset, ..) = held_session();
    let before = grid(&preset).inferred;
    preset.execute("perspective.grid.preset", &json!({"kind": 3})).unwrap();
    assert_eq!((grid(&preset).kind, grid(&preset).inferred), (3, before));
    // A file saved while a plane is held opens with the plane in its place.
    let saved = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    let back = vectorcraft_format::load(&saved).unwrap();
    assert!(PerspectiveGrid::positioning(&back), "the saved file still holds the point");
    s.add_document(back, None);
    assert_eq!(grid(&s).inferred, None);
    assert_eq!(grid(&s).cell, 31.0);
}
