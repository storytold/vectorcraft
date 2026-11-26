//! General › Delete Removes Anchor Points and Keeps Paths Joined (`deleteKeepsPathsJoined`). With
//! it on, Delete (`edit.clear`) removes the direct-selected anchors of a path as Remove Anchor
//! Points does, without opening the path. Off (the default), Delete removes them with their
//! segments.

use serde_json::json;
use vectorcraft_geom::{PathData, Point};

use super::history;
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn keep_joined(s: &mut Session, on: bool) {
    s.execute("prefs.set", &json!({"key": "deleteKeepsPathsJoined", "value": on})).unwrap();
}

fn path_of(s: &Session, id: NodeId) -> Option<PathData> {
    s.doc().unwrap().doc.node(id).and_then(|n| n.path_data().cloned())
}

fn points(s: &Session, id: NodeId) -> Vec<Point> {
    path_of(s, id).unwrap().subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p)).collect()
}

fn create(s: &mut Session, anchors: serde_json::Value, closed: bool) -> NodeId {
    NodeId(s.execute("path.create", &json!({ "anchors": anchors, "closed": closed })).unwrap()["id"].as_u64().unwrap())
}

fn square(s: &mut Session, x: f64) -> NodeId {
    create(s, json!([{"x": x, "y": 100}, {"x": x + 100.0, "y": 100}, {"x": x + 100.0, "y": 200}, {"x": x, "y": 200}]), true)
}

fn line(s: &mut Session, y: f64) -> NodeId {
    create(s, json!([{"x": 100, "y": y}, {"x": 150, "y": y - 20.0}, {"x": 200, "y": y}, {"x": 250, "y": y + 20.0}, {"x": 300, "y": y}]), false)
}

fn select(s: &mut Session, id: NodeId, anchors: serde_json::Value, mode: &str) {
    s.execute("select.anchors", &json!({"id": id.0, "anchors": anchors, "mode": mode})).unwrap();
}

fn objects(s: &Session) -> usize {
    s.doc().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, |c| c.len())).sum()
}

#[test]
fn off_by_default_delete_opens_a_closed_path() {
    let mut s = session();
    assert!(!s.prefs.delete_keeps_paths_joined);
    let id = square(&mut s, 100.0);
    select(&mut s, id, json!([[0, 1]]), "set");
    s.execute("edit.clear", &json!({})).unwrap();
    let path = path_of(&s, id).unwrap();
    assert!(!path.subpaths[0].closed, "the path opens where the anchor was");
    assert_eq!(path.subpaths[0].anchors.len(), 3);
}

#[test]
fn with_the_preference_on_delete_keeps_a_closed_path_closed_and_selects_the_next_anchor() {
    let mut s = session();
    keep_joined(&mut s, true);
    let id = square(&mut s, 100.0);
    select(&mut s, id, json!([[0, 1]]), "set");
    let before = history(&s).0;
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!(history(&s).0, before + 1, "one undo step");
    let path = path_of(&s, id).unwrap();
    assert_eq!(path.subpaths.len(), 1);
    assert!(path.subpaths[0].closed, "the path stays closed");
    assert!(!points(&s, id).contains(&Point::new(200.0, 100.0)), "the selected anchor is gone");
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects, vec![id], "the path stays selected");
    assert_eq!(st.selection.anchors[&id], [(0, 1)].into(), "the anchor after the removed one is selected");
    // A second Delete goes on along the path.
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!(points(&s, id), vec![Point::new(100.0, 100.0), Point::new(100.0, 200.0)]);
}

#[test]
fn an_open_path_loses_an_end_anchor_and_an_interior_anchor_without_splitting() {
    let mut s = session();
    keep_joined(&mut s, true);
    let id = line(&mut s, 300.0);
    select(&mut s, id, json!([[0, 0], [0, 2]]), "set");
    s.execute("edit.clear", &json!({})).unwrap();
    let path = path_of(&s, id).unwrap();
    assert_eq!(path.subpaths.len(), 1, "one piece");
    assert!(!path.subpaths[0].closed);
    assert_eq!(
        points(&s, id),
        vec![Point::new(150.0, 280.0), Point::new(250.0, 320.0), Point::new(300.0, 300.0)],
        "the path is shorter at its start"
    );
    assert_eq!(objects(&s), 1, "no new object");
    assert_eq!(s.doc().unwrap().selection.anchors[&id], [(0, 1)].into(), "the anchor after the last removed one is selected");
    // Removing the last anchor of an open path selects the one before it.
    select(&mut s, id, json!([[0, 2]]), "set");
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!(points(&s, id), vec![Point::new(150.0, 280.0), Point::new(250.0, 320.0)], "the path is shorter at its end");
    assert_eq!(s.doc().unwrap().selection.anchors[&id], [(0, 1)].into(), "the new end anchor is selected");
}

#[test]
fn a_subpath_left_with_one_anchor_goes_and_the_path_stays_selected_whole() {
    let mut s = session();
    keep_joined(&mut s, true);
    let id = square(&mut s, 100.0);
    let sq = json!({"closed": true, "anchors": [{"x": 100, "y": 100}, {"x": 200, "y": 100}, {"x": 200, "y": 200}, {"x": 100, "y": 200}]});
    let pair = json!({"closed": false, "anchors": [{"x": 300, "y": 100}, {"x": 400, "y": 100}]});
    s.execute("path.setAnchors", &json!({"id": id.0, "subpaths": [sq, pair]})).unwrap();
    select(&mut s, id, json!([[1, 0]]), "set");
    s.execute("edit.clear", &json!({})).unwrap();
    let path = path_of(&s, id).unwrap();
    assert_eq!(path.subpaths.len(), 1, "the subpath left with one anchor goes");
    assert!(path.subpaths[0].closed && path.subpaths[0].anchors.len() == 4, "the other subpath is untouched");
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects, vec![id], "the path stays selected");
    assert!(st.selection.anchors.get(&id).is_none_or(|a| a.is_empty()), "no anchor is left to go on from");
}

