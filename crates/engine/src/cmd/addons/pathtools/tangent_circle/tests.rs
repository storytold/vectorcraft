//! Tangent Circle: circles drawn for two shapes and a radius or three shapes, one undo step, the
//! current paint, compound paths, one circle picked, and the errors that leave the document as it was.

use serde_json::{Value, json};
use vectorcraft_doc::{LiveShape, NodeId, NodeKind};
use vectorcraft_geom::Point;

use super::COMMAND;
use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn ellipse(s: &mut Session, x: f64, y: f64, w: f64, h: f64, rotation: f64) -> u64 {
    let id = s.execute("shape.ellipse", &json!({"x": x, "y": y, "width": w, "height": h, "rotation": rotation})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.none", &json!({})).unwrap();
    id
}

fn circle(s: &mut Session, cx: f64, cy: f64, r: f64) -> u64 {
    ellipse(s, cx - r, cy - r, 2.0 * r, 2.0 * r, 0.0)
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn count(s: &Session) -> usize {
    s.doc().unwrap().doc.layers[0].children().unwrap().len()
}

/// The circle path `id` is: (center, radius) from its live ellipse.
fn drawn(s: &Session, id: &Value) -> (Point, f64) {
    let n = s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap();
    let NodeKind::Path { live: Some(live), .. } = &n.kind else { panic!("not a live shape") };
    let LiveShape::Ellipse { w, h, xf, .. } = live.as_ref() else { panic!("not an ellipse") };
    assert!((w - h).abs() < 1e-9);
    (*xf * Point::new(w / 2.0, h / 2.0), w / 2.0)
}

fn refused(s: &mut Session, p: Value) -> String {
    let (steps, objects) = (undo_steps(s), count(s));
    let e = s.execute(COMMAND, &p).unwrap_err().to_string();
    assert_eq!((undo_steps(s), count(s)), (steps, objects), "{p} changed the document");
    e
}

#[test]
fn two_circles_and_a_radius_draw_every_circle_as_one_step() {
    let mut s = session();
    let a = circle(&mut s, 100.0, 100.0, 10.0);
    let b = circle(&mut s, 130.0, 100.0, 10.0);
    s.execute("paint.setFill", &json!({"color": "#12ab34"})).unwrap();
    let steps = undo_steps(&s);
    let r = s.execute(COMMAND, &json!({"ids": [a, b], "radius": 20})).unwrap();
    assert_eq!(r["found"], 6);
    let ids = r["ids"].as_array().unwrap().clone();
    assert_eq!(ids.len(), 6);
    assert_eq!(undo_steps(&s), steps + 1);
    assert_eq!(count(&s), 8);
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects.len(), 6);
    let fill = st.doc.node(NodeId(ids[0].as_u64().unwrap())).unwrap().appearance.fill_paint();
    assert_eq!(fill, s.paint.fill);
    // Outside both: 30 from each center, at x = 115.
    let h = (900.0f64 - 225.0).sqrt();
    let centers: Vec<(Point, f64)> = ids.iter().map(|id| drawn(&s, id)).collect();
    for want in [Point::new(115.0, 100.0 - h), Point::new(115.0, 100.0 + h)] {
        // Within the Bézier circles' own error (0.0273% of the radius) of the ideal answer.
        assert!(centers.iter().any(|(c, r)| (*c - want).hypot() < 0.02 && (r - 20.0).abs() < 1e-9), "{centers:?}");
    }
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(count(&s), 2);
}

#[test]
fn three_circles_and_one_picked() {
    let mut s = session();
    let ids = [circle(&mut s, 100.0, 100.0, 10.0), circle(&mut s, 180.0, 100.0, 10.0), circle(&mut s, 100.0, 160.0, 10.0)];
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    // The selection: all eight, smallest first; the first is the one outside all three, about the
    // circumcenter (140, 130), 50 from each center.
    let r = s.execute(COMMAND, &json!({"circle": 1})).unwrap();
    assert_eq!(r["found"], 8);
    let one = r["ids"].as_array().unwrap();
    assert_eq!(one.len(), 1);
    let (c, radius) = drawn(&s, &one[0]);
    assert!((c - Point::new(140.0, 130.0)).hypot() < 0.05 && (radius - 40.0).abs() < 0.05, "{c:?} {radius}");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    assert_eq!(s.execute(COMMAND, &json!({})).unwrap()["ids"].as_array().unwrap().len(), 8);
    assert!(refused(&mut s, json!({"ids": ids, "circle": 9})).contains("past the last circle found (8)"));
}

#[test]
fn compound_rotated_and_locked_shapes() {
    let mut s = session();
    // A ring as a compound path, and an ellipse turned 30° inside its hole.
    let outer = circle(&mut s, 200.0, 150.0, 80.0);
    let inner = circle(&mut s, 200.0, 150.0, 60.0);
    s.execute("select.set", &json!({"ids": [outer, inner]})).unwrap();
    let ring = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    let oval = ellipse(&mut s, 180.0, 140.0, 40.0, 20.0, 30.0);
    s.execute("select.set", &json!({"ids": [oval]})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [ring]})).unwrap();
    s.execute("object.hide", &json!({})).unwrap();
    let r = s.execute(COMMAND, &json!({"ids": [ring, oval], "radius": 22})).unwrap();
    let ids = r["ids"].as_array().unwrap();
    // In the hole, 38 from the center, and 22 from the oval (reaching 42 along its long axis and
    // 32 along its short one): two by each end of the oval.
    assert_eq!(ids.len(), 4, "{r}");
    for id in ids {
        let (c, radius) = drawn(&s, id);
        assert!(((c - Point::new(200.0, 150.0)).hypot() - (60.0 - radius)).abs() < 0.05, "{c:?}");
    }
    // The locked oval and the hidden ring were read and left as they were.
    let d = &s.doc().unwrap().doc;
    assert!(d.node(NodeId(oval)).unwrap().locked);
    assert!(!d.node(NodeId(ring)).unwrap().visible);
}

