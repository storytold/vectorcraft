//! Live Corners per corner: `object.setLiveShape` rounds the corners of the anchors picked with
//! the Direct Selection tool (or the corners named), and keeps those corners picked.

use serde_json::{Value, json};

use super::*;
use vectorcraft_doc::{LiveShape, NodeId, NodeKind};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

/// A selected 100 × 50 live rectangle at (10, 10).
fn rect() -> (Session, u64) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 300, "height": 300}));
    let id = run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 50}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({ "ids": [id] }));
    (s, id)
}

fn radii(s: &Session, id: u64) -> [f64; 4] {
    let n = s.doc().unwrap().doc.node(NodeId(id)).unwrap();
    let NodeKind::Path { live: Some(LiveShape::Rectangle { radii, .. }), .. } = &n.kind else { panic!("not a live rectangle") };
    *radii
}

/// The document points of the picked anchors.
fn picked(s: &Session, id: u64) -> Vec<(f64, f64)> {
    let st = s.doc().unwrap();
    let path = st.doc.node(NodeId(id)).unwrap().path_data().unwrap();
    let set = st.selection.partial(NodeId(id)).cloned().unwrap_or_default();
    set.iter().map(|(si, ai)| path.subpaths[*si].anchors[*ai].p).map(|p| (p.x.round(), p.y.round())).collect()
}

#[test]
fn a_picked_corner_rounds_alone_and_stays_picked() {
    let (mut s, id) = rect();
    // The top-right corner anchor (the rectangle runs clockwise from the top-left).
    run(&mut s, "select.anchors", json!({"id": id, "anchors": [[0, 1]], "mode": "set"}));
    run(&mut s, "object.setLiveShape", json!({"radius": 8}));
    assert_eq!(radii(&s, id), [0.0, 8.0, 0.0, 0.0]);
    // Its two new anchors are picked, so a second change keeps to that corner.
    assert_eq!(picked(&s, id), [(102.0, 10.0), (110.0, 18.0)]);
    run(&mut s, "object.setLiveShape", json!({"radius": 12}));
    assert_eq!(radii(&s, id), [0.0, 12.0, 0.0, 0.0]);
    assert_eq!(s.doc().unwrap().doc.node(NodeId(id)).unwrap().path_data().unwrap().anchor_count(), 5);
    // Shift-click another corner: both round.
    run(&mut s, "select.anchors", json!({"id": id, "anchors": [[0, 4]], "mode": "add"}));
    run(&mut s, "object.setLiveShape", json!({"radius": 5}));
    assert_eq!(radii(&s, id), [0.0, 5.0, 0.0, 5.0]);
}

#[test]
fn whole_selection_and_explicit_radii() {
    let (mut s, id) = rect();
    run(&mut s, "object.setLiveShape", json!({"radius": 4}));
    assert_eq!(radii(&s, id), [4.0; 4]);
    run(&mut s, "object.setLiveShape", json!({"radii": [1, 2, 3, -4]}));
    assert_eq!(radii(&s, id), [1.0, 2.0, 3.0, 0.0]);
    run(&mut s, "object.setLiveShape", json!({"radius": 9, "corners": [2]}));
    assert_eq!(radii(&s, id), [1.0, 2.0, 9.0, 0.0]);
    // Clamped to half the shorter side when drawn.
    run(&mut s, "object.setLiveShape", json!({"radii": [100, 0, 0, 0]}));
    let b = s.doc().unwrap().doc.node(NodeId(id)).unwrap().path_data().unwrap().bounds().unwrap();
    assert_eq!((b.x0.round(), b.y0.round(), b.x1.round(), b.y1.round()), (10.0, 10.0, 110.0, 60.0));
}