#[test]
fn a_path_left_with_fewer_than_two_anchors_is_deleted() {
    let mut s = session();
    keep_joined(&mut s, true);
    let id = create(&mut s, json!([{"x": 100, "y": 100}, {"x": 200, "y": 100}]), false);
    let other = square(&mut s, 400.0);
    select(&mut s, id, json!([[0, 1]]), "set");
    select(&mut s, other, json!([[0, 0]]), "add");
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(path_of(&s, id).is_none(), "a single anchor is no path");
    assert!(path_of(&s, other).unwrap().subpaths[0].closed, "the other path keeps its other anchors joined");
    assert_eq!(s.doc().unwrap().selection.objects, vec![other]);
}

#[test]
fn several_paths_and_an_object_selected_whole_change_in_one_undo_step() {
    let mut s = session();
    keep_joined(&mut s, true);
    let (a, b, whole) = (square(&mut s, 100.0), line(&mut s, 400.0), square(&mut s, 500.0));
    select(&mut s, a, json!([[0, 0]]), "set");
    select(&mut s, b, json!([[0, 1], [0, 3]]), "add");
    s.execute("select.add", &json!({"ids": [whole.0]})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let before = history(&s).0;
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!(history(&s).0, before + 1, "one undo step");
    assert!(path_of(&s, whole).is_none(), "the object selected whole is deleted");
    assert!(path_of(&s, a).unwrap().subpaths[0].closed);
    assert_eq!(path_of(&s, a).unwrap().subpaths[0].anchors.len(), 3);
    assert_eq!(points(&s, b), vec![Point::new(100.0, 400.0), Point::new(200.0, 400.0), Point::new(300.0, 400.0)]);
    let st = s.doc().unwrap();
    assert!(st.selection.anchors.contains_key(&a) && st.selection.anchors.contains_key(&b), "both paths keep an anchor selected");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, doc, "Undo restores every path");
}

#[test]
fn a_compound_path_keeps_its_members_and_loses_one_left_with_no_segment() {
    let mut s = session();
    keep_joined(&mut s, true);
    let (outer, inner) = (square(&mut s, 100.0), create(&mut s, json!([{"x": 130, "y": 150}, {"x": 170, "y": 150}]), false));
    s.execute("select.set", &json!({"ids": [outer.0, inner.0]})).unwrap();
    let compound = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    select(&mut s, outer, json!([[0, 2]]), "set");
    select(&mut s, inner, json!([[0, 0]]), "add");
    s.execute("edit.clear", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.node(inner).is_none(), "the member left with one anchor goes");
    assert_eq!(d.parent_of(outer), Some(compound), "the other member stays in the compound path");
    assert!(path_of(&s, outer).unwrap().subpaths[0].closed);
    assert_eq!(path_of(&s, outer).unwrap().subpaths[0].anchors.len(), 3);
}

#[test]
fn a_selected_segment_is_still_cut() {
    let mut s = session();
    keep_joined(&mut s, true);
    let id = square(&mut s, 100.0);
    s.execute("select.segments", &json!({"id": id.0, "segments": [[0, 0]]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    let path = path_of(&s, id).unwrap();
    assert!(!path.subpaths[0].closed, "the segment's connection is removed");
    assert_eq!(path.subpaths[0].anchors.len(), 4, "every anchor stays");
}

#[test]
fn the_keep_joined_param_overrides_the_preference_and_must_be_a_boolean() {
    let mut s = session();
    let id = square(&mut s, 100.0);
    select(&mut s, id, json!([[0, 1]]), "set");
    assert!(s.execute("edit.clear", &json!({"keepJoined": "yes"})).is_err());
    assert_eq!(path_of(&s, id).unwrap().subpaths[0].anchors.len(), 4, "a refused call changes nothing");
    s.execute("edit.clear", &json!({"keepJoined": true})).unwrap();
    assert!(path_of(&s, id).unwrap().subpaths[0].closed);
    keep_joined(&mut s, true);
    s.execute("edit.clear", &json!({"keepJoined": false})).unwrap();
    assert!(!path_of(&s, id).unwrap().subpaths[0].closed, "false opens the path with the preference on");
}

#[test]
fn a_replay_removes_anchors_as_the_recorded_step_did() {
    let mut s = session();
    keep_joined(&mut s, true);
    let id = square(&mut s, 100.0);
    select(&mut s, id, json!([[0, 1]]), "set");
    let start = s.journal.len();
    s.execute("edit.clear", &json!({})).unwrap();
    let removed = path_of(&s, id);
    assert!(removed.as_ref().is_some_and(|p| p.subpaths[0].closed), "the path stays closed");
    let (cmd, params) = s.journal.since(start).last().cloned().unwrap();
    assert_eq!(params["keepJoined"], json!(true), "{cmd}: journaled");
    s.execute("edit.undo", &json!({})).unwrap();
    keep_joined(&mut s, false);
    select(&mut s, id, json!([[0, 1]]), "set");
    s.replay(&cmd, &params).unwrap();
    assert_eq!(path_of(&s, id), removed);
    // A step saved before the preference existed has no `keepJoined` and still opens the path.
    s.execute("edit.undo", &json!({})).unwrap();
    keep_joined(&mut s, true);
    select(&mut s, id, json!([[0, 1]]), "set");
    s.replay("edit.clear", &json!({})).unwrap();
    assert!(!path_of(&s, id).unwrap().subpaths[0].closed);
}
