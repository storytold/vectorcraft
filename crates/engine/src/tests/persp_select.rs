//! Perspective Selection tool: scaling in perspective, Alt-drag copies, moving perpendicular to the
//! plane (5), Transform Again and arrow nudges in perspective.

use serde_json::json;
use vectorcraft_geom::{Point, Rect, Vec2};
use vectorcraft_tools::distort::perspective::{self as persp, PerspectiveGrid, Plane};
use vectorcraft_tools::{Cursor, Mods, PointerEvent, PointerKind, ToolKey};

use crate::*;

/// An 800 × 600 document with a two-point grid and a rectangle attached to the right plane.
pub(crate) fn persp_session() -> (Session, NodeId) {
    let mut s = Session::new();
    // Separately created fixtures must compare equal across clock ticks.
    s.execute("file.new", &json!({"width": 800, "height": 600, "created": null})).unwrap();
    s.execute("perspective.grid.preset", &json!({"kind": 2})).unwrap();
    // Snap to Grid (on by default) would land moved edges on gridlines: these tests measure exact moves.
    s.execute("perspective.grid.snap", &json!({"on": false})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 450, "y": 380, "width": 60, "height": 60})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("perspective.attach", &json!({"ids": [id.0], "plane": "right"})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    (s, id)
}

pub(crate) fn grid(s: &Session) -> PerspectiveGrid {
    PerspectiveGrid::from_doc(&s.doc().unwrap().doc).unwrap()
}

pub(crate) fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// Where `id` is attached (plane, depth).
pub(crate) fn attached(s: &Session, id: NodeId) -> Option<(Plane, f64)> {
    grid(s).attachment_of(&s.doc().unwrap().doc, id)
}

/// The anchors of path `id` in the plane coordinates of `plane` at `depth`.
pub(crate) fn plane_pts(s: &Session, id: NodeId, plane: Plane, depth: f64) -> Vec<Point> {
    let g = grid(s);
    let hi = g.homography_at(plane, depth).unwrap().inverse().unwrap();
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().anchors().map(|(_, _, a)| hi.apply(a.p).unwrap()).collect()
}

pub(crate) fn bounds_of(pts: &[Point]) -> Rect {
    pts.iter().skip(1).fold(Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p))
}

/// Pointer events with the active tool: down at the first point, drags, up at the last.
pub(crate) fn drag(s: &mut Session, pts: &[Point], mods: Mods) {
    let v = ViewInfo::default();
    for (i, p) in pts.iter().enumerate() {
        let kind = if i == 0 {
            PointerKind::Down
        } else if i == pts.len() - 1 {
            PointerKind::Up
        } else {
            PointerKind::Drag
        };
        s.pointer(&PointerEvent::new(kind, p.x, p.y).with_mods(mods), v).unwrap();
    }
}

#[test]
fn handles_scale_in_perspective_about_the_opposite_handle() {
    let (mut s, id) = persp_session();
    s.select_tool("perspectiveSelection", ViewInfo::default()).unwrap();
    let g = grid(&s);
    let before = bounds_of(&plane_pts(&s, id, Plane::Right, 0.0));
    // The box's handles are drawn in perspective: the far corner of the plane-space box.
    let h = g.homography(Plane::Right).unwrap();
    let corner = h.apply(Point::new(before.x1, before.y1)).unwrap();
    let anchors = s.overlays(ViewInfo::default()).into_iter().filter(|o| matches!(o, vectorcraft_tools::Overlay::Anchor { .. })).count();
    assert!(anchors >= 8, "{anchors} handles");
    // Drag that handle to twice the size in plane space.
    let target = h.apply(Point::new(before.x0 + 2.0 * before.width(), before.y0 + 2.0 * before.height())).unwrap();
    let n = undo_len(&s);
    drag(&mut s, &[corner, target, target], Mods::default());
    assert_eq!(undo_len(&s), n + 1, "one undo step");
    assert_eq!(s.journal.last().unwrap().0, "perspective.transform");
    let after = bounds_of(&plane_pts(&s, id, Plane::Right, 0.0));
    assert!((after.x0 - before.x0).abs() < 1e-6 && (after.y0 - before.y0).abs() < 1e-6, "the opposite corner stays: {before:?} → {after:?}");
    assert!((after.width() - 2.0 * before.width()).abs() < 1e-6 && (after.height() - 2.0 * before.height()).abs() < 1e-6, "{after:?}");
    // Still a rectangle on the plane.
    for q in plane_pts(&s, id, Plane::Right, 0.0) {
        assert!(((q.x - after.x0).abs() < 1e-6 || (q.x - after.x1).abs() < 1e-6) && ((q.y - after.y0).abs() < 1e-6 || (q.y - after.y1).abs() < 1e-6));
    }
    assert_eq!(attached(&s, id), Some((Plane::Right, 0.0)));
}

