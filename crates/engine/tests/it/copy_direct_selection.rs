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
fn cut_adjacent_anchors_takes_them_out_of_the_source() {
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

    // The source keeps anchors 3 and 0 and the one segment between them.
    let src_node = s.doc().unwrap().doc.node(r).unwrap();
    let src_path = src_node.path_data().unwrap();
    assert_eq!(src_path.subpaths[0].segment_count(), 1);
    assert_eq!(src_path.subpaths[0].anchors.len(), 2);
    assert!(!src_path.subpaths[0].closed);
}

/// Every contour left in the document after `command` runs on anchors `picked` of a four-anchor
/// open path, as `[x, y]` lists. With `compound`, the path is a member of a compound path.
fn contours_after(command: &str, picked: Value, compound: bool) -> Vec<Vec<[f64; 2]>> {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let id = NodeId(run(&mut s, "path.create", json!({"anchors": [{"x": 10, "y": 20}]}))["id"].as_u64().unwrap());
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 60, "y": 20}));
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 100, "y": 80}));
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 150, "y": 80}));
    if compound {
        let other = rect(&mut s, 10.0, 100.0, 40.0, 40.0);
        run(&mut s, "select.set", json!({"ids": [id.0, other.0]}));
        run(&mut s, "object.compoundPath.make", json!({}));
    }
    run(&mut s, "select.anchors", json!({"id": id.0, "anchors": picked, "mode": "set"}));
    run(&mut s, command, json!({}));
    check_session(&s).unwrap();
    contours(&s)
}

/// Every contour in the document, as `[x, y]` lists.
fn contours(s: &Session) -> Vec<Vec<[f64; 2]>> {
    let mut left = vec![];
    for layer in &s.doc().unwrap().doc.layers {
        layer.walk(&mut |n| {
            if let Some(path) = n.path_data() {
                left.extend(path.subpaths.iter().map(|sp| sp.anchors.iter().map(|a| [a.p.x, a.p.y]).collect::<Vec<_>>()));
            }
        });
    }
    left
}

#[test]
fn cut_end_anchors_leaves_the_source_as_clear_does() {
    for compound in [false, true] {
        let picked = || json!([[0, 0], [0, 1]]);
        let left = contours_after("edit.cut", picked(), compound);
        assert_eq!(left, contours_after("edit.clear", picked(), compound), "compound: {compound}");
        assert_eq!(left[0], [[100.0, 80.0], [150.0, 80.0]], "compound: {compound}");
    }
}

#[test]
fn cut_interior_anchors_leaves_the_source_as_clear_does() {
    let picked = || json!([[0, 1], [0, 2]]);
    let left = contours_after("edit.cut", picked(), true);
    assert_eq!(left, contours_after("edit.clear", picked(), true));
    assert!(left.iter().flatten().all(|p| p[0] != 60.0 && p[0] != 100.0), "the picked anchors are gone: {left:?}");
}

#[test]
fn cut_one_end_anchor_of_a_compound_member_shortens_it() {
    let left = contours_after("edit.cut", json!([[0, 3]]), true);
    assert_eq!(left[0], [[10.0, 20.0], [60.0, 20.0], [100.0, 80.0]]);
}

