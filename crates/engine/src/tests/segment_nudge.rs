//! The arrow keys with a curved segment selected bend it: its anchors stay, its handles keep their
//! directions, and the middle of the curve moves toward the arrow.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_doc::{Appearance, Document, Node, NodeId};
use vectorcraft_geom::{Anchor, ParamCurve, PathData, Point, SubPath, Vec2};

use crate::*;

/// A session with one open path of one segment from (100, 300) to (400, 300), with the handles
/// `out` at its start and `in` at its end, and that segment selected with Direct Selection.
fn selected_segment(out: Point, h_in: Point) -> (Session, NodeId) {
    let (p0, p1) = (Point::new(100.0, 300.0), Point::new(400.0, 300.0));
    let (s, id) = session_with(vec![Anchor::with_handles(p0, p0, out), Anchor::with_handles(p1, h_in, p1)]);
    (select(s, id, &[0]), id)
}

/// A session with one open path through `anchors`.
fn session_with(anchors: Vec<Anchor>) -> (Session, NodeId) {
    let mut d = Document::new(1000.0, 1000.0);
    let id = d.alloc_id();
    let node = Node::path(id, PathData::single(SubPath::new(anchors, false)), Appearance::default_art());
    if let Some(ch) = Arc::make_mut(&mut d.layers[0]).children_mut() {
        ch.push(Arc::new(node));
    }
    let mut s = Session::new();
    s.add_document(d, None);
    (s, id)
}

/// `s` with segments `segments` of path `id` selected with Direct Selection.
fn select(mut s: Session, id: NodeId, segments: &[usize]) -> Session {
    let refs: Vec<_> = segments.iter().map(|&seg| [0, seg]).collect();
    s.execute("select.segments", &json!({"id": id.0, "segments": refs})).unwrap();
    s
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn segment(s: &Session, id: NodeId) -> SubPath {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0].clone()
}

/// Each handle as a vector from its anchor: the outgoing one at the start, the incoming one at
/// the end.
fn handles(sp: &SubPath) -> (Vec2, Vec2) {
    (sp.anchors[0].h_out - sp.anchors[0].p, sp.anchors[1].h_in - sp.anchors[1].p)
}

fn middle(sp: &SubPath) -> Point {
    sp.segment(0).eval(0.5)
}

#[test]
fn arrows_bend_a_selected_curved_segment_and_keep_its_anchors() {
    // The handle at the start points up, the one at the end left.
    let (mut s, id) = selected_segment(Point::new(100.0, 150.0), Point::new(300.0, 300.0));
    let step = s.prefs.keyboard_increment;
    let before = segment(&s, id);
    let (up, left) = handles(&before);

    // Up: the upward handle grows, the one across the arrow stays.
    s.execute("object.nudge", &json!({"dx": 0, "dy": -1})).unwrap();
    let after = segment(&s, id);
    let (up2, left2) = handles(&after);
    assert_eq!((after.anchors[0].p, after.anchors[1].p), (before.anchors[0].p, before.anchors[1].p), "the anchors stay");
    assert!(up2.hypot() > up.hypot() && up2.cross(up).abs() < 1e-9 && up2.dot(up) > 0.0, "the upward handle grows in its direction: {up2:?}");
    assert_eq!(left2, left, "the handle across the arrow stays");
    assert!((middle(&after) - middle(&before) - Vec2::new(0.0, -step)).hypot() < 1e-9, "the middle moves one step up");

    // Right: the handle pointing left shrinks, the upward one stays.
    s.execute("object.nudge", &json!({"dx": 1, "dy": 0})).unwrap();
    let last = segment(&s, id);
    let (up3, left3) = handles(&last);
    assert_eq!(up3, up2, "the handle across the arrow stays");
    assert!(left3.hypot() < left2.hypot() && left3.cross(left2).abs() < 1e-9 && left3.dot(left2) > 0.0, "the leftward handle shrinks: {left3:?}");
    assert!((middle(&last) - middle(&after) - Vec2::new(step, 0.0)).hypot() < 1e-9, "the middle moves one step right");
}