#[test]
fn alt_drag_copies_and_the_copy_stays_attached() {
    let (mut s, id) = persp_session();
    s.select_tool("perspectiveSelection", ViewInfo::default()).unwrap();
    let before = plane_pts(&s, id, Plane::Right, 0.0);
    let c = bounds_of(&before).center();
    let h = grid(&s).homography(Plane::Right).unwrap();
    let (from, to) = (h.apply(c).unwrap(), h.apply(c + Vec2::new(80.0, 0.0)).unwrap());
    let n = undo_len(&s);
    drag(&mut s, &[from, to, to], Mods { alt: true, ..Default::default() });
    assert_eq!(undo_len(&s), n + 1);
    let copy = *s.doc().unwrap().selection.objects.first().unwrap();
    assert_ne!(copy, id, "the copy is selected");
    assert_eq!(plane_pts(&s, id, Plane::Right, 0.0), before, "the original stays");
    assert_eq!(attached(&s, copy), Some((Plane::Right, 0.0)), "the copy is attached");
    for (a, b) in before.iter().zip(plane_pts(&s, copy, Plane::Right, 0.0)) {
        assert!((b - *a - Vec2::new(80.0, 0.0)).hypot() < 1e-6);
    }
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(copy).is_none());
}

#[test]
fn shift_drag_moves_on_a_plane_axis() {
    let (mut s, id) = persp_session();
    s.select_tool("perspectiveSelection", ViewInfo::default()).unwrap();
    let before = plane_pts(&s, id, Plane::Right, 0.0);
    let h = grid(&s).homography(Plane::Right).unwrap();
    let start = bounds_of(&before).center();
    let (from, to) = (h.apply(start).unwrap(), h.apply(start + Vec2::new(80.0, 20.0)).unwrap());
    let n = undo_len(&s);
    drag(&mut s, &[from, to, to], Mods { shift: true, ..Default::default() });
    assert_eq!(undo_len(&s), n + 1);
    for (a, b) in before.iter().zip(plane_pts(&s, id, Plane::Right, 0.0)) {
        assert!((b - *a - Vec2::new(80.0, 0.0)).hypot() < 1e-6, "shift move: {a:?} → {b:?}");
    }
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(plane_pts(&s, id, Plane::Right, 0.0), before);
}

/// An attached rectangle with a horizontal edge close enough to a gridline to be pulled by snapping.
fn near_gridline_session() -> (Session, NodeId) {
    let (mut s, id) = persp_session();
    let g = grid(&s);
    let b = bounds_of(&plane_pts(&s, id, Plane::Right, 0.0));
    let dy = (b.y0 / g.cell).round() * g.cell + g.cell * 0.1 - b.y0;
    let h = g.homography(Plane::Right).unwrap();
    let from = h.apply(b.center()).unwrap();
    let to = h.apply(b.center() + Vec2::new(0.0, dy)).unwrap();
    s.execute("perspective.move", &json!({"ids": [id.0], "from": [from.x, from.y], "to": [to.x, to.y], "snap": false})).unwrap();
    s.execute("perspective.grid.snap", &json!({"on": true})).unwrap();
    (s, id)
}

