//! Direct-selection Copy and Cut tests (#2122).

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_engine::Session;
use vectorcraft_geom::FillRule;
use vectorcraft_testkit::invariants::check_session;

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.execute(command, &params).unwrap()
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    NodeId(run(s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h}))["id"].as_u64().unwrap())
}

#[test]
fn copy_and_paste_direct_selected_segment_of_rect() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let r = rect(&mut s, 10.0, 10.0, 100.0, 50.0);

    // Select segment 0 of the rectangle.
    run(&mut s, "select.segments", json!({"id": r.0, "segments": [[0, 0]]}));
    run(&mut s, "edit.copy", json!({}));
    let pasted = run(&mut s, "edit.paste", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 1);

    let pasted_node = s.doc().unwrap().doc.node(NodeId(ids[0])).unwrap();
    if let NodeKind::Path { live, .. } = &pasted_node.kind {
        assert!(live.is_none(), "extracted partial path must not retain live shape metadata");
    }
    let path = pasted_node.path_data().unwrap();
    assert_eq!(path.subpaths.len(), 1);
    let sp = &path.subpaths[0];
    assert!(!sp.closed);
    assert_eq!(sp.anchors.len(), 2);
}

#[test]
fn copy_and_paste_direct_selected_adjacent_anchors_of_rect() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let r = rect(&mut s, 10.0, 10.0, 100.0, 50.0);

    // Direct-select two adjacent anchors: 0 and 1.
    run(&mut s, "select.anchors", json!({"id": r.0, "anchors": [[0, 0], [0, 1]], "mode": "set"}));
    run(&mut s, "edit.copy", json!({}));
    let pasted = run(&mut s, "edit.paste", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 1);

    let pasted_node = s.doc().unwrap().doc.node(NodeId(ids[0])).unwrap();
    let path = pasted_node.path_data().unwrap();
    assert_eq!(path.subpaths.len(), 1);
    let sp = &path.subpaths[0];
    assert!(!sp.closed);
    assert_eq!(sp.anchors.len(), 2);
}

#[test]
fn copy_single_anchor_of_rect_copies_three_anchors_with_two_segments() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let r = rect(&mut s, 10.0, 10.0, 100.0, 50.0);

    // Direct-select only anchor 1 of the closed 4-anchor rectangle.
    run(&mut s, "select.anchors", json!({"id": r.0, "anchors": [[0, 1]], "mode": "set"}));
    run(&mut s, "edit.copy", json!({}));
    let pasted = run(&mut s, "edit.paste", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 1);

    let pasted_node = s.doc().unwrap().doc.node(NodeId(ids[0])).unwrap();
    let path = pasted_node.path_data().unwrap();
    assert_eq!(path.subpaths.len(), 1);
    let sp = &path.subpaths[0];
    assert!(!sp.closed);
    // Copying a single anchor on a closed path copies the anchor and both its incident segments.
    assert_eq!(sp.anchors.len(), 3);
    assert_eq!(sp.segment_count(), 2);
}

#[test]
fn copy_disjoint_segments_does_not_copy_unselected_segment_in_between() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let r = rect(&mut s, 10.0, 10.0, 100.0, 50.0);

    // Rectangle has 4 segments (0, 1, 2, 3). Select disjoint segments 0 and 2.
    run(&mut s, "select.segments", json!({"id": r.0, "segments": [[0, 0], [0, 2]]}));
    run(&mut s, "edit.copy", json!({}));
    let pasted = run(&mut s, "edit.paste", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    // Two separate open path pieces should be created, each with 2 anchors.
    assert_eq!(ids.len(), 2);
    for id in ids {
        let node = s.doc().unwrap().doc.node(NodeId(id)).unwrap();
        let path = node.path_data().unwrap();
        assert_eq!(path.subpaths.len(), 1);
        let sp = &path.subpaths[0];
        assert!(!sp.closed);
        assert_eq!(sp.anchors.len(), 2);
    }
}

