//! Smooth Bridge: the outline in place of the circles or above them, one undo step, the radius,
//! and what it refuses.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::kurbo::Shape;

use crate::Session;

const BRIDGE: &str = "addon.pathtools.bridge";

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

/// A circle about (`x`, `y`) with radius `r`.
fn circle(s: &mut Session, x: f64, y: f64, r: f64) -> NodeId {
    let id = s.execute("shape.ellipse", &json!({"x": x - r, "y": y - r, "width": 2.0 * r, "height": 2.0 * r})).unwrap()["id"].as_u64().unwrap();
    NodeId(id)
}

fn select(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn top_level(s: &Session) -> Vec<NodeId> {
    s.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id).collect()
}

fn area(s: &Session, id: NodeId) -> f64 {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().to_bezpath().area().abs()
}

fn err(s: &mut Session, p: Value) -> String {
    s.execute(BRIDGE, &p).unwrap_err().to_string()
}

#[test]
fn two_circles_become_one_outline_in_the_upper_ones_place() {
    let mut s = session();
    let a = circle(&mut s, 100.0, 100.0, 20.0);
    let b = circle(&mut s, 160.0, 100.0, 20.0);
    select(&mut s, &[a, b]);
    let steps = s.doc().unwrap().history.undo.len();
    let r = s.execute(BRIDGE, &json!({})).unwrap();
    assert_eq!(r["id"], json!(b.0));
    assert_eq!(top_level(&s), [b]);
    let d = &s.doc().unwrap().doc;
    let NodeKind::Path { path, live, .. } = &d.node(b).unwrap().kind else { panic!("a path") };
    assert!(live.is_none(), "a plain path now");
    assert!(path.subpaths.len() == 1 && path.subpaths[0].closed);
    let bounds = path.bounds().unwrap();
    assert!((bounds.x0 - 80.0).abs() < 1e-2 && (bounds.x1 - 180.0).abs() < 1e-2 && (bounds.height() - 40.0).abs() < 1e-2, "{bounds:?}");
    // Both circles and the bridge between them: more than the two circles' area.
    assert!(area(&s, b) > 2.0 * std::f64::consts::PI * 400.0);
    assert_eq!(s.doc().unwrap().selection.objects.to_vec(), [b]);
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "one undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(top_level(&s), [a, b]);
    assert!(matches!(&s.doc().unwrap().doc.node(b).unwrap().kind, NodeKind::Path { live: Some(_), .. }), "the circle is back as it was");
}

#[test]
fn keep_originals_adds_the_outline_above_the_upper_circle() {
    let mut s = session();
    let a = circle(&mut s, 100.0, 100.0, 30.0);
    let b = circle(&mut s, 170.0, 100.0, 10.0);
    let c = circle(&mut s, 300.0, 250.0, 5.0);
    s.execute("paint.setFill", &json!({"ids": [b.0], "color": "#00ff00"})).unwrap();
    let steps = s.doc().unwrap().history.undo.len();
    let id = NodeId(s.execute(BRIDGE, &json!({"ids": [b.0, a.0], "keepOriginals": true, "radius": 400})).unwrap()["id"].as_u64().unwrap());
    assert_eq!(top_level(&s), [a, b, id, c]);
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.node(id).unwrap().appearance, d.node(b).unwrap().appearance, "painted as the upper circle");
    assert!(matches!(&d.node(a).unwrap().kind, NodeKind::Path { live: Some(_), .. }), "the circles stay as they are");
    assert!(matches!(&d.node(b).unwrap().kind, NodeKind::Path { live: Some(_), .. }));
    let bounds = d.node(id).unwrap().path_data().unwrap().bounds().unwrap();
    assert!((bounds.x0 - 70.0).abs() < 1e-2 && (bounds.x1 - 180.0).abs() < 1e-2 && (bounds.y0 - 70.0).abs() < 1e-2, "{bounds:?}");
    assert_eq!(s.doc().unwrap().selection.objects.to_vec(), [id]);
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1);
}

