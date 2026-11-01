//! Clear deletes its exact targets, including compound members (#786).

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::{EngineError, Session};

fn compound() -> (Session, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let mut rectangle =
        |x| NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap());
    let a = rectangle(10);
    let b = rectangle(50);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let c = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    (s, a, b, c)
}

/// The two right-hand corners of a square are indexes 1 and 2. Delete must remove those corners
/// and leave the left side, not a diagonal from shifting indexes as the path opens.
#[test]
fn clear_deletes_every_selected_anchor_of_a_closed_path() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = NodeId(s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap());
    let points = |s: &Session| {
        s.active().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0].anchors.iter().map(|a| (a.p.x, a.p.y)).collect::<Vec<_>>()
    };
    let before = points(&s);
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 1], [0, 2]]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    let sp = &s.active().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0];
    assert!(!sp.closed);
    assert_eq!(points(&s), vec![before[3], before[0]]);
    assert_eq!(s.active().unwrap().selection.objects, vec![id]);
    assert!(s.active().unwrap().selection.anchors.is_empty());
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(s.active().unwrap().doc.node(id).is_none(), "a second Delete removes the remainder");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(points(&s), before);
    // One corner still opens the square into the other three, in order.
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 1]]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!(points(&s), vec![before[2], before[3], before[0]]);
}

/// A marquee that covers one path completely and only some anchors of another stores the first as
/// a whole object. Delete removes that object and the selected anchors of the other.
#[test]
fn clear_deletes_fully_selected_objects_along_with_partial_anchors() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let mut rectangle =
        |x| NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap());
    let whole = rectangle(10);
    let partial = rectangle(80);
    let refs = |s: &Session, id| {
        s.active().unwrap().doc.node(id).unwrap().path_data().unwrap().anchors().map(|(si, ai, _)| vec![si, ai]).collect::<Vec<_>>()
    };
    let all = refs(&s, whole);
    let one = refs(&s, partial)[0].clone();
    let before = refs(&s, partial).len();
    s.execute("select.anchorsMany", &json!({"items": [{"id": whole.0, "anchors": all}, {"id": partial.0, "anchors": [one]}]})).unwrap();
    assert!(s.active().unwrap().selection.contains(whole) && s.active().unwrap().selection.partial(whole).is_none());
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(s.active().unwrap().doc.node(whole).is_none());
    assert_eq!(refs(&s, partial).len(), before - 1);
    assert_eq!(s.active().unwrap().selection.objects, vec![partial]);
    assert!(s.active().unwrap().selection.anchors.is_empty());
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(s.active().unwrap().doc.node(partial).is_none(), "a second Delete removes what the first left");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.active().unwrap().doc.node(whole).is_some());
    assert_eq!(refs(&s, partial).len(), before);
}

#[test]
fn clear_selected_compound_member_preserves_its_sibling_and_is_undoable() {
    let (mut s, a, b, c) = compound();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(s.active().unwrap().doc.node(a).is_none());
    assert_eq!(s.active().unwrap().doc.parent_of(b), Some(c));
    assert!(s.active().unwrap().selection.is_empty());
    assert_eq!(s.execute("edit.undo", &json!({})).unwrap()["undone"], "Clear");
    assert_eq!(s.active().unwrap().doc.parent_of(a), Some(c));
    assert_eq!(s.active().unwrap().selection.objects, vec![a]);
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(s.active().unwrap().doc.node(a).is_none());
    assert_eq!(s.active().unwrap().doc.parent_of(b), Some(c));
}

#[test]
fn clear_explicit_ids_works_without_a_selection_and_overrides_selected_anchors() {
    let (mut s, a, b, c) = compound();
    s.execute("select.none", &json!({})).unwrap();
    let info = s.commands().into_iter().find(|info| info.id == "edit.clear").unwrap();
    assert!(!info.enabled, "the menu stays disabled with no selection");
    s.execute("edit.clear", &json!({"ids": [a.0]})).unwrap();
    assert!(s.active().unwrap().doc.node(a).is_none());
    assert_eq!(s.active().unwrap().doc.parent_of(b), Some(c));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.anchors", &json!({"id": b.0, "anchors": [[0, 0]]})).unwrap();
    let sibling = s.active().unwrap().doc.node(b).unwrap().clone();
    s.execute("edit.clear", &json!({"ids": [a.0]})).unwrap();
    assert!(s.active().unwrap().doc.node(a).is_none());
    assert_eq!(s.active().unwrap().doc.node(b).unwrap(), &sibling);
}