fn shift_move_near_gridline(diagonal: bool, copy: bool) {
    let (mut s, id) = near_gridline_session();
    s.select_tool("perspectiveSelection", ViewInfo::default()).unwrap();
    let before = plane_pts(&s, id, Plane::Right, 0.0);
    let h = grid(&s).homography(Plane::Right).unwrap();
    let start = bounds_of(&before).center();
    let raw = if diagonal { Vec2::new(40.0, 50.0) } else { Vec2::new(80.0, 20.0) };
    let from = h.apply(start).unwrap();
    let to = h.apply(start + raw).unwrap();
    drag(&mut s, &[from, to, to], Mods { shift: true, alt: copy, ..Default::default() });
    let target = if copy { *s.doc().unwrap().selection.objects.first().unwrap() } else { id };
    assert!(!copy || target != id);
    let after = plane_pts(&s, target, Plane::Right, 0.0);
    for (a, b) in before.iter().zip(after) {
        let d = b - *a;
        if diagonal {
            assert!((d.x - d.y).abs() < 1e-6, "diagonal move left its line: {d:?}");
        } else {
            assert!(d.y.abs() < 1e-6, "axis move left its line: {d:?}");
        }
    }
    if copy {
        assert_eq!(plane_pts(&s, id, Plane::Right, 0.0), before);
    }
}

#[test]
fn shift_axis_move_stays_on_axis_with_grid_snap() {
    shift_move_near_gridline(false, false);
}

#[test]
fn shift_axis_copy_stays_on_axis_with_grid_snap() {
    shift_move_near_gridline(false, true);
}

#[test]
fn shift_diagonal_move_stays_on_diagonal_with_grid_snap() {
    shift_move_near_gridline(true, false);
}

#[test]
fn shift_diagonal_copy_stays_on_diagonal_with_grid_snap() {
    shift_move_near_gridline(true, true);
}

#[test]
fn constrained_move_command_stays_on_axis_with_grid_snap() {
    let (mut s, id) = near_gridline_session();
    let before = plane_pts(&s, id, Plane::Right, 0.0);
    let h = grid(&s).homography(Plane::Right).unwrap();
    let start = bounds_of(&before).center();
    let from = h.apply(start).unwrap();
    let to = h.apply(start + Vec2::new(80.0, 0.0)).unwrap();
    s.execute("perspective.move", &json!({"ids": [id.0], "from": [from.x, from.y], "to": [to.x, to.y], "constrain": true})).unwrap();
    for (a, b) in before.iter().zip(plane_pts(&s, id, Plane::Right, 0.0)) {
        assert!((b.y - a.y).abs() < 1e-6, "constrained command left axis: {a:?} to {b:?}");
    }
}

#[test]
fn diagonal_shift_snap_lands_an_edge_on_a_gridline() {
    let (mut s, id) = near_gridline_session();
    let g = grid(&s);
    let before = plane_pts(&s, id, Plane::Right, 0.0);
    let b = bounds_of(&before);
    let (raw, correction) = (1..200)
        .map(|n| {
            let raw = Vec2::new(n as f64 * g.cell * 0.05, n as f64 * g.cell * 0.05);
            (raw, g.snap_offset(b, raw) - raw)
        })
        .find(|(_, fix)| fix.x.abs() < 1e-9 && fix.y.abs() > 1e-6)
        .unwrap();
    let h = g.homography(Plane::Right).unwrap();
    let start = b.center();
    let from = h.apply(start).unwrap();
    let to = h.apply(start + raw).unwrap();
    s.execute("perspective.move", &json!({"ids": [id.0], "from": [from.x, from.y], "to": [to.x, to.y], "constrain": true})).unwrap();
    let after = plane_pts(&s, id, Plane::Right, 0.0);
    let delta = after[0] - before[0];
    assert!((delta.x - delta.y).abs() < 1e-6, "{delta:?}");
    let edge = [b.y0 + delta.y, b.y1 + delta.y];
    assert!(edge.iter().any(|y| ((y / g.cell).round() * g.cell - y).abs() < 1e-6), "{edge:?}, correction {correction:?}");
}

