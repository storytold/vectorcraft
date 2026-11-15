//! The Shared Edge Merge Tool's command and Keep Shared Points Together: one undo step each, bad
//! params refused, the tool's click through the session, and only the add-on's commands affected.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::{PathData, Point};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use crate::{Session, ViewInfo};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn line(s: &mut Session, pts: &[(f64, f64)], closed: bool) -> NodeId {
    let anchors: Vec<Value> = pts.iter().map(|(x, y)| json!({"x": x, "y": y})).collect();
    NodeId(s.execute("path.create", &json!({"anchors": anchors, "closed": closed})).unwrap()["id"].as_u64().unwrap())
}

fn path(s: &Session, id: NodeId) -> PathData {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().clone()
}

fn steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn points(s: &Session, id: NodeId) -> Vec<Point> {
    path(s, id).anchors().map(|(_, _, a)| a.p).collect()
}

/// Two closed shapes with a bent edge each between (100, 100) and (200, 100).
fn shapes(s: &mut Session) -> (NodeId, NodeId) {
    let a = line(s, &[(100.0, 100.0), (150.0, 104.0), (200.0, 100.0), (200.0, 50.0), (100.0, 50.0)], true);
    let b = line(s, &[(200.0, 100.0), (150.0, 96.0), (100.0, 100.0), (100.0, 150.0), (200.0, 150.0)], true);
    (a, b)
}