#[test]
fn cut_direct_selected_segment_cuts_source_and_pastes_segment() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let r = rect(&mut s, 10.0, 10.0, 100.0, 50.0);

    // Select segment 0 and cut it.
    run(&mut s, "select.segments", json!({"id": r.0, "segments": [[0, 0]]}));
    run(&mut s, "edit.cut", json!({}));

    // Source rectangle should now have segment 0 removed, turning it into an open path.
    let src_node = s.doc().unwrap().doc.node(r).unwrap();
    let src_path = src_node.path_data().unwrap();
    assert_eq!(src_path.subpaths.len(), 1);
    assert!(!src_path.subpaths[0].closed);

    // Paste the cut segment:
    let pasted = run(&mut s, "edit.paste", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 1);
    let pasted_node = s.doc().unwrap().doc.node(NodeId(ids[0])).unwrap();
    let pasted_path = pasted_node.path_data().unwrap();
    assert_eq!(pasted_path.subpaths.len(), 1);
    assert!(!pasted_path.subpaths[0].closed);
    assert_eq!(pasted_path.subpaths[0].anchors.len(), 2);
}

#[test]
fn copy_compound_path_direct_selected_segment_inherits_appearance() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let outer = NodeId(run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 80, "height": 80}))["id"].as_u64().unwrap());
    let inner = NodeId(run(&mut s, "shape.rectangle", json!({"x": 30, "y": 30, "width": 40, "height": 40}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [outer.0, inner.0]}));
    let compound = NodeId(run(&mut s, "object.compoundPath.make", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));

    // Select segment 0 of the inner member.
    run(&mut s, "select.segments", json!({"id": inner.0, "segments": [[0, 0]]}));
    run(&mut s, "edit.copy", json!({}));
    let pasted = run(&mut s, "edit.paste", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 1);

    let pasted_node = s.doc().unwrap().doc.node(NodeId(ids[0])).unwrap();
    let compound_node = s.doc().unwrap().doc.node(compound).unwrap();
    assert_eq!(pasted_node.appearance, compound_node.appearance);
}

#[test]
fn direct_selection_copy_preserves_fill_rule_and_attributes() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let r = rect(&mut s, 10.0, 10.0, 100.0, 50.0);
    run(&mut s, "path.setFillRule", json!({"rule": "evenOdd"}));

    run(&mut s, "select.segments", json!({"id": r.0, "segments": [[0, 0]]}));
    run(&mut s, "edit.copy", json!({}));
    let pasted = run(&mut s, "edit.paste", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    let pasted_node = s.doc().unwrap().doc.node(NodeId(ids[0])).unwrap();
    assert!(matches!(pasted_node.kind, NodeKind::Path { rule: FillRule::EvenOdd, .. }));
}

#[test]
fn pen_tool_drawing_copies_whole_path_not_single_anchor() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    // Start drawing with Pen tool
    s.select_tool("pen", vectorcraft_engine::ViewInfo::default()).unwrap();
    let id = NodeId(run(&mut s, "path.create", json!({"anchors": [{"x": 10, "y": 10}]}))["id"].as_u64().unwrap());
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 50, "y": 10}));
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 50, "y": 50}));

    // Pen tool has the last anchor in sel.anchors, but active tool is pen
    run(&mut s, "edit.copy", json!({}));
    let pasted = run(&mut s, "edit.paste", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 1);

    // The whole 3-point path must be copied, not just the last anchor!
    let pasted_node = s.doc().unwrap().doc.node(NodeId(ids[0])).unwrap();
    let path = pasted_node.path_data().unwrap();
    assert_eq!(path.subpaths[0].anchors.len(), 3);
}