#[test]
fn constrained_move_uses_each_objects_plane() {
    let (mut s, right) = persp_session();
    let left_result = s.execute("shape.rectangle", &json!({"x": 240, "y": 380, "width": 60, "height": 60})).unwrap();
    let left = NodeId(left_result["id"].as_u64().unwrap());
    s.execute("perspective.attach", &json!({"ids": [left.0], "plane": "left"})).unwrap();
    let right_before = plane_pts(&s, right, Plane::Right, 0.0);
    let left_before = plane_pts(&s, left, Plane::Left, 0.0);
    let from = grid(&s).homography(Plane::Right).unwrap().apply(bounds_of(&right_before).center()).unwrap();
    let to = from + Vec2::new(70.0, 30.0);
    let g = grid(&s);
    for plane in [Plane::Right, Plane::Left] {
        let raw = g.plane_delta(plane, 0.0, from, to).unwrap();
        assert!(raw.x.abs() > 1e-3 && raw.y.abs() > 1e-3 && (raw.x.abs() - raw.y.abs()).abs() > 1e-3, "raw {plane:?}: {raw:?}");
    }
    s.execute("perspective.move", &json!({"ids": [right.0, left.0], "from": [from.x, from.y], "to": [to.x, to.y], "constrain": true, "snap": false}))
        .unwrap();
    for (id, plane, before) in [(right, Plane::Right, right_before), (left, Plane::Left, left_before)] {
        let after = plane_pts(&s, id, plane, 0.0);
        let delta = after[0] - before[0];
        assert!(delta.x.abs() < 1e-6 || delta.y.abs() < 1e-6 || (delta.x.abs() - delta.y.abs()).abs() < 1e-6, "{plane:?}: {delta:?}");
    }
}

#[test]
fn constrained_move_rejects_non_boolean_switch() {
    let (mut s, id) = near_gridline_session();
    for direction in [json!([0.0, 0.0]), json!([1.0]), json!(1), json!("true")] {
        let error = s.execute("perspective.move", &json!({"ids": [id.0], "from": [1.0, 1.0], "to": [2.0, 2.0], "constrain": direction})).unwrap_err();
        assert!(error.to_string().contains("`constrain` must be true or false"), "{error}");
    }
}

#[test]
fn shift_drag_keeps_its_displacement_after_a_plane_key() {
    let (mut s, id) = persp_session();
    let v = ViewInfo::default();
    s.select_tool("perspectiveSelection", v).unwrap();
    let before = plane_pts(&s, id, Plane::Right, 0.0);
    let h = grid(&s).homography(Plane::Right).unwrap();
    let start = bounds_of(&before).center();
    let (from, to) = (h.apply(start).unwrap(), h.apply(start + Vec2::new(80.0, 20.0)).unwrap());
    let shift = Mods { shift: true, ..Default::default() };
    s.pointer(&PointerEvent::new(PointerKind::Down, from.x, from.y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, to.x, to.y), v).unwrap();
    s.tool_key(ToolKey::Digit(2), shift, v).unwrap();
    assert_eq!(grid(&s).plane, Plane::Ground);
    s.pointer(&PointerEvent::new(PointerKind::Up, to.x, to.y).with_mods(shift), v).unwrap();
    for (a, b) in before.iter().zip(plane_pts(&s, id, Plane::Right, 0.0)) {
        assert!((b - *a - Vec2::new(80.0, 0.0)).hypot() < 1e-6, "plane key: {a:?} → {b:?}");
    }
}