#[test]
fn cut_of_an_anchor_with_a_segment_on_another_contour_leaves_the_source_as_clear_does() {
    let after = |command: &str| {
        let mut s = Session::new();
        run(&mut s, "file.new", json!({"width": 400, "height": 400}));
        let id = NodeId(run(&mut s, "path.create", json!({"anchors": [{"x": 10, "y": 20}]}))["id"].as_u64().unwrap());
        let contour = |y: i32| json!({"anchors": [{"x": 10, "y": y}, {"x": 60, "y": y}, {"x": 110, "y": y}], "closed": false});
        run(&mut s, "path.setAnchors", json!({"id": id.0, "subpaths": [contour(20), contour(120)]}));
        run(&mut s, "select.anchors", json!({"id": id.0, "anchors": [[0, 0]], "mode": "set"}));
        run(&mut s, "select.segments", json!({"id": id.0, "segments": [[1, 1]], "mode": "add"}));
        run(&mut s, command, json!({}));
        check_session(&s).unwrap();
        s
    };
    let cut = after("edit.cut");
    assert_eq!(contours(&cut), contours(&after("edit.clear")));
    assert!(contours(&cut).contains(&vec![[10.0, 20.0], [60.0, 20.0], [110.0, 20.0]]), "the picked anchor stays: {:?}", contours(&cut));
    // The clipboard holds what left the source: the picked segment alone.
    assert_eq!(cut.clipboard.nodes.len(), 1);
    let clip = cut.clipboard.nodes[0].path_data().unwrap();
    assert_eq!(clip.subpaths.len(), 1);
    assert_eq!(clip.subpaths[0].anchors.iter().map(|a| [a.p.x, a.p.y]).collect::<Vec<_>>(), [[60.0, 120.0], [110.0, 120.0]]);
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
fn compound_member_selected_whole_with_direct_selection_copies_only_the_member() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let outer = NodeId(run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 80, "height": 80}))["id"].as_u64().unwrap());
    let inner = NodeId(run(&mut s, "shape.rectangle", json!({"x": 30, "y": 30, "width": 40, "height": 40}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [outer.0, inner.0]}));
    let compound = NodeId(run(&mut s, "object.compoundPath.make", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));

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

    let undo_before = s.doc().unwrap().history.undo.len();
    run(&mut s, "edit.copy", json!({}));
    // The clipboard holds the member as an ordinary path with the compound's paint, and the
    // piece extracted from the standalone path.
    let clip_ids: Vec<NodeId> = s.clipboard.nodes.iter().map(|n| n.id).collect();
    assert_eq!(clip_ids.len(), 2, "the member plus the extracted standalone piece");
    assert!(clip_ids.contains(&outer), "the member is copied itself: {clip_ids:?}");
    assert!(!clip_ids.contains(&compound), "the compound is not copied");
    let member = s.clipboard.nodes.iter().find(|n| n.id == outer).unwrap();
    assert!(matches!(member.kind, NodeKind::Path { .. }));
    assert_eq!(member.appearance, s.doc().unwrap().doc.node(compound).unwrap().appearance);
    // Copy leaves the document and its history as they were.
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_before);
    assert_eq!(s.doc().unwrap().doc.node(compound).unwrap().children().unwrap().len(), 2);
}

#[test]
fn cut_whole_compound_member_with_direct_selection_cuts_member_and_extracted_piece() {
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

    let undo_before = s.doc().unwrap().history.undo.len();
    run(&mut s, "edit.cut", json!({}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_before + 1, "Cut is one undo step");
    let clip_ids: Vec<NodeId> = s.clipboard.nodes.iter().map(|n| n.id).collect();
    assert_eq!(clip_ids.len(), 2, "the member plus the extracted standalone piece");
    assert!(clip_ids.contains(&outer) && !clip_ids.contains(&compound), "{clip_ids:?}");

    // The member left; the other member stays in the compound.
    let doc = &s.doc().unwrap().doc;
    assert!(doc.node(outer).is_none(), "the cut member is removed");
    assert_eq!(doc.node(compound).and_then(|n| n.children()).map(|c| c.iter().map(|m| m.id).collect::<Vec<_>>()), Some(vec![inner]));
    // Standalone has cut pieces remaining
    assert!(doc.node(standalone).is_some(), "partially cut standalone path remains");
    check_session(&s).unwrap();

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().doc.node(compound).unwrap().children().unwrap().len(), 2, "Undo puts the member back");
    run(&mut s, "edit.redo", json!({}));
    assert!(s.doc().unwrap().doc.node(outer).is_none());
    check_session(&s).unwrap();
}

/// A compound path of `n` rectangles side by side (x = 10, 60, 110, …), filled red, with its id
/// and its members' ids.
fn compound_of(s: &mut Session, n: usize) -> (NodeId, Vec<NodeId>) {
    let members: Vec<NodeId> = (0..n).map(|i| rect(s, 10.0 + 50.0 * i as f64, 10.0, 40.0, 40.0)).collect();
    run(s, "select.set", json!({"ids": members.iter().map(|m| m.0).collect::<Vec<_>>()}));
    let compound = NodeId(run(s, "object.compoundPath.make", json!({}))["id"].as_u64().unwrap());
    run(s, "paint.setFill", json!({"color": "#ff0000"}));
    (compound, members)
}