#[test]
fn cut_adjacent_anchors_clipboard_plus_remaining_equals_original() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let r = rect(&mut s, 10.0, 10.0, 100.0, 50.0);

    // Direct-select two adjacent anchors: 1 and 2 of the 4-anchor rectangle.
    run(&mut s, "select.anchors", json!({"id": r.0, "anchors": [[0, 1], [0, 2]], "mode": "set"}));
    run(&mut s, "edit.cut", json!({}));

    // Cut placed 1 segment on the clipboard (anchors 1 to 2).
    assert_eq!(s.clipboard.nodes.len(), 1);
    let clip_path = s.clipboard.nodes[0].path_data().unwrap();
    assert_eq!(clip_path.subpaths[0].segment_count(), 1);
    assert_eq!(clip_path.subpaths[0].anchors.len(), 2);

    // Remaining source path must have the remaining 3 segments (segments 0, 2, 3), not 1.
    let src_node = s.doc().unwrap().doc.node(r).unwrap();
    let src_path = src_node.path_data().unwrap();
    assert_eq!(src_path.subpaths[0].segment_count(), 3);
    assert_eq!(src_path.subpaths[0].anchors.len(), 4);
    assert!(!src_path.subpaths[0].closed);

    // Sum of segments in clipboard + remaining = 1 + 3 = 4 (the original 4 segments).
    assert_eq!(clip_path.subpaths[0].segment_count() + src_path.subpaths[0].segment_count(), 4);
}

#[test]
fn copy_non_adjacent_anchors_on_rect_copies_full_object() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let r = rect(&mut s, 10.0, 10.0, 100.0, 50.0);

    // Select non-adjacent anchors 1 and 3 of the 4-anchor rectangle.
    // Incident segments for anchor 1 are segments 0 and 1; for anchor 3 are segments 2 and 3.
    // Together they cover all 4 segments, so the path is copied whole.
    run(&mut s, "select.anchors", json!({"id": r.0, "anchors": [[0, 1], [0, 3]], "mode": "set"}));
    run(&mut s, "edit.copy", json!({}));
    let pasted = run(&mut s, "edit.paste", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 1);

    let pasted_node = s.doc().unwrap().doc.node(NodeId(ids[0])).unwrap();
    if let NodeKind::Path { live, .. } = &pasted_node.kind {
        assert!(live.is_some(), "copying all segments keeps live shape metadata");
    }
    let path = pasted_node.path_data().unwrap();
    assert!(path.subpaths[0].closed);
    assert_eq!(path.subpaths[0].segment_count(), 4);
}

#[test]
fn copy_non_adjacent_anchors_on_five_point_path_extracts_selected_segments() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let id = NodeId(run(&mut s, "path.create", json!({"anchors": [{"x": 10, "y": 10}]}))["id"].as_u64().unwrap());
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 50, "y": 10}));
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 60, "y": 50}));
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 30, "y": 80}));
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 10, "y": 50}));
    run(&mut s, "path.close", json!({"id": id.0}));

    // Path has 5 anchors (0..4) and 5 segments (0..4).
    // Select non-adjacent anchors 1 and 3.
    // Anchor 1 gives segments 0, 1. Anchor 3 gives segments 2, 3. Segment 4 is unselected.
    run(&mut s, "select.anchors", json!({"id": id.0, "anchors": [[0, 1], [0, 3]], "mode": "set"}));
    run(&mut s, "edit.copy", json!({}));
    let pasted = run(&mut s, "edit.paste", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 1);

    let pasted_node = s.doc().unwrap().doc.node(NodeId(ids[0])).unwrap();
    let path = pasted_node.path_data().unwrap();
    assert!(!path.subpaths[0].closed);
    assert_eq!(path.subpaths[0].segment_count(), 4);
    assert_eq!(path.subpaths[0].anchors.len(), 5);
}

#[test]
fn copy_when_all_segments_selected_preserves_live_shape() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let r = rect(&mut s, 10.0, 10.0, 100.0, 50.0);

    // Select all 4 segments explicitly (like Select > Object > Direction Handles).
    run(&mut s, "select.segments", json!({"id": r.0, "segments": [[0, 0], [0, 1], [0, 2], [0, 3]]}));
    run(&mut s, "edit.copy", json!({}));
    let pasted = run(&mut s, "edit.paste", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 1);

    let pasted_node = s.doc().unwrap().doc.node(NodeId(ids[0])).unwrap();
    if let NodeKind::Path { live, .. } = &pasted_node.kind {
        assert!(live.is_some(), "all segments selected preserves live shape");
    }
}

