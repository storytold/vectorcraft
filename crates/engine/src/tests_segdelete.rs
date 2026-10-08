//! Deleting segments picked with the Direct Selection tool: just those segments go (a closed path
//! opens there, an open one splits), live shapes become plain paths, and undo restores them.

use serde_json::{Value, json};

use super::*;
use vectorcraft_doc::{NodeId, NodeKind};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 300, "height": 300}));
    s
}

/// Each subpath's anchor points and whether it is closed.
fn shape(s: &Session, id: u64) -> Vec<(Vec<(f64, f64)>, bool)> {
    let path = s.doc().unwrap().doc.node(NodeId(id)).unwrap().path_data().unwrap().clone();
    path.subpaths.iter().map(|sp| (sp.anchors.iter().map(|a| (a.p.x.round(), a.p.y.round())).collect(), sp.closed)).collect()
}

fn pick_segment(s: &mut Session, id: u64, si: usize, a0: usize, a1: usize, mode: &str) {
    run(s, "select.anchors", json!({"id": id, "anchors": [[si, a0], [si, a1]], "segments": [[si, a0]], "mode": mode}));
}

#[test]
fn deleting_a_rectangle_edge_opens_it_there_and_undo_restores_it() {
    let mut s = session();
    let id = run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 50}))["id"].as_u64().unwrap();
    let before = shape(&s, id);
    // The top edge: anchors 0 (top-left) and 1 (top-right).
    pick_segment(&mut s, id, 0, 0, 1, "set");
    run(&mut s, "edit.clear", json!({}));
    assert_eq!(shape(&s, id), [(vec![(110.0, 10.0), (110.0, 60.0), (10.0, 60.0), (10.0, 10.0)], false)]);
    // No longer a live rectangle.
    let n = s.doc().unwrap().doc.node(NodeId(id)).unwrap().clone();
    assert!(matches!(n.kind, NodeKind::Path { live: None, .. }));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(shape(&s, id), before);
}

#[test]
fn two_segments_split_the_path_and_the_last_one_removes_it() {
    let mut s = session();
    let id = run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 50}))["id"].as_u64().unwrap();
    // Top and bottom edges: two open pieces, the right and the left sides.
    pick_segment(&mut s, id, 0, 0, 1, "set");
    run(&mut s, "select.anchors", json!({"id": id, "anchors": [[0, 2], [0, 3]], "segments": [[0, 2]], "mode": "add"}));
    run(&mut s, "edit.clear", json!({}));
    assert_eq!(shape(&s, id), [(vec![(110.0, 10.0), (110.0, 60.0)], false), (vec![(10.0, 60.0), (10.0, 10.0)], false)]);
    // An open line: deleting its only segment deletes the line.
    let line = run(&mut s, "shape.line", json!({"x1": 0, "y1": 0, "x2": 50, "y2": 50}))["id"].as_u64().unwrap();
    pick_segment(&mut s, line, 0, 0, 1, "set");
    run(&mut s, "edit.clear", json!({}));
    assert!(s.doc().unwrap().doc.node(NodeId(line)).is_none());
}

#[test]
fn picked_anchors_alone_still_delete_anchors() {
    let mut s = session();
    let id = run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 50}))["id"].as_u64().unwrap();
    // A point click picks no segment: Delete removes the point (and its two segments).
    run(&mut s, "select.anchors", json!({"id": id, "anchors": [[0, 1]], "mode": "set"}));
    run(&mut s, "edit.clear", json!({}));
    assert_eq!(shape(&s, id), [(vec![(110.0, 60.0), (10.0, 60.0), (10.0, 10.0)], false)]);
    // A segment whose anchor was then unpicked no longer counts.
    let id = run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 50}))["id"].as_u64().unwrap();
    pick_segment(&mut s, id, 0, 0, 1, "set");
    run(&mut s, "select.anchors", json!({"id": id, "anchors": [[0, 0]], "mode": "toggle"}));
    let st = s.doc().unwrap();
    assert!(st.selection.segments_of(&st.doc, NodeId(id)).is_empty());
}
