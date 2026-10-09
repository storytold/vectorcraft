//! Lines and shapes with the Direct Selection tool: dragging one end pivots the line, dropping an
//! open end on another joins them into one path (closing it when both are its own ends), and only
//! closed shapes keep a fill.

use serde_json::{Value, json};

use super::*;
use vectorcraft_doc::NodeId;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 300, "height": 300}));
    s
}

fn shape(s: &Session, id: u64) -> Vec<(Vec<(f64, f64)>, bool)> {
    let path = s.doc().unwrap().doc.node(NodeId(id)).unwrap().path_data().unwrap().clone();
    path.subpaths.iter().map(|sp| (sp.anchors.iter().map(|a| (a.p.x.round(), a.p.y.round())).collect(), sp.closed)).collect()
}

fn filled(s: &Session, id: u64) -> bool {
    !s.doc().unwrap().doc.node(NodeId(id)).unwrap().appearance.fill_paint().is_none()
}

fn stroked(s: &Session, id: u64) -> bool {
    !s.doc().unwrap().doc.node(NodeId(id)).unwrap().appearance.stroke_paint().is_none()
}

fn paths(s: &Session) -> Vec<u64> {
    let mut v = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if n.path_data().is_some() {
            v.push(n.id.0);
        }
    });
    v
}

/// A filled, stroked 100 × 100 box with its top edge deleted: left, bottom and right lines.
fn open_box(s: &mut Session) -> u64 {
    run(s, "paint.setFill", json!({"color": "#3366cc"}));
    let id = run(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100}))["id"].as_u64().unwrap();
    run(s, "select.anchors", json!({"id": id, "anchors": [[0, 0], [0, 1]], "segments": [[0, 0]], "mode": "set"}));
    run(s, "edit.clear", json!({}));
    id
}

#[test]
fn deleting_an_edge_leaves_unfilled_lines_over_a_fill_only_copy() {
    let mut s = session();
    let id = open_box(&mut s);
    assert_eq!(shape(&s, id), [(vec![(100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (0.0, 0.0)], false)]);
    assert!(!filled(&s, id) && stroked(&s, id), "the lines keep their stroke, not the fill");
    let all = paths(&s);
    assert_eq!(all.len(), 2);
    // The fill is its own closed shape, just below the lines, with no stroke.
    let fill = all[0];
    assert_eq!(all[1], id);
    assert!(filled(&s, fill) && !stroked(&s, fill));
    assert!(shape(&s, fill).iter().all(|(_, closed)| *closed));
    // One undo brings back the filled box.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(paths(&s), [id]);
    assert!(filled(&s, id) && stroked(&s, id));
}

#[test]
fn dragging_one_end_moves_only_that_end() {
    let mut s = session();
    let id = run(&mut s, "shape.line", json!({"x1": 0, "y1": 0, "x2": 100, "y2": 0}))["id"].as_u64().unwrap();
    run(&mut s, "select.anchors", json!({"id": id, "anchors": [[0, 1]], "mode": "set"}));
    run(&mut s, "path.moveAnchors", json!({"dx": -100, "dy": 100}));
    assert_eq!(shape(&s, id), [(vec![(0.0, 0.0), (0.0, 100.0)], false)]);
}

#[test]
fn dropping_an_end_on_the_other_end_of_its_path_closes_it_into_a_triangle() {
    let mut s = session();
    let id = open_box(&mut s);
    // Drag the right line's top end (100, 0) onto the left line's top end (0, 0): a triangle.
    run(&mut s, "select.anchors", json!({"id": id, "anchors": [[0, 0]], "mode": "set"}));
    run(&mut s, "path.moveAnchors", json!({"dx": -100, "dy": 0, "join": true}));
    assert_eq!(shape(&s, id), [(vec![(0.0, 0.0), (100.0, 100.0), (0.0, 100.0)], true)]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(shape(&s, id), [(vec![(100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (0.0, 0.0)], false)]);
}

#[test]
fn dropping_an_end_on_another_line_joins_the_two_lines() {
    let mut s = session();
    let a = run(&mut s, "shape.line", json!({"x1": 0, "y1": 0, "x2": 50, "y2": 0}))["id"].as_u64().unwrap();
    let b = run(&mut s, "shape.line", json!({"x1": 60, "y1": 0, "x2": 60, "y2": 50}))["id"].as_u64().unwrap();
    run(&mut s, "select.anchors", json!({"id": a, "anchors": [[0, 1]], "mode": "set"}));
    run(&mut s, "path.moveAnchors", json!({"dx": 10, "dy": 0, "join": true}));
    assert_eq!(shape(&s, a), [(vec![(0.0, 0.0), (60.0, 0.0), (60.0, 50.0)], false)]);
    assert!(s.doc().unwrap().doc.node(NodeId(b)).is_none());
    // Without `join` (no snap) the ends just meet.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "select.anchors", json!({"id": a, "anchors": [[0, 1]], "mode": "set"}));
    run(&mut s, "path.moveAnchors", json!({"dx": 10, "dy": 0}));
    assert!(s.doc().unwrap().doc.node(NodeId(b)).is_some());
}

#[test]
fn pen_lines_are_unfilled_until_the_path_closes() {
    let mut s = session();
    run(&mut s, "paint.setFill", json!({"color": "#3366cc"}));
    let id = run(&mut s, "path.create", json!({"anchors": [{"x": 0, "y": 0}]}))["id"].as_u64().unwrap();
    run(&mut s, "path.appendAnchor", json!({"id": id, "x": 100, "y": 0}));
    run(&mut s, "path.appendAnchor", json!({"id": id, "x": 50, "y": 80}));
    assert!(!filled(&s, id) && stroked(&s, id));
    run(&mut s, "path.close", json!({"id": id}));
    assert!(filled(&s, id));
    // The Line tool never fills.
    let l = run(&mut s, "shape.line", json!({"x1": 0, "y1": 0, "x2": 50, "y2": 50}))["id"].as_u64().unwrap();
    assert!(!filled(&s, l));
}