#[test]
fn compound_member_selected_whole_with_direct_selection_promotes_member() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let outer = NodeId(run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 80, "height": 80}))["id"].as_u64().unwrap());
    let inner = NodeId(run(&mut s, "shape.rectangle", json!({"x": 30, "y": 30, "width": 40, "height": 40}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [outer.0, inner.0]}));
    let compound = NodeId(run(&mut s, "object.compoundPath.make", json!({}))["id"].as_u64().unwrap());

    let standalone = rect(&mut s, 120.0, 10.0, 50.0, 50.0);

    // Select outer compound member whole, and direct-select anchor 0 on standalone.
    let outer_anchors: Vec<Vec<usize>> = (0..4).map(|i| vec![0, i]).collect();
    run(
        &mut s,
        "select.anchorsMany",
        json!({
            "items": [
                {"id": outer.0, "anchors": outer_anchors},
                {"id": standalone.0, "anchors": [[0, 0]]}
            ]
        }),
    );

    // outer is selected whole, standalone is partially selected.
    assert!(s.active().unwrap().selection.partial(outer).is_none());
    assert!(s.active().unwrap().selection.partial(standalone).is_some());

    run(&mut s, "edit.copy", json!({}));
    // Clipboard should have:
    // 1. The compound path (outer promoted to compound #786)
    // 2. The extracted segment piece from standalone
    let clip_ids: Vec<NodeId> = s.clipboard.nodes.iter().map(|n| n.id).collect();
    assert_eq!(clip_ids.len(), 2, "promoted compound plus the extracted standalone piece");
    assert!(clip_ids.contains(&compound), "whole compound member must be promoted to compound parent");
}

#[test]
fn cut_whole_compound_member_with_direct_selection_cuts_compound_and_extracted_piece() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let outer = NodeId(run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 80, "height": 80}))["id"].as_u64().unwrap());
    let inner = NodeId(run(&mut s, "shape.rectangle", json!({"x": 30, "y": 30, "width": 40, "height": 40}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [outer.0, inner.0]}));
    let compound = NodeId(run(&mut s, "object.compoundPath.make", json!({}))["id"].as_u64().unwrap());

    let standalone = NodeId(run(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 50, "height": 50}))["id"].as_u64().unwrap());

    let outer_anchors = (0..4).map(|i| vec![0, i]).collect::<Vec<_>>();
    run(
        &mut s,
        "select.anchorsMany",
        json!({
            "items": [
                {"id": outer.0, "anchors": outer_anchors},
                {"id": standalone.0, "anchors": [[0, 0]]}
            ]
        }),
    );

    run(&mut s, "edit.cut", json!({}));
    let clip_ids: Vec<NodeId> = s.clipboard.nodes.iter().map(|n| n.id).collect();
    assert_eq!(clip_ids.len(), 2, "promoted compound plus the extracted standalone piece");
    assert!(clip_ids.contains(&compound), "whole compound member must be promoted to compound parent");

    // Compound was removed because member was selected whole
    let doc = &s.doc().unwrap().doc;
    assert!(doc.node(compound).is_none(), "compound path must be removed on cut");
    // Standalone has cut pieces remaining
    assert!(doc.node(standalone).is_some(), "partially cut standalone path remains");
}

#[test]
fn cut_direct_selected_only_path_in_layer_preserves_layer() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    // Create a new layer
    let layer_id = NodeId(run(&mut s, "layer.new", json!({}))["id"].as_u64().unwrap());
    // Create a path inside the layer
    let rect = NodeId(run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50}))["id"].as_u64().unwrap());
    let doc = &s.doc().unwrap().doc;
    assert_eq!(doc.parent_of(rect), Some(layer_id));

    // Direct-select all 4 anchors of rect and cut
    let all_anchors = (0..4).map(|i| vec![0, i]).collect::<Vec<_>>();
    run(&mut s, "select.anchors", json!({"id": rect.0, "anchors": all_anchors, "mode": "set"}));
    run(&mut s, "edit.cut", json!({}));

    // The rect was cut, but the layer must NOT be deleted
    let doc = &s.doc().unwrap().doc;
    assert!(doc.node(rect).is_none(), "rect was cut");
    assert!(doc.node(layer_id).is_some(), "layer must remain even when empty");
    assert_eq!(doc.node(layer_id).unwrap().children().unwrap().len(), 0);
}