#[test]
fn clear_explicit_targets_are_validated_atomically() {
    let (mut s, a, _, _) = compound();
    s.execute("select.none", &json!({})).unwrap();
    for ids in [json!([a.0, 999]), json!([a.0, -1]), json!([a.0, "bad"]), json!([a.0, 2.5]), json!(null), json!(a.0)] {
        let doc = s.active().unwrap().doc.clone();
        let journal = s.journal.clone();
        assert!(matches!(s.execute("edit.clear", &json!({"ids": ids})), Err(EngineError::BadParams { .. })), "{ids}");
        assert_eq!(s.active().unwrap().doc, doc);
        assert_eq!(s.journal, journal);
    }
    assert!(s.execute("edit.clear", &json!({})).is_err());
    assert!(Session::new().execute("edit.clear", &json!({"ids": [a.0]})).is_err());
}

#[test]
fn clear_compound_and_member_together_deletes_the_compound() {
    for explicit in [false, true] {
        let (mut s, a, b, c) = compound();
        s.execute("select.set", &json!({"ids": [a.0, c.0]})).unwrap();
        s.execute("edit.clear", &if explicit { json!({"ids": [a.0, c.0, c.0]}) } else { json!({}) }).unwrap();
        for id in [a, b, c] {
            assert!(s.active().unwrap().doc.node(id).is_none());
        }
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.parent_of(a), Some(c));
        assert_eq!(s.active().unwrap().doc.parent_of(b), Some(c));
    }
}

#[test]
fn cut_compound_member_keeps_copy_and_delete_targets_consistent() {
    let (mut s, a, b, c) = compound();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("edit.cut", &json!({})).unwrap();
    assert_eq!(s.clipboard.nodes.len(), 1);
    assert_eq!(s.clipboard.nodes[0].id, c);
    assert!(s.active().unwrap().doc.node(c).is_none());
    s.execute("edit.pasteInPlace", &json!({})).unwrap();
    let pasted = s.active().unwrap().selection.objects[0];
    assert_eq!(s.active().unwrap().doc.node(pasted).unwrap().children().unwrap().len(), 2);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.parent_of(a), Some(c));
    assert_eq!(s.active().unwrap().doc.parent_of(b), Some(c));
}

#[test]
fn clear_direct_selected_anchors_keeps_the_rest_of_the_path() {
    let (mut s, a, b, c) = compound();
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 0]]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.node(a).unwrap().path_data().unwrap().anchor_count(), 3);
    assert_eq!(s.active().unwrap().doc.parent_of(b), Some(c));
}

#[test]
fn clear_guides_and_empty_explicit_targets_keep_their_separate_behavior() {
    let (mut s, a, _, _) = compound();
    s.execute("guide.add", &json!({"vertical": true, "pos": 10})).unwrap();
    s.execute("guide.select", &json!({"indexes": [0]})).unwrap();
    let history = s.active().unwrap().history.undo.len();
    let selection = s.active().unwrap().selection.clone();
    s.execute("edit.clear", &json!({"ids": []})).unwrap();
    assert_eq!(s.active().unwrap().history.undo.len(), history);
    assert_eq!(s.active().unwrap().selection, selection);
    assert_eq!(s.active().unwrap().doc.guides.len(), 1);
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(s.active().unwrap().doc.guides.is_empty());
    assert!(s.active().unwrap().doc.node(a).is_some());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.guides.len(), 1);
}

#[test]
fn clear_keeps_layers_but_deletes_an_explicitly_targeted_child() {
    let (mut s, a, b, c) = compound();
    let layer = s.active().unwrap().doc.layers[0].id;
    s.execute("select.none", &json!({})).unwrap();
    s.execute("edit.clear", &json!({"ids": [layer.0, a.0]})).unwrap();
    assert!(s.active().unwrap().doc.node(layer).is_some());
    assert!(s.active().unwrap().doc.node(a).is_none());
    assert_eq!(s.active().unwrap().doc.parent_of(b), Some(c));
}

#[test]
fn cut_direct_selected_anchors_retains_anchor_deletion() {
    let (mut s, a, b, c) = compound();
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 0]]})).unwrap();
    s.execute("edit.cut", &json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.node(a).unwrap().path_data().unwrap().anchor_count(), 3);
    assert_eq!(s.active().unwrap().doc.parent_of(b), Some(c));
}