#[test]
fn a_kept_outline_gets_its_own_mask_art() {
    let mut s = session();
    let a = circle(&mut s, 100.0, 100.0, 20.0);
    let b = circle(&mut s, 150.0, 100.0, 20.0);
    let m = NodeId(s.execute("shape.rectangle", &json!({"x": 130, "y": 80, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap());
    s.execute("transparency.makeOpacityMask", &json!({"ids": [b.0, m.0]})).unwrap();
    let id = NodeId(s.execute(BRIDGE, &json!({"ids": [a.0, b.0], "keepOriginals": true})).unwrap()["id"].as_u64().unwrap());
    let d = &s.doc().unwrap().doc;
    let art = |n: NodeId| d.node(n).unwrap().extras().mask.as_deref().map(|m| m.art.id);
    let (kept, copied) = (art(b).unwrap(), art(id).unwrap());
    assert_ne!(kept, copied, "the outline's mask art has ids of its own");
}

#[test]
fn a_larger_radius_makes_a_wider_bridge() {
    let area_at = |radius: f64| {
        let mut s = session();
        let a = circle(&mut s, 100.0, 100.0, 20.0);
        let b = circle(&mut s, 150.0, 100.0, 15.0);
        let id = NodeId(s.execute(BRIDGE, &json!({"ids": [a.0, b.0], "radius": radius})).unwrap()["id"].as_u64().unwrap());
        area(&s, id)
    };
    let (default, wider, widest) = (area_at(100.0), area_at(200.0), area_at(400.0));
    assert!(default < wider && wider < widest, "{default} {wider} {widest}");
    let (narrow, default, wide) = (area_at(65.0), default, area_at(300.0));
    assert!(narrow < default && default < wide, "{narrow} {default} {wide}");
    // The default is 100%: the smaller circle's radius.
    let mut s = session();
    let a = circle(&mut s, 100.0, 100.0, 20.0);
    let b = circle(&mut s, 150.0, 100.0, 15.0);
    let id = NodeId(s.execute(BRIDGE, &json!({"ids": [a.0, b.0]})).unwrap()["id"].as_u64().unwrap());
    assert!((area(&s, id) - default).abs() < 1e-9);
}

#[test]
fn overlapping_and_touching_circles_are_joined() {
    for (gap, radius) in [(-15.0, 50.0), (0.0, 50.0), (0.0, 1.0)] {
        let mut s = session();
        let a = circle(&mut s, 100.0, 100.0, 20.0);
        let b = circle(&mut s, 140.0 + gap, 100.0, 20.0);
        let id = NodeId(s.execute(BRIDGE, &json!({"ids": [a.0, b.0], "radius": radius})).unwrap()["id"].as_u64().unwrap());
        assert_eq!(top_level(&s), [id], "gap {gap}");
    }
    // Unequal overlapping circles at every radius, the largest included.
    for radius in [1.0, 100.0, 300.0, 1000.0] {
        let mut s = session();
        let a = circle(&mut s, 100.0, 100.0, 10.0);
        let b = circle(&mut s, 125.0, 100.0, 30.0);
        let id = NodeId(s.execute(BRIDGE, &json!({"ids": [a.0, b.0], "radius": radius})).unwrap()["id"].as_u64().unwrap());
        assert_eq!(top_level(&s), [id], "radius {radius}");
    }
}

#[test]
fn far_apart_nested_and_other_objects_are_refused() {
    let mut s = session();
    let a = circle(&mut s, 50.0, 100.0, 10.0);
    let far = circle(&mut s, 250.0, 100.0, 10.0);
    let near = circle(&mut s, 110.0, 100.0, 10.0);
    let inner = circle(&mut s, 52.0, 100.0, 4.0);
    let square = NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
    let oval = NodeId(s.execute("shape.ellipse", &json!({"x": 300, "y": 0, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap());
    let before = top_level(&s);
    let steps = s.doc().unwrap().history.undo.len();
    // 60 pt apart: the default radius (10 pt) cannot reach, the largest (100 pt) can.
    assert!(err(&mut s, json!({"ids": [a.0, near.0]})).ends_with("too far apart for this radius: raise the radius"));
    assert!(s.execute(BRIDGE, &json!({"ids": [a.0, near.0], "radius": 1000})).is_ok());
    s.execute("edit.undo", &json!({})).unwrap();
    // 200 pt apart even the largest radius (1000%, 100 pt) closes the neck: it stays open only above
    // ((200 / 2)² − 10²) / (2 × 10) = 495 pt. No larger radius is suggested.
    for radius in [100, 1000] {
        let e = err(&mut s, json!({"ids": [a.0, far.0], "radius": radius}));
        assert!(e.ends_with("the circles are too far apart for this radius"), "{e}");
    }
    assert!(err(&mut s, json!({"ids": [a.0, inner.0]})).contains("one circle is inside the other"));
    for ids in [json!([a.0, square.0]), json!([a.0, oval.0]), json!([a.0]), json!([a.0, far.0, inner.0])] {
        assert!(err(&mut s, json!({ "ids": ids })).contains("select two circles to bridge"), "{ids}");
    }
    assert!(err(&mut s, json!({"ids": [a.0, far.0], "radius": 0.5})).contains("`radius` must be from 1 to 1000"));
    assert!(err(&mut s, json!({"ids": [a.0, far.0], "radius": 1001})).contains("`radius`"));
    assert!(err(&mut s, json!({"ids": [a.0, far.0], "radius": "big"})).contains("`radius` must be a number"));
    assert!(err(&mut s, json!({"ids": [a.0, far.0], "keepOriginals": "yes"})).contains("`keepOriginals` must be true or false"));
    assert_eq!(top_level(&s), before);
    assert_eq!(s.doc().unwrap().history.undo.len(), steps, "nothing changed, no undo step");
}

#[test]
fn locked_and_hidden_circles_are_not_bridged() {
    let mut s = session();
    let a = circle(&mut s, 100.0, 100.0, 20.0);
    let b = circle(&mut s, 150.0, 100.0, 20.0);
    select(&mut s, &[b]);
    s.execute("object.lock", &json!({})).unwrap();
    assert!(err(&mut s, json!({"ids": [a.0, b.0]})).contains("select two circles to bridge"));
    s.execute("object.unlockAll", &json!({})).unwrap();
    select(&mut s, &[a]);
    s.execute("object.hide", &json!({})).unwrap();
    assert!(err(&mut s, json!({"ids": [a.0, b.0]})).contains("select two circles to bridge"));
}