#[test]
fn arrows_move_a_selected_straight_segment_whole() {
    let (mut s, id) = selected_segment(Point::new(100.0, 300.0), Point::new(400.0, 300.0));
    let step = s.prefs.keyboard_increment;
    let before = segment(&s, id);
    s.execute("object.nudge", &json!({"dx": 0, "dy": 1})).unwrap();
    let after = segment(&s, id);
    for (a, b) in before.anchors.iter().zip(&after.anchors) {
        assert_eq!(b.p - a.p, Vec2::new(0.0, step));
    }
}

#[test]
fn a_selected_straight_segment_beside_a_curved_one_moves_whole() {
    let (p0, p1, p2) = (Point::new(100.0, 300.0), Point::new(400.0, 300.0), Point::new(600.0, 300.0));
    let (s, id) = session_with(vec![
        Anchor::with_handles(p0, p0, Point::new(100.0, 150.0)),
        Anchor::with_handles(p1, Point::new(300.0, 300.0), p1),
        Anchor::corner(p2),
    ]);
    let mut s = select(s, id, &[0, 1]);
    let step = s.prefs.keyboard_increment;
    let before = segment(&s, id);
    s.execute("object.nudge", &json!({"dx": 0, "dy": -1})).unwrap();
    let after = segment(&s, id);
    assert_eq!(after.anchors[0].p, before.anchors[0].p, "the curved segment's other anchor stays");
    for i in [1, 2] {
        assert_eq!(after.anchors[i].p - before.anchors[i].p, Vec2::new(0.0, -step), "the straight segment's anchor {i} moves");
    }
}

#[test]
fn a_segment_whose_handles_lie_across_the_arrow_records_no_undo_step() {
    // Both handles point up: ← and → bend nothing.
    let (mut s, id) = selected_segment(Point::new(100.0, 150.0), Point::new(400.0, 150.0));
    let (before, steps) = (segment(&s, id), undo_steps(&s));
    s.execute("object.nudge", &json!({"dx": 1, "dy": 0})).unwrap();
    assert_eq!(segment(&s, id), before);
    assert_eq!(undo_steps(&s), steps);
}

#[test]
fn a_handle_bent_through_a_smooth_anchor_leaves_the_next_segment_as_it_is() {
    // A smooth anchor between the selected segment, whose handle there is short and points up,
    // and the next one.
    let (p0, p1, p2) = (Point::new(100.0, 300.0), Point::new(400.0, 300.0), Point::new(700.0, 300.0));
    let smooth = Anchor::with_handles(p0, Point::new(100.0, 310.0), Point::new(100.0, 299.5));
    let (s, id) = session_with(vec![
        Anchor::corner(Point::new(-200.0, 300.0)),
        smooth,
        Anchor::with_handles(p1, Point::new(300.0, 300.0), p1),
        Anchor::corner(p2),
    ]);
    assert_eq!(smooth.kind, vectorcraft_geom::AnchorKind::Smooth);
    let mut s = select(s, id, &[1]);
    let h_in = segment(&s, id).anchors[1].h_in;
    // ↓ shrinks the upward handle through its anchor.
    s.execute("object.nudge", &json!({"dx": 0, "dy": 1})).unwrap();
    let after = segment(&s, id).anchors[1];
    assert!(after.h_out.y > p0.y, "the handle comes out pointing down: {:?}", after.h_out);
    assert_eq!(after.h_in, h_in, "the next segment's handle stays");
}

#[test]
fn a_retracted_handle_the_arrow_leaves_keeps_its_smooth_anchor_smooth() {
    // A smooth anchor whose outgoing handle is retracted, so it points toward the other handle's
    // end, level with it and across ↓; only the other handle, pointing up, changes.
    let (p0, p1) = (Point::new(100.0, 300.0), Point::new(200.0, 400.0));
    let smooth = Anchor { p: p0, h_in: Point::new(0.0, 300.0), h_out: p0, kind: vectorcraft_geom::AnchorKind::Smooth };
    let (s, id) = session_with(vec![Anchor::corner(Point::new(-200.0, 300.0)), smooth, Anchor::with_handles(p1, Point::new(200.0, 300.0), p1)]);
    let mut s = select(s, id, &[1]);
    let before = segment(&s, id);
    s.execute("object.nudge", &json!({"dx": 0, "dy": 1})).unwrap();
    let after = segment(&s, id);
    assert_ne!(after.anchors[2].h_in, before.anchors[2].h_in, "the upward handle changes");
    assert_eq!(after.anchors[1], before.anchors[1], "the smooth anchor stays as it was");
}