/// Every anchor of the four-anchor rectangle `id` picked with Direct Selection, which selects it
/// whole.
fn pick_all_anchors(s: &mut Session, id: NodeId) {
    run(s, "select.anchorsMany", json!({"items": [{"id": id.0, "anchors": [[0, 0], [0, 1], [0, 2], [0, 3]]}]}));
    assert!(s.doc().unwrap().selection.contains(id) && s.doc().unwrap().selection.partial(id).is_none(), "the member is selected whole");
}

fn members_of(s: &Session, compound: NodeId) -> Option<Vec<NodeId>> {
    s.doc().unwrap().doc.node(compound).and_then(|n| n.children()).map(|c| c.iter().map(|m| m.id).collect())
}

#[test]
fn cut_of_one_member_of_two_leaves_a_compound_of_one() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let (compound, m) = compound_of(&mut s, 2);
    pick_all_anchors(&mut s, m[0]);
    let undo_before = s.doc().unwrap().history.undo.len();
    run(&mut s, "edit.cut", json!({}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_before + 1, "Cut is one undo step");
    assert_eq!(members_of(&s, compound), Some(vec![m[1]]), "the other member stays a compound of one, as after Clear");
    assert_eq!(s.clipboard.nodes.len(), 1);
    assert_eq!(s.clipboard.nodes[0].id, m[0]);
    assert!(matches!(s.clipboard.nodes[0].kind, NodeKind::Path { .. }), "the clipboard holds an ordinary path");
    check_session(&s).unwrap();

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(members_of(&s, compound), Some(m.clone()), "Undo puts the member back in its place");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(members_of(&s, compound), Some(vec![m[1]]));
    check_session(&s).unwrap();
}

#[test]
fn cut_of_the_middle_member_of_three_leaves_the_other_two() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let (compound, m) = compound_of(&mut s, 3);
    pick_all_anchors(&mut s, m[1]);
    run(&mut s, "edit.cut", json!({}));
    assert_eq!(members_of(&s, compound), Some(vec![m[0], m[2]]));
    assert_eq!(s.clipboard.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), [m[1]]);
    check_session(&s).unwrap();
}

#[test]
fn clear_of_a_whole_member_removes_only_the_member_and_keeps_the_clipboard() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let (compound, m) = compound_of(&mut s, 2);
    let standalone = rect(&mut s, 10.0, 120.0, 40.0, 40.0);
    run(&mut s, "select.set", json!({"ids": [standalone.0]}));
    run(&mut s, "edit.copy", json!({}));
    pick_all_anchors(&mut s, m[0]);
    run(&mut s, "edit.clear", json!({}));
    assert_eq!(members_of(&s, compound), Some(vec![m[1]]));
    assert_eq!(s.clipboard.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), [standalone]);
}

#[test]
fn group_selection_of_one_member_copies_and_cuts_only_that_member() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let (compound, m) = compound_of(&mut s, 2);
    s.select_tool("groupSelection", vectorcraft_engine::ViewInfo::default()).unwrap();
    run(&mut s, "select.set", json!({"ids": [m[1].0]}));
    run(&mut s, "edit.copy", json!({}));
    assert_eq!(s.clipboard.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), [m[1]]);
    assert!(matches!(s.clipboard.nodes[0].kind, NodeKind::Path { .. }));
    assert_eq!(members_of(&s, compound), Some(m.clone()), "Copy leaves both members");

    run(&mut s, "edit.cut", json!({}));
    assert_eq!(members_of(&s, compound), Some(vec![m[0]]));
    assert_eq!(s.clipboard.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), [m[1]]);
    check_session(&s).unwrap();
}

#[test]
fn every_member_selected_whole_copies_and_cuts_the_compound() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let (compound, m) = compound_of(&mut s, 2);
    s.select_tool("groupSelection", vectorcraft_engine::ViewInfo::default()).unwrap();
    run(&mut s, "select.set", json!({"ids": [m[0].0, m[1].0]}));
    run(&mut s, "edit.copy", json!({}));
    assert_eq!(s.clipboard.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), [compound]);
    run(&mut s, "edit.cut", json!({}));
    assert!(s.doc().unwrap().doc.node(compound).is_none(), "the compound leaves whole");
    assert_eq!(s.clipboard.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), [compound]);
    check_session(&s).unwrap();
}