#[test]
fn loose_art_move_survives_clearing_the_active_plane() {
    for (drag_after_key, switch_before_drag) in [(false, false), (true, false), (false, true)] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 600, "created": null})).unwrap();
        s.execute("perspective.grid.preset", &json!({"kind": 2})).unwrap();
        s.execute("perspective.grid.snap", &json!({"on": false})).unwrap();
        s.execute("perspective.plane.set", &json!({"plane": "right"})).unwrap();
        let result = s.execute("shape.rectangle", &json!({"x": 450, "y": 380, "width": 60, "height": 60})).unwrap();
        let id = NodeId(result["id"].as_u64().unwrap());
        assert_eq!(attached(&s, id), None);
        let before = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
        let (from, to) = (before.center(), before.center() + Vec2::new(30.0, 15.0));
        let v = ViewInfo::default();
        s.select_tool("perspectiveSelection", v).unwrap();
        let undo_before = undo_len(&s);
        s.pointer(&PointerEvent::new(PointerKind::Down, from.x, from.y), v).unwrap();
        if switch_before_drag {
            s.tool_key(ToolKey::Digit(2), Mods::default(), v).unwrap();
        }
        s.pointer(&PointerEvent::new(PointerKind::Drag, to.x, to.y), v).unwrap();
        s.tool_key(ToolKey::Digit(4), Mods::default(), v).unwrap();
        assert_eq!(grid(&s).plane, Plane::None);
        if drag_after_key {
            s.pointer(&PointerEvent::new(PointerKind::Drag, to.x + 10.0, to.y), v).unwrap();
        }
        s.pointer(&PointerEvent::new(PointerKind::Up, to.x, to.y), v).unwrap();
        assert_eq!(attached(&s, id), Some((if switch_before_drag { Plane::Ground } else { Plane::Right }, 0.0)));
        assert_ne!(s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap(), before);
        assert_eq!(undo_len(&s), undo_before + 1);
    }
}

#[test]
fn cmd_alt_shift_drag_copies_on_a_plane_axis() {
    let (mut s, id) = persp_session();
    let v = ViewInfo::default();
    s.select_tool("directSelection", v).unwrap();
    s.select_tool("perspectiveSelection", v).unwrap();
    let before = plane_pts(&s, id, Plane::Right, 0.0);
    let count = s.doc().unwrap().doc.node_count();
    let h = grid(&s).homography(Plane::Right).unwrap();
    let start = bounds_of(&before).center();
    let (from, to) = (h.apply(start).unwrap(), h.apply(start + Vec2::new(80.0, 20.0)).unwrap());
    let n = undo_len(&s);
    drag(&mut s, &[from, to, to], Mods { cmd: true, alt: true, shift: true, ..Default::default() });
    assert_eq!(undo_len(&s), n + 1);
    assert_eq!(s.doc().unwrap().doc.node_count(), count + 1);
    let copy = *s.doc().unwrap().selection.objects.first().unwrap();
    assert_ne!(copy, id);
    assert_eq!(s.doc().unwrap().selection.objects, vec![copy]);
    assert_eq!(plane_pts(&s, id, Plane::Right, 0.0), before);
    assert_eq!(attached(&s, copy), Some((Plane::Right, 0.0)));
    for (a, b) in before.iter().zip(plane_pts(&s, copy, Plane::Right, 0.0)) {
        assert!((b - *a - Vec2::new(80.0, 0.0)).hypot() < 1e-6, "shift copy: {a:?} → {b:?}");
    }
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node_count(), count);
    assert_eq!(plane_pts(&s, id, Plane::Right, 0.0), before);
    assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
}