#[test]
fn copy_and_paste_path_in_compound_path_isolation_adds_member_to_compound() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let outer = NodeId(run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 80, "height": 80}))["id"].as_u64().unwrap());
    let inner = NodeId(run(&mut s, "shape.rectangle", json!({"x": 30, "y": 30, "width": 40, "height": 40}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [outer.0, inner.0]}));
    let compound = NodeId(run(&mut s, "object.compoundPath.make", json!({}))["id"].as_u64().unwrap());

    // Enter isolation mode on the compound path.
    run(&mut s, "object.isolate", json!({"id": compound.0}));
    assert_eq!(s.doc().unwrap().isolation, Some(compound));

    // In isolation mode, select member `inner` whole.
    run(&mut s, "select.set", json!({"ids": [inner.0]}));
    run(&mut s, "edit.copy", json!({}));

    // Clipboard copies the selected member path, NOT the entire compound path.
    assert_eq!(s.clipboard.nodes.len(), 1);
    assert_eq!(s.clipboard.nodes[0].id, inner);

    // Paste in isolation mode: the pasted path must be inserted into the compound path,
    // not as a new path beside the compound path.
    let pasted = run(&mut s, "edit.paste", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 1);
    let pasted_id = NodeId(ids[0]);

    let doc = &s.doc().unwrap().doc;
    assert_eq!(doc.parent_of(pasted_id), Some(compound), "pasted path must be added to the compound path");
    let compound_node = doc.node(compound).unwrap();
    assert_eq!(compound_node.children().unwrap().len(), 3, "compound path now has 3 member paths");
}

#[test]
fn cut_and_paste_direct_selected_anchors_in_compound_path_isolation() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let outer = NodeId(run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 80, "height": 80}))["id"].as_u64().unwrap());
    let inner = NodeId(run(&mut s, "shape.rectangle", json!({"x": 30, "y": 30, "width": 40, "height": 40}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [outer.0, inner.0]}));
    let compound = NodeId(run(&mut s, "object.compoundPath.make", json!({}))["id"].as_u64().unwrap());

    // Enter isolation mode on the compound path.
    run(&mut s, "object.isolate", json!({"id": compound.0}));

    // Direct-select anchor 0 of `inner` and cut.
    run(&mut s, "select.anchors", json!({"id": inner.0, "anchors": [[0, 0]], "mode": "set"}));
    run(&mut s, "edit.cut", json!({}));

    // Clipboard holds the cut segment piece.
    assert_eq!(s.clipboard.nodes.len(), 1);

    // Paste inside isolation mode:
    let pasted = run(&mut s, "edit.paste", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 1);
    let pasted_id = NodeId(ids[0]);

    let doc = &s.doc().unwrap().doc;
    assert_eq!(doc.parent_of(pasted_id), Some(compound), "cut and pasted anchors must be inserted into compound path");
    let compound_node = doc.node(compound).unwrap();
    assert_eq!(compound_node.children().unwrap().len(), 3);
    check_session(&s).unwrap();
}

#[test]
fn paste_non_path_in_compound_path_isolation_inserts_into_layer() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let a = rect(&mut s, 10.0, 10.0, 40.0, 40.0);
    let b = rect(&mut s, 60.0, 10.0, 40.0, 40.0);
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    let _group = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());

    // Copy the group to the clipboard.
    run(&mut s, "edit.copy", json!({}));

    // Create a compound path on the layer.
    let c1 = rect(&mut s, 10.0, 100.0, 80.0, 80.0);
    let c2 = rect(&mut s, 30.0, 120.0, 40.0, 40.0);
    run(&mut s, "select.set", json!({"ids": [c1.0, c2.0]}));
    let compound = NodeId(run(&mut s, "object.compoundPath.make", json!({}))["id"].as_u64().unwrap());
    let layer_id = s.doc().unwrap().doc.parent_of(compound).unwrap();

    // Isolate the compound path.
    run(&mut s, "object.isolate", json!({"id": compound.0}));
    assert_eq!(s.doc().unwrap().isolation, Some(compound));

    // Generic insertion parent inside compound isolation resolves to the layer, not the compound.
    assert_eq!(s.doc().unwrap().insertion_parent(), Some(layer_id));

    // Paste the group while in compound path isolation.
    // The group cannot be inserted into the compound path; it must be pasted into the layer beside the compound.
    let pasted = run(&mut s, "edit.paste", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 1);
    let pasted_group = NodeId(ids[0]);

    let doc = &s.doc().unwrap().doc;
    assert_eq!(doc.parent_of(pasted_group), Some(layer_id), "pasted group must be added to layer, not compound");
    let compound_node = doc.node(compound).unwrap();
    assert_eq!(compound_node.children().unwrap().len(), 2, "compound path must keep only its 2 path children");

    // All document invariants (including compound children being paths only) must hold.
    check_session(&s).unwrap();
}

