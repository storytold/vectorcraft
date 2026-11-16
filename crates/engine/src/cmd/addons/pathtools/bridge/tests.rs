//! Smooth Bridge: the outline in place of the circles or above them, one undo step, the radius or
//! gap, many circles joined to their neighbors within reach, and what it refuses.

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
    assert_eq!(r["ids"], json!([b.0]), "the same result fields as for more circles");
    assert_eq!(r["alone"], json!([]));
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
    for ids in [json!([a.0, square.0]), json!([a.0, oval.0]), json!([a.0]), json!([a.0, far.0, square.0])] {
        assert!(err(&mut s, json!({ "ids": ids })).contains("select two or more circles to bridge"), "{ids}");
    }
    // Three circles: each two within reach are bridged, and one inside another is refused there too.
    assert!(err(&mut s, json!({"ids": [a.0, far.0, inner.0]})).contains("one circle is inside the other"));
    assert!(err(&mut s, json!({"ids": [a.0, far.0], "radius": 0.5})).contains("`radius` must be from 1 to 1000"));
    assert!(err(&mut s, json!({"ids": [a.0, far.0], "radius": 1001})).contains("`radius`"));
    assert!(err(&mut s, json!({"ids": [a.0, far.0], "radius": "big"})).contains("`radius` must be a number"));
    assert!(err(&mut s, json!({"ids": [a.0, far.0], "keepOriginals": "yes"})).contains("`keepOriginals` must be true or false"));
    assert_eq!(top_level(&s), before);
    assert_eq!(s.doc().unwrap().history.undo.len(), steps, "nothing changed, no undo step");
}

/// A 3 × 3 grid of circles of radius 10, 4 apart (centers 24 apart), in rows from (100, 100).
fn grid(s: &mut Session) -> Vec<NodeId> {
    (0..9).map(|k| circle(s, 100.0 + 24.0 * (k % 3) as f64, 100.0 + 24.0 * (k / 3) as f64, 10.0)).collect()
}

fn ids(v: &Value) -> Vec<NodeId> {
    v.as_array().unwrap().iter().map(|i| NodeId(i.as_u64().unwrap())).collect()
}

fn path(s: &Session, id: NodeId) -> vectorcraft_geom::PathData {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().clone()
}

#[test]
fn a_grid_of_circles_joins_each_to_its_neighbors_in_one_shape() {
    let mut s = session();
    let g = grid(&mut s);
    select(&mut s, &g);
    let steps = s.doc().unwrap().history.undo.len();
    // Reach 5: the side neighbors (4 apart) are bridged, the diagonal ones (about 13.9 apart) not.
    let r = s.execute(BRIDGE, &json!({"gap": 4, "reach": 5})).unwrap();
    assert_eq!(ids(&r["ids"]), [g[8]], "one shape, in the topmost circle's place");
    assert_eq!(ids(&r["alone"]), []);
    assert_eq!(top_level(&s), [g[8]]);
    assert_eq!(s.doc().unwrap().selection.objects.to_vec(), [g[8]]);
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "one undo step");
    // Its outline and a hole in each square of four circles, which no diagonal bridge fills.
    let outline = path(&s, g[8]);
    assert_eq!(outline.subpaths.len(), 5, "{}", outline.subpaths.len());
    let inside = |p: &vectorcraft_geom::PathData, x: f64, y: f64| p.to_bezpath().winding(vectorcraft_geom::kurbo::Point::new(x, y)) != 0;
    assert!(inside(&outline, 112.0, 100.0) && inside(&outline, 100.0, 112.0) && !inside(&outline, 112.0, 112.0));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(top_level(&s), g);
    // Reach 15 takes in the diagonal neighbors, whose bridges fill the middle of each square.
    select(&mut s, &g);
    s.execute(BRIDGE, &json!({"gap": 4, "reach": 15})).unwrap();
    let outline = path(&s, g[8]);
    assert_eq!(outline.subpaths.len(), 1);
    assert!(inside(&outline, 112.0, 112.0));
}