#[test]
fn cmd_alt_drag_copies_on_the_perspective_plane() {
    let (mut s, id) = persp_session();
    let v = ViewInfo::default();
    s.select_tool("directSelection", v).unwrap();
    s.select_tool("perspectiveSelection", v).unwrap();
    let before = plane_pts(&s, id, Plane::Right, 0.0);
    let count = s.doc().unwrap().doc.node_count();
    let c = bounds_of(&before).center();
    let h = grid(&s).homography(Plane::Right).unwrap();
    let (from, to) = (h.apply(c).unwrap(), h.apply(c + Vec2::new(80.0, 0.0)).unwrap());
    let n = undo_len(&s);
    drag(&mut s, &[from, to, to], Mods { cmd: true, alt: true, ..Default::default() });
    assert_eq!(undo_len(&s), n + 1, "one undo step");
    assert_eq!(s.doc().unwrap().doc.node_count(), count + 1, "one copy");
    assert_eq!(s.doc().unwrap().selection.objects.len(), 1);
    let copy = s.doc().unwrap().selection.objects[0];
    assert_ne!(copy, id, "the copy is selected");
    assert_eq!(plane_pts(&s, id, Plane::Right, 0.0), before, "the original stays");
    assert_eq!(attached(&s, copy), Some((Plane::Right, 0.0)), "the copy is attached");
    for (a, b) in before.iter().zip(plane_pts(&s, copy, Plane::Right, 0.0)) {
        assert!((b - *a - Vec2::new(80.0, 0.0)).hypot() < 1e-6);
    }
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node_count(), count);
    assert_eq!(plane_pts(&s, id, Plane::Right, 0.0), before);
    assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
}

#[test]
fn cmd_drag_still_lends_selection_from_perspective_selection() {
    let (mut s, id) = persp_session();
    let v = ViewInfo::default();
    s.select_tool("directSelection", v).unwrap();
    s.select_tool("perspectiveSelection", v).unwrap();
    let p = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().center();
    let cmd = Mods { cmd: true, ..Default::default() };
    s.pointer(&PointerEvent::new(PointerKind::Down, p.x, p.y).with_mods(cmd), v).unwrap();
    assert_eq!(s.tool_id(), "directSelection");
    s.pointer(&PointerEvent::new(PointerKind::Up, p.x, p.y).with_mods(cmd), v).unwrap();
    assert_eq!(s.tool_id(), "perspectiveSelection");
    let cmd_alt = Mods { cmd: true, alt: true, ..Default::default() };
    s.pointer(&PointerEvent::new(PointerKind::Down, p.x, p.y).with_mods(cmd_alt), v).unwrap();
    assert_eq!(s.tool_id(), "perspectiveSelection");
    s.pointer(&PointerEvent::new(PointerKind::Up, p.x, p.y).with_mods(cmd_alt), v).unwrap();
}

#[test]
fn cmd_alt_cursor_stays_with_perspective_selection() {
    let (mut s, id) = persp_session();
    let v = ViewInfo::default();
    s.select_tool("directSelection", v).unwrap();
    s.select_tool("perspectiveSelection", v).unwrap();
    let p = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().center();
    let cmd_alt = Mods { cmd: true, alt: true, ..Default::default() };
    assert_eq!(s.cursor(p, cmd_alt, v), Cursor::Arrow);
    assert_eq!(s.tool_id(), "perspectiveSelection");
}