#[test]
fn merge_edge_replaces_both_stretches_as_one_undo_step() {
    let mut s = session();
    let (a, b) = shapes(&mut s);
    s.execute("select.none", &json!({})).unwrap();
    let (pa, pb) = (path(&s, a), path(&s, b));
    let n = steps(&s);
    let r = s.execute("addon.pathtools.mergeEdge", &json!({"a": a.0, "b": b.0, "at": [125, 100]})).unwrap();
    assert_eq!(r, json!({"a": a.0, "b": b.0, "made": "edge"}));
    assert_eq!(steps(&s), n + 1);
    assert!(path(&s, a).subpaths[0].closed && path(&s, b).subpaths[0].closed);
    assert_eq!(points(&s, a)[..3], [Point::new(100.0, 100.0), Point::new(150.0, 100.0), Point::new(200.0, 100.0)]);
    assert_eq!(points(&s, b)[..3], [Point::new(200.0, 100.0), Point::new(150.0, 100.0), Point::new(100.0, 100.0)]);
    // Nothing left to do: no step.
    let r = s.execute("addon.pathtools.mergeEdge", &json!({"a": a.0, "b": b.0, "at": [125, 100]})).unwrap();
    assert_eq!(r["made"], json!("nothing"));
    assert_eq!(steps(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((path(&s, a), path(&s, b)), (pa, pb));
}

#[test]
fn merge_edge_follows_the_path_asked_and_makes_ends() {
    let mut s = session();
    let (a, b) = shapes(&mut s);
    let pa = path(&s, a);
    s.execute("addon.pathtools.mergeEdge", &json!({"a": a.0, "b": b.0, "at": [125, 100], "follow": "a"})).unwrap();
    assert_eq!(path(&s, a), pa);
    assert_eq!(points(&s, b)[1], Point::new(150.0, 104.0));
    // Ends asked for: the near anchors of two lines meet, one undo step.
    let c = line(&mut s, &[(10.0, 200.0), (50.0, 200.0)], false);
    let d = line(&mut s, &[(54.0, 200.0), (90.0, 240.0)], false);
    let n = steps(&s);
    let r = s.execute("addon.pathtools.mergeEdge", &json!({"a": c.0, "b": d.0, "at": [52, 200], "ends": true})).unwrap();
    assert_eq!((r["made"].clone(), steps(&s)), (json!("end"), n + 1));
    assert_eq!((points(&s, c)[1], points(&s, d)[0]), (Point::new(52.0, 200.0), Point::new(52.0, 200.0)));
}

#[test]
fn merge_edge_refuses_bad_params_and_locked_paths() {
    let mut s = session();
    let (a, b) = shapes(&mut s);
    let (i, j) = (a.0, b.0);
    let n = steps(&s);
    for p in [
        json!({}),
        json!({"a": i, "b": i, "at": [125, 100]}),
        json!({"a": i, "b": j}),
        json!({"a": i, "b": j, "at": [1]}),
        json!({"a": i, "b": j, "at": [1, "x"]}),
        json!({"a": i, "b": j, "at": [125, 100], "diameter": 0.1}),
        json!({"a": i, "b": j, "at": [125, 100], "diameter": 201}),
        json!({"a": i, "b": j, "at": [125, 100], "tolerance": 3.5}),
        json!({"a": i, "b": j, "at": [125, 100], "tolerance": -1}),
        json!({"a": i, "b": j, "at": [125, 100], "follow": "up"}),
        json!({"a": i, "b": j, "at": [125, 100], "ends": 1}),
        json!({"a": i, "b": u64::MAX, "at": [125, 100]}),
        json!({"a": i, "b": j, "at": [390, 290]}),
    ] {
        assert!(s.execute("addon.pathtools.mergeEdge", &p).is_err(), "{p}");
    }
    assert_eq!(steps(&s), n);
    s.execute("select.set", &json!({"ids": [i]})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    assert!(s.execute("addon.pathtools.mergeEdge", &json!({"a": i, "b": j, "at": [125, 100]})).is_err());
}

#[test]
fn the_tool_merges_unselected_paths_on_a_click_with_shift_following_the_upper() {
    let mut s = session();
    let (lower, upper) = shapes(&mut s);
    s.execute("select.none", &json!({})).unwrap();
    let view = ViewInfo { smart_guides: false, ..ViewInfo::default() };
    s.select_tool("sharedEdge", view).unwrap();
    let keep = path(&s, upper);
    let n = steps(&s);
    let shifted = |kind| {
        let mut e = PointerEvent::new(kind, 125.0, 100.0);
        e.mods = Mods { shift: true, ..Mods::default() };
        e
    };
    s.pointer(&shifted(PointerKind::Down), view).unwrap();
    s.pointer(&shifted(PointerKind::Up), view).unwrap();
    assert_eq!(steps(&s), n + 1);
    assert_eq!(path(&s, upper), keep);
    assert_eq!(points(&s, lower)[1], Point::new(150.0, 96.0));
}

#[test]
fn keep_shared_points_together_is_off_by_default_and_a_preference_of_the_tool() {
    let mut s = session();
    assert_eq!(s.tool_options_of("sharedEdge")["together"], json!(false));
    assert_eq!(s.execute("addon.pathtools.movePointsTogether", &json!({"enabled": true})).unwrap(), json!({"enabled": true}));
    assert_eq!(s.tool_options_of("sharedEdge")["together"], json!(true), "kept with another tool active");
    assert_eq!(s.execute("addon.pathtools.movePointsTogether", &json!({})).unwrap(), json!({"enabled": false}));
    assert!(s.execute("addon.pathtools.movePointsTogether", &json!({"enabled": 1})).is_err());
}

#[test]
fn a_slide_takes_coincident_anchors_along_in_one_step_when_the_option_is_on() {
    let mut s = session();
    let a = line(&mut s, &[(100.0, 100.0), (200.0, 100.0), (300.0, 100.0)], false);
    let b = line(&mut s, &[(200.0, 100.0), (200.0, 200.0)], false);
    let far = line(&mut s, &[(200.0, 101.0), (250.0, 200.0)], false);
    let slide = json!({"id": a.0, "anchor": 1, "t": 0.5});
    let n = steps(&s);
    s.execute("addon.pathtools.slideAnchor", &slide).unwrap();
    assert_eq!(points(&s, b)[0], Point::new(200.0, 100.0), "off: it stays");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("addon.pathtools.movePointsTogether", &json!({"enabled": true})).unwrap();
    s.execute("addon.pathtools.slideAnchor", &slide).unwrap();
    assert_eq!(steps(&s), n + 1);
    let moved = points(&s, a)[1];
    assert!(moved.distance(Point::new(250.0, 100.0)) < 1e-6, "{moved:?}");
    assert!(points(&s, b)[0].distance(moved) < 1e-9);
    assert_eq!(points(&s, b)[1], Point::new(200.0, 200.0));
    assert_eq!(points(&s, far)[0], Point::new(200.0, 101.0), "not coincident");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(points(&s, b)[0], Point::new(200.0, 100.0));
    // A wider coincidence reaches it, and a locked path stays.
    s.set_tool_options(Some("sharedEdge"), &serde_json::Map::from_iter([("tolerance".to_string(), json!(2))]));
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    s.execute("addon.pathtools.slideAnchor", &slide).unwrap();
    assert!(points(&s, far)[0].distance(Point::new(250.0, 101.0)) < 1e-6);
    assert_eq!(points(&s, b)[0], Point::new(200.0, 100.0));
}

#[test]
fn set_anchor_and_extend_take_coincident_anchors_along_but_built_in_moves_do_not() {
    let mut s = session();
    let a = line(&mut s, &[(100.0, 100.0), (200.0, 100.0)], false);
    let b = line(&mut s, &[(200.0, 100.0), (200.0, 200.0)], false);
    s.execute("addon.pathtools.movePointsTogether", &json!({"enabled": true})).unwrap();
    // Set Anchor on one selected anchor.
    s.select(|_, sel| {
        sel.set([a]);
        sel.anchors.insert(a, [(0, 1)].into_iter().collect());
    })
    .unwrap();
    let n = steps(&s);
    s.execute("addon.pathtools.setAnchor", &json!({"x": 220, "y": 90})).unwrap();
    assert_eq!(steps(&s), n + 1);
    assert_eq!(points(&s, b), [Point::new(220.0, 90.0), Point::new(200.0, 200.0)]);
    // An edit that moves no anchor moves none.
    s.execute("addon.pathtools.corner", &json!({})).unwrap();
    assert_eq!(points(&s, b)[0], Point::new(220.0, 90.0));
    // Extend Path moves an end only when it adds no anchor.
    s.execute("addon.pathtools.extend", &json!({"id": a.0, "end": "end", "length": 10, "mode": "bezier"})).unwrap();
    let end = points(&s, a)[1];
    assert!(points(&s, b)[0].distance(end) < 1e-9);
    let end = points(&s, b)[0];
    s.execute("addon.pathtools.extend", &json!({"id": a.0, "end": "end", "length": 10})).unwrap();
    assert_eq!(points(&s, a).len(), 3);
    assert_eq!(points(&s, b)[0], end, "the end it left stays with what shares it");
    // The app's own moves are untouched by the option.
    s.select(|_, sel| {
        sel.set([a]);
        sel.anchors.insert(a, [(0, 1)].into_iter().collect());
    })
    .unwrap();
    s.execute("object.move", &json!({"dx": 5, "dy": 5})).unwrap();
    assert_eq!(points(&s, b)[0], end);
}