#[test]
fn a_copied_member_keeps_the_compounds_transparency_mask_and_fill_rule() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let (compound, m) = compound_of(&mut s, 2);
    let mask = rect(&mut s, 0.0, 0.0, 120.0, 60.0);
    run(&mut s, "select.set", json!({"ids": [compound.0, mask.0]}));
    run(&mut s, "transparency.makeOpacityMask", json!({}));
    let mask_art = s.doc().unwrap().doc.node(compound).unwrap().extras().mask.clone();
    assert!(mask_art.is_some(), "the compound has an opacity mask");
    run(&mut s, "select.set", json!({"ids": [compound.0]}));
    run(&mut s, "transparency.set", json!({"opacity": 50, "blend": "Multiply"}));
    run(&mut s, "path.setFillRule", json!({"rule": "evenOdd"}));
    assert!(matches!(s.doc().unwrap().doc.node(m[0]).unwrap().kind, NodeKind::Path { rule: FillRule::NonZero, .. }), "the member keeps its own rule");
    let (opacity, blend) = {
        let c = s.doc().unwrap().doc.node(compound).unwrap();
        (c.opacity, c.blend)
    };
    assert_eq!(opacity, 0.5);
    pick_all_anchors(&mut s, m[0]);
    run(&mut s, "edit.copy", json!({}));
    assert_eq!(s.clipboard.nodes.iter().map(|n| (n.id, n.opacity, n.blend)).collect::<Vec<_>>(), [(m[0], opacity, blend)]);
    assert!(matches!(s.clipboard.nodes[0].kind, NodeKind::Path { rule: FillRule::EvenOdd, .. }));
    assert_eq!(s.clipboard.nodes[0].extras().mask, mask_art, "the member keeps the compound's opacity mask");
}

#[test]
fn a_copied_member_with_its_own_appearance_takes_the_compounds() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let (compound, m) = compound_of(&mut s, 2);
    // The member carries an appearance of its own, as an imported member can; the compound's
    // appearance is the one drawn.
    run(&mut s, "paint.setFill", json!({"ids": [compound.0], "color": "#00ff00"}));
    let own_look = s.doc().unwrap().doc.node(compound).unwrap().appearance.clone();
    run(&mut s, "paint.setFill", json!({"ids": [compound.0], "color": "#0000ff"}));
    let compound_look = s.doc().unwrap().doc.node(compound).unwrap().appearance.clone();
    assert_ne!(own_look, compound_look);
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(m[0]).unwrap().appearance = own_look.clone();
    assert!(!s.doc().unwrap().doc.node(m[0]).unwrap().appearance.items.is_empty(), "the member has its own appearance");
    pick_all_anchors(&mut s, m[0]);
    run(&mut s, "edit.copy", json!({}));
    assert_eq!(s.clipboard.nodes[0].appearance, compound_look);
}

#[test]
fn every_segment_of_every_member_picked_copies_and_cuts_the_compound() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let (compound, m) = compound_of(&mut s, 2);
    let all: Vec<Vec<usize>> = (0..4).map(|i| vec![0, i]).collect();
    run(&mut s, "select.segments", json!({"id": m[0].0, "segments": all}));
    run(&mut s, "select.segments", json!({"id": m[1].0, "segments": all, "mode": "add"}));
    let sel = &s.doc().unwrap().selection;
    assert!(sel.segments.contains_key(&m[0]) && sel.segments.contains_key(&m[1]), "both members keep their picked segments");
    run(&mut s, "edit.copy", json!({}));
    assert_eq!(s.clipboard.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), [compound]);
    run(&mut s, "edit.cut", json!({}));
    assert!(s.doc().unwrap().doc.node(compound).is_none(), "the compound leaves whole");
    assert_eq!(s.clipboard.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), [compound]);
    check_session(&s).unwrap();
}