#[test]
fn pressing_5_while_dragging_moves_perpendicular_to_the_plane() {
    let (mut s, id) = persp_session();
    s.select_tool("perspectiveSelection", ViewInfo::default()).unwrap();
    let before = plane_pts(&s, id, Plane::Right, 0.0);
    let c = bounds_of(&before).center();
    let g = grid(&s);
    let from = g.homography(Plane::Right).unwrap().apply(c).unwrap();
    // The point 40 pt in front of the plane (along its normal, toward the viewer's left wall side).
    let to = g.homography_at(Plane::Right, 40.0).unwrap().apply(c).unwrap();
    let v = ViewInfo::default();
    let n = undo_len(&s);
    s.pointer(&PointerEvent::new(PointerKind::Down, from.x, from.y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, to.x, to.y), v).unwrap();
    s.tool_key(ToolKey::Digit(5), Mods::default(), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, to.x, to.y), v).unwrap();
    assert_eq!(undo_len(&s), n + 1);
    let (plane, depth) = attached(&s, id).unwrap();
    assert_eq!(plane, Plane::Right);
    assert!((depth - 40.0).abs() < 1e-6, "{depth}");
    // Same place within the plane, on the parallel plane 40 pt along the normal.
    for (a, b) in before.iter().zip(plane_pts(&s, id, Plane::Right, 40.0)) {
        assert!((*a - b).hypot() < 1e-6, "{a:?} {b:?}");
    }
    // Pressing 5 again goes back to moving within the plane.
    let c2 = g.homography_at(Plane::Right, 40.0).unwrap().apply(c).unwrap();
    let to2 = g.homography_at(Plane::Right, 40.0).unwrap().apply(c + Vec2::new(30.0, 0.0)).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, c2.x, c2.y), v).unwrap();
    s.tool_key(ToolKey::Digit(5), Mods::default(), v).unwrap();
    s.tool_key(ToolKey::Digit(5), Mods::default(), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, to2.x, to2.y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, to2.x, to2.y), v).unwrap();
    assert!((attached(&s, id).unwrap().1 - 40.0).abs() < 1e-6);
    for (a, b) in before.iter().zip(plane_pts(&s, id, Plane::Right, 40.0)) {
        assert!((b - *a - Vec2::new(30.0, 0.0)).hypot() < 1e-6);
    }
}

#[test]
fn transform_again_repeats_the_last_perspective_move() {
    let (mut s, id) = persp_session();
    s.select_tool("perspectiveSelection", ViewInfo::default()).unwrap();
    let before = plane_pts(&s, id, Plane::Right, 0.0);
    let c = bounds_of(&before).center();
    let h = grid(&s).homography(Plane::Right).unwrap();
    let (from, to) = (h.apply(c).unwrap(), h.apply(c + Vec2::new(25.0, 10.0)).unwrap());
    drag(&mut s, &[from, to, to], Mods::default());
    // Twice the plane-space move, not twice the page move (which foreshortening would distort).
    s.execute("object.transformAgain", &json!({})).unwrap();
    for (a, b) in before.iter().zip(plane_pts(&s, id, Plane::Right, 0.0)) {
        assert!((b - *a - Vec2::new(50.0, 20.0)).hypot() < 1e-6, "{a:?} → {b:?}");
    }
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Transform in Perspective");
    // An ordinary move takes over Transform Again.
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    let p0 = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
    s.execute("object.transformAgain", &json!({})).unwrap();
    let p1 = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
    assert!((p1.x0 - p0.x0 - 5.0).abs() < 1e-9 && (p1.y0 - p0.y0).abs() < 1e-9);
    // A cancelled drag records nothing.
    s.execute("perspective.transform", &json!({"matrix": [1, 0, 0, 1, 7, 0]})).unwrap();
    let v = ViewInfo::default();
    let c = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().center();
    s.pointer(&PointerEvent::new(PointerKind::Down, c.x, c.y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, c.x + 20.0, c.y), v).unwrap();
    assert!(s.in_interaction());
    s.cancel_interaction().unwrap();
    assert_eq!(s.doc().unwrap().last_perspective.as_ref().unwrap()["matrix"][4], json!(7.0));
}

