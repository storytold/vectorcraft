//! A copy of a clip group's clipping path pasted outside it is an ordinary path (#9227).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_engine::Session;

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.execute(command, &params).unwrap()
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    NodeId(run(s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h}))["id"].as_u64().unwrap())
}

fn clipping(s: &Session, id: NodeId) -> bool {
    matches!(s.doc().unwrap().doc.node(id).unwrap().kind, NodeKind::Path { clipping: true, .. })
}

/// A rectangle clipped by a smaller one, and the clipping path's id.
fn clip_group(s: &mut Session) -> NodeId {
    run(s, "file.new", json!({"width": 400, "height": 400}));
    let art = rect(s, 20.0, 20.0, 200.0, 200.0);
    let mask = rect(s, 60.0, 60.0, 100.0, 100.0);
    run(s, "select.set", json!({"ids": [art.0, mask.0]}));
    run(s, "object.clippingMask.make", json!({}));
    assert!(clipping(s, mask));
    mask
}

#[test]
fn offset_path_offsets_a_pasted_copy_of_a_clipping_path() {
    let mut s = Session::new();
    let mask = clip_group(&mut s);
    run(&mut s, "select.set", json!({"ids": [mask.0]}));
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "select.none", json!({}));
    let pasted = run(&mut s, "edit.pasteInPlace", json!({}));
    let copy = NodeId(pasted["ids"][0].as_u64().unwrap());
    assert!(!clipping(&s, copy), "the pasted copy clips nothing, so it is not a clipping path");
    assert!(clipping(&s, mask), "the original still clips its group");

    run(&mut s, "select.set", json!({"ids": [copy.0]}));
    let out = s.execute("object.path.offsetPath", &json!({"offset": 5}));
    assert!(out.is_ok(), "Offset Path refuses the pasted copy: {out:?}");
}

#[test]
fn release_makes_a_lone_clipping_path_an_ordinary_path() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 400}));
    let lone = rect(&mut s, 60.0, 60.0, 100.0, 100.0);
    // As a paste made one before #9227, in a document saved then: flagged, outside any clip group.
    let d = Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    if let NodeKind::Path { clipping, .. } = &mut d.node_mut(lone).unwrap().kind {
        *clipping = true;
    }
    run(&mut s, "select.set", json!({"ids": [lone.0]}));
    assert!(s.execute("object.path.offsetPath", &json!({"offset": 5})).is_err(), "the flagged path is skipped");

    run(&mut s, "object.clippingMask.release", json!({}));
    assert!(!clipping(&s, lone));
    let out = s.execute("object.path.offsetPath", &json!({"offset": 5}));
    assert!(out.is_ok(), "Offset Path refuses the released path: {out:?}");
}