#[test]
fn cut_of_a_whole_member_with_anchors_of_another_member_takes_each_by_its_own_rule() {
    let after = |command: &str| {
        let mut s = Session::new();
        run(&mut s, "file.new", json!({"width": 200, "height": 200}));
        let (compound, m) = compound_of(&mut s, 2);
        let whole: Vec<Vec<usize>> = (0..4).map(|i| vec![0, i]).collect();
        run(&mut s, "select.anchorsMany", json!({"items": [{"id": m[0].0, "anchors": whole}, {"id": m[1].0, "anchors": [[0, 1], [0, 2]]}]}));
        run(&mut s, command, json!({}));
        check_session(&s).unwrap();
        (s, compound, m)
    };
    let (s, compound, m) = after("edit.cut");
    // The whole member and the segment between the two picked anchors went to the clipboard.
    assert_eq!(s.clipboard.nodes.len(), 2);
    assert!(s.clipboard.nodes.iter().any(|n| n.id == m[0]));
    assert!(s.clipboard.nodes.iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
    // The partly picked member stays in the compound, with what Clear leaves of its outline.
    assert_eq!(members_of(&s, compound).map(|c| c.len()), Some(1));
    assert_eq!(contours(&s), contours(&after("edit.clear").0));
    assert_eq!(contours(&s).len(), 1);
}

#[test]
fn cut_of_a_member_of_a_compound_inside_a_group_leaves_the_group_and_compound() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let (compound, m) = compound_of(&mut s, 2);
    let other = rect(&mut s, 10.0, 120.0, 40.0, 40.0);
    run(&mut s, "select.set", json!({"ids": [compound.0, other.0]}));
    let group = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    pick_all_anchors(&mut s, m[0]);
    run(&mut s, "edit.cut", json!({}));
    let doc = &s.doc().unwrap().doc;
    assert_eq!(doc.parent_of(compound), Some(group));
    assert_eq!(members_of(&s, compound), Some(vec![m[1]]));
    assert_eq!(s.clipboard.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), [m[0]]);
    check_session(&s).unwrap();
}

#[test]
fn paste_in_place_and_in_front_of_a_cut_member_paste_an_ordinary_path() {
    for command in ["edit.pasteInPlace", "edit.pasteInFront"] {
        let mut s = Session::new();
        run(&mut s, "file.new", json!({"width": 200, "height": 200}));
        let (compound, m) = compound_of(&mut s, 2);
        let fill = s.doc().unwrap().doc.node(compound).unwrap().appearance.clone();
        pick_all_anchors(&mut s, m[0]);
        run(&mut s, "edit.cut", json!({}));
        let pasted = run(&mut s, command, json!({}));
        let ids = pasted["ids"].as_array().unwrap();
        assert_eq!(ids.len(), 1, "{command}");
        let id = NodeId(ids[0].as_u64().unwrap());
        let doc = &s.doc().unwrap().doc;
        let node = doc.node(id).unwrap();
        assert!(matches!(node.kind, NodeKind::Path { .. }), "{command}");
        assert_eq!(node.appearance, fill, "{command}: the path keeps the compound's paint");
        assert!(doc.parent_of(id).and_then(|p| doc.node(p)).is_some_and(|p| p.is_layer()), "{command}: pasted into the layer");
        let anchors: Vec<[f64; 2]> = node.path_data().unwrap().subpaths[0].anchors.iter().map(|a| [a.p.x, a.p.y]).collect();
        assert!(anchors.contains(&[10.0, 10.0]) && anchors.contains(&[50.0, 50.0]), "{command}: where it was: {anchors:?}");
        assert_eq!(members_of(&s, compound), Some(vec![m[1]]), "{command}");
        check_session(&s).unwrap();
    }
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

#[test]
fn copy_and_paste_direct_selected_segment_of_open_path() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let id = NodeId(run(&mut s, "path.create", json!({"anchors": [{"x": 10, "y": 20}]}))["id"].as_u64().unwrap());
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 60, "y": 20}));
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 100, "y": 80}));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "paint.setStroke", json!({"color": "#0000ff"}));
    run(&mut s, "stroke.set", json!({"weight": 4.0}));

    // Select segment 0 of the open path (from (10, 20) to (60, 20)).
    run(&mut s, "select.segments", json!({"id": id.0, "segments": [[0, 0]]}));
    run(&mut s, "edit.copy", json!({}));

    // Paste in place: position and appearance must be preserved.
    let pasted = run(&mut s, "edit.pasteInPlace", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 1);

    let pasted_node = s.doc().unwrap().doc.node(NodeId(ids[0])).unwrap();
    let src_node = s.doc().unwrap().doc.node(id).unwrap();
    assert_eq!(pasted_node.appearance, src_node.appearance, "appearance (stroke & fill) must match source");

    let path = pasted_node.path_data().unwrap();
    assert_eq!(path.subpaths.len(), 1);
    let sp = &path.subpaths[0];
    assert!(!sp.closed);
    assert_eq!(sp.segment_count(), 1);
    assert_eq!(sp.anchors.len(), 2);
    assert_eq!(sp.anchors[0].p.x, 10.0);
    assert_eq!(sp.anchors[0].p.y, 20.0);
    assert_eq!(sp.anchors[1].p.x, 60.0);
    assert_eq!(sp.anchors[1].p.y, 20.0);

    check_session(&s).unwrap();
}