#[test]
fn transform_again_repeats_a_perspective_scale_and_copies() {
    let (mut s, id) = persp_session();
    let before = bounds_of(&plane_pts(&s, id, Plane::Right, 0.0));
    let r = s.execute("perspective.transform", &json!({"matrix": [1, 0, 0, 1, 100, 0], "copy": true})).unwrap();
    let copy = NodeId(r["ids"][0].as_u64().unwrap());
    s.execute("object.transformAgain", &json!({})).unwrap();
    let third = *s.doc().unwrap().selection.objects.first().unwrap();
    assert!(third != copy && third != id);
    let b = bounds_of(&plane_pts(&s, third, Plane::Right, 0.0));
    assert!((b.x0 - before.x0 - 200.0).abs() < 1e-6, "{before:?} {b:?}");
    assert_eq!(attached(&s, third), Some((Plane::Right, 0.0)));
    assert!(s.execute("perspective.transform", &json!({"matrix": [0, 0, 0, 0, 0, 0]})).is_err());
    assert!(s.execute("perspective.transform", &json!({"matrix": [1, 0, 0, 1, 0, 0], "depth": f64::MAX})).is_err());
}

#[test]
fn arrow_keys_nudge_in_perspective() {
    let (mut s, id) = persp_session();
    let v = ViewInfo::default();
    s.select_tool("perspectiveSelection", v).unwrap();
    assert!(s.tool_claims_key(ToolKey::Right, v), "the tool takes the arrows for a selection in perspective");
    let before = plane_pts(&s, id, Plane::Right, 0.0);
    let n = undo_len(&s);
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    assert_eq!(undo_len(&s), n + 1);
    let after = plane_pts(&s, id, Plane::Right, 0.0);
    let d = after[0] - before[0];
    for (a, b) in before.iter().zip(&after) {
        assert!((*b - *a - d).hypot() < 1e-6, "moves within the plane");
    }
    // The page step at the centre is the keyboard increment.
    let h = grid(&s).homography(Plane::Right).unwrap();
    let c = bounds_of(&before).center();
    let step = h.apply(c + d).unwrap() - h.apply(c).unwrap();
    assert!((step - Vec2::new(s.prefs.keyboard_increment, 0.0)).hypot() < 1e-6, "{step:?}");
    // Alt copies; Shift takes ten steps.
    s.tool_key(ToolKey::Left, Mods { alt: true, shift: true, ..Default::default() }, v).unwrap();
    let copy = *s.doc().unwrap().selection.objects.first().unwrap();
    assert_ne!(copy, id);
    assert_eq!(attached(&s, copy).map(|a| a.0), Some(Plane::Right));
    // Nothing attached: the arrows nudge as usual.
    s.execute("perspective.release", &json!({"ids": [id.0, copy.0]})).unwrap();
    assert!(!s.tool_claims_key(ToolKey::Right, v));
    assert!(s.execute("perspective.nudge", &json!({"dx": 1, "dy": 0})).is_err());
    assert!(s.execute("perspective.nudge", &json!({"dx": 0, "dy": 0})).is_err());
}

#[test]
fn selecting_an_attached_object_switches_to_its_plane() {
    let (mut s, id) = persp_session();
    s.execute("perspective.plane.set", &json!({"plane": "left"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    s.select_tool("perspectiveSelection", ViewInfo::default()).unwrap();
    let c = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().center();
    drag(&mut s, &[c, c], Mods::default());
    assert_eq!(grid(&s).plane, Plane::Right);
    assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
    // A click is no edit.
    assert_eq!(persp::attachment(s.doc().unwrap().doc.node(id).unwrap()), Some((Plane::Right, 0.0)));
}

#[test]
fn attachments_round_trip_through_native_files() {
    let (mut s, id) = persp_session();
    s.execute("perspective.transform", &json!({"matrix": [1, 0, 0, 1, 0, 0], "depth": 12.5})).unwrap();
    let doc = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(persp::attachment(doc.node(id).unwrap()), Some((Plane::Right, 12.5)));
    assert_eq!(*doc.node(id).unwrap(), *s.doc().unwrap().doc.node(id).unwrap());
    // Objects outside perspective write nothing for it.
    let v = serde_json::to_value(vectorcraft_doc::Node::new(NodeId(1), vectorcraft_doc::NodeKind::Group { children: vec![], clip: false })).unwrap();
    assert!(v.get("perspective").is_none());
}
