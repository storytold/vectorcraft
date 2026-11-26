//! General › Double-Click Adds Anchor Points (`doubleClickAddsAnchorPoints`). With it on, a
//! Direct Selection double-click on a path segment adds an anchor there. Off (the default), the
//! double-click adds nothing.

use serde_json::json;
use vectorcraft_geom::kurbo::ParamCurveNearest;
use vectorcraft_geom::{ParamCurve, PathData, Point};
use vectorcraft_tools::{PointerEvent, PointerKind};

use super::history;
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn set_pref(s: &mut Session, key: &str, on: bool) {
    s.execute("prefs.set", &json!({"key": key, "value": on})).unwrap();
}

fn path_of(s: &Session, id: NodeId) -> Option<PathData> {
    s.doc().unwrap().doc.node(id).and_then(|n| n.path_data().cloned())
}

fn points(s: &Session, id: NodeId) -> Vec<Point> {
    path_of(s, id).unwrap().subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p)).collect()
}

/// Two clicks and the double-click that follows them, with Direct Selection, at `at`. On a segment
/// the clicks select it, so the double-click finds its path selected.
fn double_click(s: &mut Session, at: Point) {
    let v = ViewInfo::default();
    s.select_tool("directSelection", v).unwrap();
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick] {
        s.pointer(&PointerEvent::new(kind, at.x, at.y), v).unwrap();
    }
}

/// An open curve from (100, 300) to (300, 300) whose middle, at t = 0.5, is (200, 225).
fn arch(s: &mut Session) -> NodeId {
    let anchors = json!([{"x": 100, "y": 300, "out": [100, 200]}, {"x": 300, "y": 300, "in": [300, 200]}]);
    NodeId(s.execute("path.create", &json!({ "anchors": anchors, "closed": false })).unwrap()["id"].as_u64().unwrap())
}

#[test]
fn off_by_default() {
    let s = session();
    assert!(!s.prefs.double_click_adds_anchor_points);
}

#[test]
fn with_the_preference_on_a_double_click_on_a_segment_adds_an_anchor_in_one_undo_step() {
    // The clicks before the double-click select the segment either way; an unselected path at the
    // double-click is covered by the tools crate's tests.
    for selected in [true, false] {
        let mut s = session();
        set_pref(&mut s, "doubleClickAddsAnchorPoints", true);
        let id = arch(&mut s);
        if !selected {
            s.execute("select.none", &json!({})).unwrap();
        }
        let curve = path_of(&s, id).unwrap().subpaths[0].segment(0);
        let before = history(&s).0;
        double_click(&mut s, Point::new(200.0, 225.0));
        assert_eq!(history(&s).0, before + 1, "one undo step (selected before the clicks: {selected})");
        assert_eq!(points(&s, id).len(), 3, "an anchor is added (selected before the clicks: {selected})");
        assert!(points(&s, id)[1].distance(Point::new(200.0, 225.0)) < 0.5, "at the clicked point: {:?}", points(&s, id));
        let halves = path_of(&s, id).unwrap().subpaths[0].clone();
        for seg in 0..2 {
            for t in [0.25, 0.5, 0.75] {
                let q = ParamCurve::eval(&halves.segment(seg), t);
                let d = ParamCurveNearest::nearest(&curve, q, 1e-9).distance_sq;
                assert!(d < 1e-6, "the curve keeps its shape at {q:?}");
            }
        }
        let st = s.doc().unwrap();
        assert_eq!(st.selection.objects, vec![id]);
        assert_eq!(st.selection.anchors.get(&id), Some(&[(0, 1)].into()), "only the new anchor is selected (selected before the clicks: {selected})");
        assert!(st.selection.segments.get(&id).is_none_or(|s| s.is_empty()), "no segment stays selected");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(points(&s, id).len(), 2, "undo removes it");
    }
}

#[test]
fn with_the_preference_off_a_double_click_on_a_segment_adds_nothing() {
    let mut s = session();
    let id = arch(&mut s);
    let before = history(&s).0;
    double_click(&mut s, Point::new(200.0, 225.0));
    assert_eq!(history(&s).0, before);
    assert_eq!(points(&s, id).len(), 2);
}

/// The two General preferences are separate: Delete Removes Anchor Points and Keeps Paths Joined
/// does not turn on the double-click, and the double-click preference does not change Delete.
#[test]
fn the_double_click_and_delete_preferences_are_separate() {
    let mut s = session();
    set_pref(&mut s, "deleteKeepsPathsJoined", true);
    let id = arch(&mut s);
    let before = history(&s).0;
    double_click(&mut s, Point::new(200.0, 225.0));
    assert_eq!(history(&s).0, before, "deleteKeepsPathsJoined alone adds no anchor");
    assert_eq!(points(&s, id).len(), 2);

    let mut s = session();
    set_pref(&mut s, "doubleClickAddsAnchorPoints", true);
    let anchors = json!([{"x": 100, "y": 100}, {"x": 200, "y": 100}, {"x": 200, "y": 200}, {"x": 100, "y": 200}]);
    let id = NodeId(s.execute("path.create", &json!({ "anchors": anchors, "closed": true })).unwrap()["id"].as_u64().unwrap());
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 1]], "mode": "set"})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(!path_of(&s, id).unwrap().subpaths[0].closed, "Delete still opens the path");
}

#[test]
fn the_select_anchor_param_must_be_a_boolean() {
    let mut s = session();
    let id = arch(&mut s);
    assert!(s.execute("path.insertAnchor", &json!({"id": id.0, "segment": 0, "selectAnchor": "yes"})).is_err());
    assert_eq!(points(&s, id).len(), 2, "a refused call changes nothing");
}