#[test]
fn copy_two_non_adjacent_segments_of_open_path_pastes_two_paths() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 300, "height": 200}));
    let id = NodeId(run(&mut s, "path.create", json!({"anchors": [{"x": 10, "y": 10}]}))["id"].as_u64().unwrap());
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 40, "y": 10}));
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 80, "y": 10}));
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 120, "y": 10}));
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 160, "y": 10}));

    // Path has 5 anchors (0..4) and 4 segments (0..3).
    // Select disjoint/non-adjacent segments 0 and 2.
    run(&mut s, "select.segments", json!({"id": id.0, "segments": [[0, 0], [0, 2]]}));
    run(&mut s, "edit.copy", json!({}));
    let pasted = run(&mut s, "edit.paste", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 2, "two non-adjacent segments paste as two independent paths");

    for node_id in ids {
        let node = s.doc().unwrap().doc.node(NodeId(node_id)).unwrap();
        let path = node.path_data().unwrap();
        assert_eq!(path.subpaths.len(), 1);
        assert!(!path.subpaths[0].closed);
        assert_eq!(path.subpaths[0].segment_count(), 1);
        assert_eq!(path.subpaths[0].anchors.len(), 2);
    }

    check_session(&s).unwrap();
}

#[test]
fn copy_all_anchors_of_open_path_copies_whole_path() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let id = NodeId(run(&mut s, "path.create", json!({"anchors": [{"x": 10, "y": 10}]}))["id"].as_u64().unwrap());
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 50, "y": 10}));
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 80, "y": 40}));

    // Select all 3 anchors.
    run(&mut s, "select.anchors", json!({"id": id.0, "anchors": [[0, 0], [0, 1], [0, 2]], "mode": "set"}));
    run(&mut s, "edit.copy", json!({}));
    let pasted = run(&mut s, "edit.paste", json!({}));
    let ids: Vec<u64> = pasted["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 1);

    let pasted_node = s.doc().unwrap().doc.node(NodeId(ids[0])).unwrap();
    let path = pasted_node.path_data().unwrap();
    assert_eq!(path.subpaths.len(), 1);
    assert!(!path.subpaths[0].closed);
    assert_eq!(path.subpaths[0].segment_count(), 2);
    assert_eq!(path.subpaths[0].anchors.len(), 3);

    check_session(&s).unwrap();
}