#[test]
fn a_circle_with_no_neighbor_in_reach_stays_as_it_is() {
    let mut s = session();
    let g = grid(&mut s);
    let far = circle(&mut s, 300.0, 250.0, 10.0);
    let pair = [circle(&mut s, 300.0, 40.0, 8.0), circle(&mut s, 320.0, 40.0, 8.0)];
    let mut all = g.clone();
    all.extend([far, pair[0], pair[1]]);
    let r = s.execute(BRIDGE, &json!({"ids": all.iter().map(|i| i.0).collect::<Vec<_>>(), "reach": 5})).unwrap();
    assert_eq!(ids(&r["ids"]), [g[8], pair[1]]);
    assert_eq!(ids(&r["alone"]), [far]);
    assert_eq!(top_level(&s), [g[8], far, pair[1]]);
    assert!(matches!(&s.doc().unwrap().doc.node(far).unwrap().kind, NodeKind::Path { live: Some(_), .. }), "untouched");
    let mut selected = s.doc().unwrap().selection.objects.to_vec();
    selected.sort();
    let mut want = vec![g[8], pair[1], far];
    want.sort();
    assert_eq!(selected, want, "the shapes and the circle left alone");
}

#[test]
fn keep_originals_adds_each_shape_above_its_topmost_circle() {
    let mut s = session();
    let row: Vec<NodeId> = (0..3).map(|i| circle(&mut s, 100.0 + 25.0 * i as f64, 100.0, 10.0)).collect();
    let other = [circle(&mut s, 100.0, 200.0, 10.0), circle(&mut s, 122.0, 200.0, 10.0)];
    s.execute("paint.setFill", &json!({"ids": [row[2].0], "color": "#00ff00"})).unwrap();
    let all: Vec<u64> = row.iter().chain(&other).map(|i| i.0).collect();
    let r = s.execute(BRIDGE, &json!({"ids": all, "keepOriginals": true})).unwrap();
    let made = ids(&r["ids"]);
    assert_eq!(made.len(), 2);
    assert_eq!(top_level(&s), [row[0], row[1], row[2], made[0], other[0], other[1], made[1]]);
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.node(made[0]).unwrap().appearance, d.node(row[2]).unwrap().appearance, "painted as the topmost circle");
    let b = d.node(made[0]).unwrap().path_data().unwrap().bounds().unwrap();
    assert!((b.x0 - 90.0).abs() < 1e-2 && (b.x1 - 160.0).abs() < 1e-2, "{b:?}");
}

#[test]
fn the_gap_form_gives_arcs_the_smaller_radius_plus_the_gap() {
    let area_with = |p: Value| {
        let mut s = session();
        let a = circle(&mut s, 100.0, 100.0, 10.0);
        let b = circle(&mut s, 124.0, 100.0, 10.0);
        let mut p = p;
        p["ids"] = json!([a.0, b.0]);
        let id = NodeId(s.execute(BRIDGE, &p).unwrap()["id"].as_u64().unwrap());
        area(&s, id)
    };
    // Radius 10 plus a gap of 4 is 140% of 10.
    let (gap, percent) = (area_with(json!({"gap": 4})), area_with(json!({"radius": 140})));
    assert!((gap - percent).abs() < 1e-9, "{gap} {percent}");
    assert!(area_with(json!({"gap": 0})) < gap && gap < area_with(json!({"gap": 20})));
}

#[test]
fn many_circle_and_gap_refusals() {
    let mut s = session();
    let a = circle(&mut s, 50.0, 100.0, 10.0);
    let b = circle(&mut s, 250.0, 100.0, 10.0);
    let c = circle(&mut s, 330.0, 100.0, 10.0);
    let far = circle(&mut s, 380.0, 100.0, 10.0);
    let before = top_level(&s);
    let steps = s.doc().unwrap().history.undo.len();
    assert!(err(&mut s, json!({"ids": [a.0, b.0], "radius": 100, "gap": 4})).contains("give `radius` or `gap`, not both"));
    // Centers 200 apart: a gap of 0 does not bridge them, the largest (1000 pt) does. With centers
    // 330 apart even that one closes the neck: it stays open only below 2 × √(10² + 2 × 10 × 1010)
    // ≈ 285.
    assert!(err(&mut s, json!({"ids": [a.0, b.0], "gap": 0})).ends_with("too far apart for this gap: raise the gap"));
    assert!(err(&mut s, json!({"ids": [a.0, far.0], "gap": 0})).ends_with("the circles are too far apart for this gap"));
    assert!(err(&mut s, json!({"ids": [a.0, b.0, c.0], "reach": 5})).contains("no two circles are within reach: raise the reach"));
    // b and c are 60 apart: within a reach of 100, but too far for the default radius.
    assert!(err(&mut s, json!({"ids": [a.0, b.0, c.0], "reach": 100})).ends_with("too far apart for this radius: raise the radius"));
    assert!(err(&mut s, json!({"ids": [a.0, b.0, c.0], "reach": -1})).contains("`reach` must be from 0 to 10000"));
    assert!(err(&mut s, json!({"ids": [a.0, b.0], "gap": 1001})).contains("`gap` must be from 0 to 1000"));
    assert_eq!(top_level(&s), before);
    assert_eq!(s.doc().unwrap().history.undo.len(), steps, "nothing changed, no undo step");
}