#[test]
fn circles_go_into_an_isolated_group() {
    let mut s = session();
    let a = circle(&mut s, 100.0, 100.0, 10.0);
    let b = circle(&mut s, 130.0, 100.0, 10.0);
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let group = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("object.isolate", &json!({"id": group})).unwrap();
    let r = s.execute(COMMAND, &json!({"ids": [a, b], "radius": 5})).unwrap();
    let id = NodeId(r["ids"][0].as_u64().unwrap());
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.parent_of(id), Some(NodeId(group)));
    // On top of the group's art.
    assert_eq!(d.node(NodeId(group)).unwrap().children().unwrap().last().map(|n| n.id), Some(id));
}

#[test]
fn circles_join_an_isolated_compound_path() {
    let mut s = session();
    let a = circle(&mut s, 100.0, 100.0, 10.0);
    let b = circle(&mut s, 130.0, 100.0, 10.0);
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let compound = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("object.isolate", &json!({"id": compound})).unwrap();
    let members: Vec<u64> = s.doc().unwrap().doc.node(NodeId(compound)).unwrap().children().unwrap().iter().map(|c| c.id.0).collect();
    let r = s.execute(COMMAND, &json!({"ids": members, "radius": 5})).unwrap();
    let id = NodeId(r["ids"][0].as_u64().unwrap());
    assert_eq!(s.doc().unwrap().doc.parent_of(id), Some(NodeId(compound)));
    // A compound path and one of its own members are taken as the compound path alone: one shape.
    let e = refused(&mut s, json!({"ids": [compound, members[0]], "radius": 5}));
    assert!(e.contains("two or three"), "{e}");
    // A repeated member is refused even though the compound path holds it.
    let other = circle(&mut s, 300.0, 100.0, 10.0);
    let e = refused(&mut s, json!({"ids": [compound, members[0], members[0], other], "radius": 5}));
    assert!(e.contains("twice"), "{e}");
}

#[test]
fn errors_leave_the_document_unchanged() {
    let mut s = session();
    let a = circle(&mut s, 50.0, 50.0, 10.0);
    let b = circle(&mut s, 250.0, 50.0, 10.0);
    let c = circle(&mut s, 150.0, 200.0, 10.0);
    assert!(refused(&mut s, json!({"ids": [a, b], "radius": 5})).contains("no circle touches"));
    assert!(refused(&mut s, json!({"ids": [a, b]})).contains("need a `radius`"));
    assert!(refused(&mut s, json!({"ids": [a, b, c], "radius": 5})).contains("leave `radius` out"));
    assert!(refused(&mut s, json!({"ids": [a, b], "radius": 0})).contains("0.01"));
    assert!(refused(&mut s, json!({"ids": [a, b], "radius": "5 mm"})).contains("must be a number"));
    assert!(refused(&mut s, json!({"ids": [a], "radius": 5})).contains("two or three"));
    assert!(refused(&mut s, json!({"ids": [a, a], "radius": 5})).contains("twice"));
    let e = refused(&mut s, json!({"ids": [a, a as f64], "radius": 5}));
    assert!(e.contains("not a non-negative integer"), "{e}");
    // Two lone anchors at one place.
    let dot = |s: &mut Session| s.execute("path.create", &json!({"anchors": [{"x": 5, "y": 5}]})).unwrap()["id"].as_u64().unwrap();
    let (p, q) = (dot(&mut s), dot(&mut s));
    assert!(refused(&mut s, json!({"ids": [p, q], "radius": 5})).contains("too small to measure"));
    assert!(refused(&mut s, json!({"ids": [a, b], "radius": 200, "circle": 65})).contains("from 0 to 64"));
    assert!(refused(&mut s, json!({"ids": [a, b], "radius": 200, "circle": -1})).contains("whole number"));
    let t = s.execute("text.create", &json!({"x": 10, "y": 280, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    assert!(refused(&mut s, json!({"ids": [a, t], "radius": 5})).contains("not a path"));
    let anchors: Vec<Value> = (0..2002).map(|i| json!({"x": i as f64 * 0.1, "y": 250.0 + (i % 2) as f64})).collect();
    let zigzag = s.execute("path.create", &json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap();
    assert!(refused(&mut s, json!({"ids": [a, zigzag], "radius": 5})).contains("at most 2000 segments"));
    // Concentric circles touched all the way round.
    let inner = circle(&mut s, 50.0, 50.0, 40.0);
    assert!(refused(&mut s, json!({"ids": [a, inner], "radius": 15})).contains("whole curve"));
}