#[test]
fn copy_single_anchor_on_open_path_and_lone_point_states_what_is_pasted() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));

    // 1. Single interior anchor of an open path:
    // Path: (10, 10) -> (50, 10) -> (90, 50).
    // Selecting interior anchor 1 selects both incident segments (0 and 1).
    // Stated behavior: pasting pastes an open path containing the anchor and both its incident segments (3 anchors, 2 segments).
    let p1 = NodeId(run(&mut s, "path.create", json!({"anchors": [{"x": 10, "y": 10}]}))["id"].as_u64().unwrap());
    run(&mut s, "path.appendAnchor", json!({"id": p1.0, "x": 50, "y": 10}));
    run(&mut s, "path.appendAnchor", json!({"id": p1.0, "x": 90, "y": 50}));
    run(&mut s, "select.anchors", json!({"id": p1.0, "anchors": [[0, 1]], "mode": "set"}));
    run(&mut s, "edit.copy", json!({}));
    let pasted1 = run(&mut s, "edit.paste", json!({}));
    let ids1: Vec<u64> = pasted1["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids1.len(), 1);
    let path1 = s.doc().unwrap().doc.node(NodeId(ids1[0])).unwrap().path_data().unwrap();
    assert!(!path1.subpaths[0].closed);
    assert_eq!(path1.subpaths[0].anchors.len(), 3);
    assert_eq!(path1.subpaths[0].segment_count(), 2);

    // 2. Single endpoint anchor of an open path:
    // Selecting endpoint anchor 0 selects its single incident segment (segment 0).
    // Stated behavior: pasting pastes an open path for that segment (2 anchors, 1 segment).
    run(&mut s, "select.anchors", json!({"id": p1.0, "anchors": [[0, 0]], "mode": "set"}));
    run(&mut s, "edit.copy", json!({}));
    let pasted2 = run(&mut s, "edit.paste", json!({}));
    let ids2: Vec<u64> = pasted2["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids2.len(), 1);
    let path2 = s.doc().unwrap().doc.node(NodeId(ids2[0])).unwrap().path_data().unwrap();
    assert!(!path2.subpaths[0].closed);
    assert_eq!(path2.subpaths[0].anchors.len(), 2);
    assert_eq!(path2.subpaths[0].segment_count(), 1);

    // 3. A lone point path (a path with only 1 anchor and 0 segments):
    // Selecting the lone anchor on a single-point path copies the object whole.
    // Stated behavior: pasting pastes a standalone lone-point path (1 anchor, 0 segments).
    let p_lone = NodeId(run(&mut s, "path.create", json!({"anchors": [{"x": 100, "y": 100}]}))["id"].as_u64().unwrap());
    run(&mut s, "select.anchors", json!({"id": p_lone.0, "anchors": [[0, 0]], "mode": "set"}));
    run(&mut s, "edit.copy", json!({}));
    let pasted_lone = run(&mut s, "edit.paste", json!({}));
    let ids_lone: Vec<u64> = pasted_lone["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids_lone.len(), 1);
    let path_lone = s.doc().unwrap().doc.node(NodeId(ids_lone[0])).unwrap().path_data().unwrap();
    assert_eq!(path_lone.subpaths[0].anchors.len(), 1);
    assert_eq!(path_lone.subpaths[0].segment_count(), 0);

    check_session(&s).unwrap();
}

#[test]
fn cut_anchor_of_closed_path_opens_it_with_single_undo_step() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let r = rect(&mut s, 10.0, 10.0, 100.0, 50.0);

    // Direct-select anchor 1 of the closed 4-anchor rectangle.
    run(&mut s, "select.anchors", json!({"id": r.0, "anchors": [[0, 1]], "mode": "set"}));
    let undo_before = s.doc().unwrap().history.undo.len();

    // Cut the anchor.
    run(&mut s, "edit.cut", json!({}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_before + 1, "cut is recorded as one undo step");

    // Source rectangle opens: 1 open subpath with 3 anchors remaining.
    let src_node = s.doc().unwrap().doc.node(r).unwrap();
    let src_path = src_node.path_data().unwrap();
    assert_eq!(src_path.subpaths.len(), 1);
    assert!(!src_path.subpaths[0].closed, "cutting anchor of closed path opens the path");
    assert_eq!(src_path.subpaths[0].anchors.len(), 3);
    assert_eq!(src_path.subpaths[0].segment_count(), 2);

    // Clipboard holds the cut portion (the anchor and its two incident segments: 3 anchors).
    assert_eq!(s.clipboard.nodes.len(), 1);
    let clip_path = s.clipboard.nodes[0].path_data().unwrap();
    assert!(!clip_path.subpaths[0].closed);
    assert_eq!(clip_path.subpaths[0].anchors.len(), 3);
    assert_eq!(clip_path.subpaths[0].segment_count(), 2);

    // Verify undo restores the closed path in one step.
    run(&mut s, "edit.undo", json!({}));
    let restored_node = s.doc().unwrap().doc.node(r).unwrap();
    let restored_path = restored_node.path_data().unwrap();
    assert_eq!(restored_path.subpaths.len(), 1);
    assert!(restored_path.subpaths[0].closed, "undo restores closed path");
    assert_eq!(restored_path.subpaths[0].anchors.len(), 4);
    assert_eq!(restored_path.subpaths[0].segment_count(), 4);

    // Verify redo re-applies the cut.
    run(&mut s, "edit.redo", json!({}));
    let redone_node = s.doc().unwrap().doc.node(r).unwrap();
    let redone_path = redone_node.path_data().unwrap();
    assert!(!redone_path.subpaths[0].closed);
    assert_eq!(redone_path.subpaths[0].anchors.len(), 3);

    check_session(&s).unwrap();
}

/// Cutting an interior connection copies that segment and leaves two selected sibling objects.
#[test]
fn cut_middle_segment_of_open_path_splits_into_two_open_paths_with_single_undo_step() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let id = NodeId(run(&mut s, "path.create", json!({"anchors": [{"x": 10, "y": 10}]}))["id"].as_u64().unwrap());
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 50, "y": 10}));
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 90, "y": 10}));
    run(&mut s, "path.appendAnchor", json!({"id": id.0, "x": 130, "y": 10}));

    // Open path has 4 anchors (0, 1, 2, 3) and 3 segments (0, 1, 2).
    // Direct-select middle segment 1 (between anchor 1 and 2).
    run(&mut s, "select.segments", json!({"id": id.0, "segments": [[0, 1]]}));
    let undo_before = s.doc().unwrap().history.undo.len();

    // Cut the middle segment.
    run(&mut s, "edit.cut", json!({}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_before + 1, "cut is recorded as one undo step");

    // Source path should now be split into two surviving open runs.
    let src_node = s.doc().unwrap().doc.node(id).unwrap();
    let src_path = src_node.path_data().unwrap();
    assert_eq!(src_path.subpaths.len(), 1);
    assert!(!src_path.subpaths[0].closed);
    assert_eq!(src_path.subpaths[0].anchors.len(), 2);
    assert_eq!(src_path.subpaths[0].anchors[0].p.x, 10.0);
    assert_eq!(src_path.subpaths[0].anchors[1].p.x, 50.0);

    let pieces = s.doc().unwrap().selection.objects.clone();
    assert_eq!(pieces.len(), 2, "two independent path objects remain after cutting the middle segment");
    assert_eq!(pieces[0], id);
    let second = &s.doc().unwrap().doc.node(pieces[1]).unwrap().path_data().unwrap().subpaths[0];
    assert!(!second.closed);
    assert_eq!(second.anchors.len(), 2);
    assert_eq!(second.anchors[0].p.x, 90.0);
    assert_eq!(second.anchors[1].p.x, 130.0);

    // Clipboard holds the cut middle segment:
    assert_eq!(s.clipboard.nodes.len(), 1);
    let clip_path = s.clipboard.nodes[0].path_data().unwrap();
    assert_eq!(clip_path.subpaths[0].anchors.len(), 2);
    assert_eq!(clip_path.subpaths[0].anchors[0].p.x, 50.0);
    assert_eq!(clip_path.subpaths[0].anchors[1].p.x, 90.0);

    // Verify undo restores the original intact path in one step:
    run(&mut s, "edit.undo", json!({}));
    let restored_node = s.doc().unwrap().doc.node(id).unwrap();
    let restored_path = restored_node.path_data().unwrap();
    assert_eq!(restored_path.subpaths.len(), 1, "undo restores single subpath");
    assert_eq!(restored_path.subpaths[0].anchors.len(), 4, "undo restores all 4 anchors");

    // Redo re-applies the split:
    run(&mut s, "edit.redo", json!({}));
    let redone_path = s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap();
    assert_eq!(redone_path.subpaths.len(), 1);
    assert_eq!(s.doc().unwrap().selection.objects, pieces);
    assert!(s.doc().unwrap().doc.node(pieces[1]).is_some());

    check_session(&s).unwrap();
}