#[test]
fn a_larger_radius_is_suggested_only_when_it_bridges_among_the_other_circles() {
    let mut s = session();
    // Centers 91.65 apart: the largest radius (1000%, 100 pt) bridges these two on their own, but
    // with a neck about 0.0007 pt wide, too thin to unite among three or more circles.
    let a = circle(&mut s, 20.0, 100.0, 10.0);
    let b = circle(&mut s, 111.65, 100.0, 10.0);
    let far = circle(&mut s, 380.0, 100.0, 10.0);
    assert!(s.execute(BRIDGE, &json!({"ids": [a.0, b.0], "radius": 1000})).is_ok());
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(err(&mut s, json!({"ids": [a.0, b.0]})).ends_with("too far apart for this radius: raise the radius"));
    for radius in [100, 1000] {
        let e = err(&mut s, json!({"ids": [a.0, b.0, far.0], "radius": radius, "reach": 100}));
        assert!(e.ends_with("the circles are too far apart for this radius"), "{e}");
    }
}

#[test]
fn too_many_circles_or_bridges_are_refused() {
    let mut s = session();
    // 130 overlapping circles in a row: 8,385 pairs within reach.
    let row: Vec<u64> = (0..130).map(|i| circle(&mut s, 20.0 + i as f64 * 0.2, 100.0, 10.0).0).collect();
    assert!(err(&mut s, json!({ "ids": row })).contains("that makes too many bridges (at most 8000): lower the reach"));
    let many: Vec<u64> = (0..2001).map(|i| circle(&mut s, 20.0 + (i % 50) as f64 * 7.0, 150.0 + (i / 50) as f64 * 3.0, 1.0).0).collect();
    assert!(err(&mut s, json!({ "ids": many })).contains("select at most 2000 circles to bridge"));
}

#[test]
fn an_id_given_twice_is_one_circle() {
    let mut s = session();
    let a = circle(&mut s, 100.0, 100.0, 20.0);
    let b = circle(&mut s, 150.0, 100.0, 20.0);
    let r = s.execute(BRIDGE, &json!({"ids": [a.0, a.0, b.0]})).unwrap();
    assert_eq!(r["id"], json!(b.0), "two circles, bridged");
    assert_eq!(top_level(&s), [b]);
}

#[test]
fn small_circles_at_the_default_reach_bridge_the_pairs_the_arcs_can_join() {
    let mut s = session();
    // Radius 3: arcs of 3 pt (100%) join a and b (2 apart) but close before reaching c (6 from b,
    // within the default reach of 10).
    let (a, b, c) = (circle(&mut s, 100.0, 100.0, 3.0), circle(&mut s, 108.0, 100.0, 3.0), circle(&mut s, 120.0, 100.0, 3.0));
    let r = s.execute(BRIDGE, &json!({"ids": [a.0, b.0, c.0]})).unwrap();
    assert_eq!(ids(&r["ids"]), [b]);
    assert_eq!(ids(&r["alone"]), [c]);
    assert_eq!(top_level(&s), [b, c]);
}

#[test]
fn locked_and_hidden_circles_are_not_bridged() {
    let mut s = session();
    let a = circle(&mut s, 100.0, 100.0, 20.0);
    let b = circle(&mut s, 150.0, 100.0, 20.0);
    select(&mut s, &[b]);
    s.execute("object.lock", &json!({})).unwrap();
    assert!(err(&mut s, json!({"ids": [a.0, b.0]})).contains("select two or more circles to bridge"));
    s.execute("object.unlockAll", &json!({})).unwrap();
    select(&mut s, &[a]);
    s.execute("object.hide", &json!({})).unwrap();
    assert!(err(&mut s, json!({"ids": [a.0, b.0]})).contains("select two or more circles to bridge"));
}