#[test]
fn shape_creation_in_compound_path_isolation_inserts_into_layer() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let outer = rect(&mut s, 10.0, 10.0, 80.0, 80.0);
    let inner = rect(&mut s, 30.0, 30.0, 40.0, 40.0);
    run(&mut s, "select.set", json!({"ids": [outer.0, inner.0]}));
    let compound = NodeId(run(&mut s, "object.compoundPath.make", json!({}))["id"].as_u64().unwrap());

    // Isolate the compound path.
    run(&mut s, "object.isolate", json!({"id": compound.0}));

    // Draw a new rectangle while isolating the compound path.
    let new_rect = rect(&mut s, 40.0, 40.0, 20.0, 20.0);

    let doc = &s.doc().unwrap().doc;
    assert_ne!(doc.parent_of(new_rect), Some(compound), "new shape path is placed outside compound path in layer");
    let compound_node = doc.node(compound).unwrap();
    assert_eq!(compound_node.children().unwrap().len(), 2, "compound path maintains original member paths");

    check_session(&s).unwrap();
}

#[test]
fn paste_compound_into_isolated_compound_preserves_child_order() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 300, "height": 300}));
    let r1 = rect(&mut s, 10.0, 10.0, 10.0, 10.0);
    let r2 = rect(&mut s, 30.0, 10.0, 10.0, 10.0);
    let r3 = rect(&mut s, 50.0, 10.0, 10.0, 10.0);
    run(&mut s, "select.set", json!({"ids": [r1.0, r2.0, r3.0]}));
    let src_compound = NodeId(run(&mut s, "object.compoundPath.make", json!({}))["id"].as_u64().unwrap());

    // Copy src_compound (contains [r1, r2, r3] in order)
    run(&mut s, "select.set", json!({"ids": [src_compound.0]}));
    run(&mut s, "edit.copy", json!({}));

    // Create target compound
    let t1 = rect(&mut s, 100.0, 100.0, 80.0, 80.0);
    let t2 = rect(&mut s, 120.0, 120.0, 40.0, 40.0);
    run(&mut s, "select.set", json!({"ids": [t1.0, t2.0]}));
    let target_compound = NodeId(run(&mut s, "object.compoundPath.make", json!({}))["id"].as_u64().unwrap());

    // Isolate target compound and select member t1 so paste-in-front targets index 1
    run(&mut s, "object.isolate", json!({"id": target_compound.0}));
    run(&mut s, "select.set", json!({"ids": [t1.0]}));

    // Paste in front of t1
    let pasted = run(&mut s, "edit.pasteInFront", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 3, "unpacked all 3 children");

    let doc = &s.doc().unwrap().doc;
    let target_node = doc.node(target_compound).unwrap();
    let children = target_node.children().unwrap();
    assert_eq!(children.len(), 5);

    // Verify the 3 pasted children appear in children in exact forward order [r1_copy, r2_copy, r3_copy]
    let child_ids: Vec<u64> = children.iter().map(|c| c.id.0).collect();
    let pos0 = child_ids.iter().position(|&id| id == ids[0]).unwrap();
    let pos1 = child_ids.iter().position(|&id| id == ids[1]).unwrap();
    let pos2 = child_ids.iter().position(|&id| id == ids[2]).unwrap();
    assert_eq!(pos1, pos0 + 1, "child 1 must immediately follow child 0");
    assert_eq!(pos2, pos1 + 1, "child 2 must immediately follow child 1");

    check_session(&s).unwrap();
}
