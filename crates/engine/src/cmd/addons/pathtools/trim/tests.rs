//! Trim at Crossings: the stretch at a point, the stretches a rectangle touches or a line crosses,
//! one undo step, and what takes no part.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind, Selection};
use vectorcraft_geom::{PathData, Point};
use vectorcraft_tools::addons::trim::plan::{self, Limits, Stretches};

use crate::Session;

const TRIM: &str = "addon.pathtools.trim";

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn line(s: &mut Session, pts: &[(f64, f64)], closed: bool) -> NodeId {
    let anchors: Vec<Value> = pts.iter().map(|(x, y)| json!({"x": x, "y": y})).collect();
    let id = NodeId(s.execute("path.create", &json!({"anchors": anchors, "closed": closed})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.none", &json!({})).unwrap();
    id
}

/// A line across at y = 100 from x = 0 to 300, and two uprights across it at x = 100 and x = 200.
fn across_and_uprights(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    let h = line(s, &[(0.0, 100.0), (300.0, 100.0)], false);
    let v1 = line(s, &[(100.0, 50.0), (100.0, 150.0)], false);
    let v2 = line(s, &[(200.0, 50.0), (200.0, 150.0)], false);
    (h, v1, v2)
}

fn path(s: &Session, id: NodeId) -> PathData {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().clone()
}

fn exists(s: &Session, id: NodeId) -> bool {
    s.doc().unwrap().doc.node(id).is_some()
}

/// The first and last anchor of the path's only subpath.
fn ends(s: &Session, id: NodeId) -> (Point, Point) {
    let p = path(s, id);
    assert_eq!(p.subpaths.len(), 1);
    (p.subpaths[0].anchors.first().unwrap().p, p.subpaths[0].anchors.last().unwrap().p)
}

fn near(p: Point, x: f64, y: f64) -> bool {
    p.distance(Point::new(x, y)) < 1e-6
}

fn steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn ids(v: &Value) -> Vec<NodeId> {
    v["ids"].as_array().unwrap().iter().map(|x| NodeId(x.as_u64().unwrap())).collect()
}

/// The ids of the first layer's objects, bottom to top.
fn stack(s: &Session) -> Vec<NodeId> {
    s.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id).collect()
}

fn set_flags(s: &mut Session, id: NodeId, locked: bool, visible: bool) {
    s.edit("flags", |d, _| {
        let n = d.node_mut(id).unwrap();
        n.locked = locked;
        n.visible = visible;
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_point_between_two_crossings_removes_that_stretch_and_splits_the_path() {
    let mut s = session();
    let (h, v1, v2) = across_and_uprights(&mut s);
    let look = s.doc().unwrap().doc.node(h).unwrap().appearance.clone();
    let (before, n) = (stack(&s), steps(&s));
    let r = s.execute(TRIM, &json!({"at": [150, 101]})).unwrap();
    assert_eq!(r["removed"], json!(1));
    let made = ids(&r);
    assert_eq!(made.len(), 2, "{r}");
    assert_eq!(made[0], h, "the first piece keeps the id");
    let (a, b) = (ends(&s, made[0]), ends(&s, made[1]));
    assert!(near(a.0, 0.0, 100.0) && near(a.1, 100.0, 100.0), "{a:?}");
    assert!(near(b.0, 200.0, 100.0) && near(b.1, 300.0, 100.0), "{b:?}");
    // Both pieces look as the path did and stand where it stood, under the uprights.
    assert!(made.iter().all(|id| s.doc().unwrap().doc.node(*id).unwrap().appearance == look));
    assert_eq!(stack(&s), [made[0], made[1], v1, v2]);
    assert_eq!(before, [h, v1, v2]);
    // The uprights are as they were, nothing got selected, and it is one undo step.
    assert_eq!((path(&s, v1).anchor_count(), path(&s, v2).anchor_count()), (2, 2));
    assert!(s.doc().unwrap().selection.is_empty());
    assert_eq!(steps(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(stack(&s), before);
    assert!(near(ends(&s, h).1, 300.0, 100.0));
}

#[test]
fn an_end_stretch_runs_to_the_open_end_and_the_rest_stays_one_path() {
    let mut s = session();
    let (h, ..) = across_and_uprights(&mut s);
    let r = s.execute(TRIM, &json!({"at": [40, 100]})).unwrap();
    assert_eq!((ids(&r), r["removed"].clone()), (vec![h], json!(1)));
    let e = ends(&s, h);
    assert!(near(e.0, 100.0, 100.0) && near(e.1, 300.0, 100.0), "{e:?}");
    // No anchor is added at the crossing the rest runs through.
    assert_eq!(path(&s, h).anchor_count(), 2);
    // A point on no path within the tolerance removes nothing and makes no step.
    let n = steps(&s);
    assert_eq!(s.execute(TRIM, &json!({"at": [250, 110]})).unwrap(), json!({"ids": [], "removed": 0}));
    assert_eq!(s.execute(TRIM, &json!({"at": [250, 110], "tolerance": 0})).unwrap()["removed"], json!(0));
    assert_eq!(steps(&s), n);
    assert_eq!(s.execute(TRIM, &json!({"at": [250, 110], "tolerance": 12})).unwrap()["removed"], json!(1));
    assert!(near(ends(&s, h).1, 200.0, 100.0));
}

#[test]
fn a_closed_path_that_loses_a_stretch_opens() {
    let mut s = session();
    let sq = NodeId(s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.none", &json!({})).unwrap();
    line(&mut s, &[(50.0, 150.0), (250.0, 150.0)], false);
    // The top half of the square's outline, between the two places the line crosses it.
    let r = s.execute(TRIM, &json!({"at": [150, 100]})).unwrap();
    assert_eq!((ids(&r), r["removed"].clone()), (vec![sq], json!(1)));
    let p = path(&s, sq);
    assert_eq!(p.subpaths.len(), 1);
    assert!(!p.subpaths[0].closed);
    let pts: Vec<Point> = p.subpaths[0].anchors.iter().map(|a| a.p).collect();
    assert_eq!(pts.len(), 4, "{pts:?}");
    assert!(pts.iter().all(|q| q.y >= 150.0 - 1e-6), "only the lower half is left: {pts:?}");
    assert!(pts.iter().any(|q| near(*q, 100.0, 200.0)) && pts.iter().any(|q| near(*q, 200.0, 200.0)));
    // It is a plain path now.
    assert!(matches!(&s.doc().unwrap().doc.node(sq).unwrap().kind, NodeKind::Path { live: None, .. }));
}

#[test]
fn a_rectangle_removes_every_stretch_it_touches_as_one_step() {
    let mut s = session();
    let (h, v1, v2) = across_and_uprights(&mut s);
    let h2 = line(&mut s, &[(0.0, 120.0), (300.0, 120.0)], false);
    let n = steps(&s);
    // Over the middle of both lines across, touching neither upright.
    let r = s.execute(TRIM, &json!({"rect": [140, 90, 20, 40]})).unwrap();
    assert_eq!(r["removed"], json!(2));
    assert_eq!(ids(&r).len(), 4);
    assert_eq!(steps(&s), n + 1, "one undo step");
    assert!(near(ends(&s, h).1, 100.0, 100.0) && near(ends(&s, h2).1, 100.0, 120.0));
    assert_eq!((path(&s, v1).anchor_count(), path(&s, v2).anchor_count()), (2, 2));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(ends(&s, h).1, 300.0, 100.0) && near(ends(&s, h2).1, 300.0, 120.0));
    // A rectangle given from its far corner is the same one; one over empty space removes nothing.
    assert_eq!(s.execute(TRIM, &json!({"rect": [160, 130, -20, -40]})).unwrap()["removed"], json!(2));
    let n = steps(&s);
    assert_eq!(s.execute(TRIM, &json!({"rect": [300, 200, 50, 50]})).unwrap()["removed"], json!(0));
    assert_eq!(steps(&s), n);
    // An upright's stretches: above the upper line, between the two, below the lower. A rectangle
    // over its top end takes the first only.
    assert_eq!(s.execute(TRIM, &json!({"rect": [95, 45, 10, 10]})).unwrap()["removed"], json!(1));
    assert!(near(ends(&s, v1).0, 100.0, 100.0) && near(ends(&s, v1).1, 100.0, 150.0));
}

#[test]
fn a_line_through_points_removes_the_stretches_it_crosses() {
    let mut s = session();
    let (h, v1, v2) = across_and_uprights(&mut s);
    let n = steps(&s);
    // Down through the middle of the line across, then right through the lower end of the second upright.
    let r = s.execute(TRIM, &json!({"points": [[150, 80], [150, 130], [250, 130]]})).unwrap();
    assert_eq!(r["removed"], json!(2));
    assert_eq!(steps(&s), n + 1);
    assert!(near(ends(&s, h).1, 100.0, 100.0));
    assert!(near(ends(&s, v2).0, 200.0, 50.0) && near(ends(&s, v2).1, 200.0, 100.0));
    assert_eq!(path(&s, v1).anchor_count(), 2);
}

#[test]
fn locked_and_hidden_paths_are_not_trimmed_and_do_not_count_as_crossing() {
    let mut s = session();
    let (h, v1, v2) = across_and_uprights(&mut s);
    set_flags(&mut s, v1, true, true);
    set_flags(&mut s, v2, false, false);
    let n = steps(&s);
    // On the locked upright, and a rectangle over the hidden one's top end: nothing goes.
    assert_eq!(s.execute(TRIM, &json!({"at": [100, 60]})).unwrap()["removed"], json!(0));
    assert_eq!(s.execute(TRIM, &json!({"rect": [195, 45, 10, 10]})).unwrap()["removed"], json!(0));
    assert_eq!(steps(&s), n);
    assert_eq!((path(&s, v1).anchor_count(), path(&s, v2).anchor_count()), (2, 2));
    // Neither marks a place on the line across: with nothing else crossing it, it goes whole.
    assert_eq!(s.execute(TRIM, &json!({"at": [150, 100]})).unwrap(), json!({"ids": [], "removed": 1}));
    assert!(!exists(&s, h) && exists(&s, v1) && exists(&s, v2));
    // A guide doesn't count either.
    s.execute("edit.undo", &json!({})).unwrap();
    set_flags(&mut s, v1, false, true);
    s.execute("select.set", &json!({"ids": [v1.0]})).unwrap();
    s.execute("view.guides.make", &json!({})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    s.execute(TRIM, &json!({"at": [150, 100]})).unwrap();
    assert!(!exists(&s, h));
}

#[test]
fn a_path_of_two_subpaths_loses_the_stretch_on_the_subpath_under_the_point() {
    let mut s = session();
    // Lines of different lengths, so the places on one fall at other parameters than on the other.
    let line_at = |x0: f64, x1: f64, y: f64| vectorcraft_geom::SubPath::polyline(&[Point::new(x0, y), Point::new(x1, y)], false);
    let two = s
        .edit("two", |d, _| {
            let layer = d.layers[0].id;
            let id = d.alloc_id();
            let both = PathData::new(vec![line_at(0.0, 300.0, 100.0), line_at(50.0, 250.0, 120.0)]);
            d.insert(Some(layer), 0, vectorcraft_doc::Node::path(id, both, Default::default()))?;
            Ok(id)
        })
        .unwrap();
    line(&mut s, &[(100.0, 50.0), (100.0, 150.0)], false);
    line(&mut s, &[(200.0, 50.0), (200.0, 150.0)], false);
    // The middle of the lower subpath goes; the upper one stays whole, both in the one path.
    assert_eq!(s.execute(TRIM, &json!({"at": [150, 120]})).unwrap()["removed"], json!(1));
    let p = path(&s, two);
    let spans: Vec<(Point, Point)> = p.subpaths.iter().map(|sp| (sp.anchors[0].p, sp.anchors.last().unwrap().p)).collect();
    assert_eq!(spans.len(), 3, "{spans:?}");
    assert!(near(spans[0].0, 0.0, 100.0) && near(spans[0].1, 300.0, 100.0), "{spans:?}");
    assert!(near(spans[1].0, 50.0, 120.0) && near(spans[1].1, 100.0, 120.0), "{spans:?}");
    assert!(near(spans[2].0, 200.0, 120.0) && near(spans[2].1, 250.0, 120.0), "{spans:?}");
}

#[test]
fn a_clipping_path_is_not_trimmed_and_does_not_count_as_crossing() {
    let mut s = session();
    let (h, v1, _) = across_and_uprights(&mut s);
    s.edit("clip", |d, _| {
        if let NodeKind::Path { clipping, .. } = &mut d.node_mut(v1).unwrap().kind {
            *clipping = true;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(s.execute(TRIM, &json!({"at": [100, 60]})).unwrap()["removed"], json!(0));
    assert_eq!(path(&s, v1).anchor_count(), 2);
    // The line across is crossed by the second upright only: its stretch from the start goes.
    assert_eq!(s.execute(TRIM, &json!({"at": [150, 100]})).unwrap()["removed"], json!(1));
    assert!(near(ends(&s, h).0, 200.0, 100.0) && near(ends(&s, h).1, 300.0, 100.0));
    // Type over the rest of it takes no part either: nothing crosses the rest, and it goes whole.
    let t = NodeId(s.execute("text.create", &json!({"x": 220, "y": 115, "text": "HHHH", "size": 40})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(s.execute(TRIM, &json!({"rect": [215, 70, 70, 50]})).unwrap(), json!({"ids": [], "removed": 1}));
    assert!(!exists(&s, h) && exists(&s, t));
}

#[test]
fn the_members_of_a_compound_clipping_path_are_not_trimmed_and_do_not_count_as_crossing() {
    let mut s = session();
    let (h, v1, _) = across_and_uprights(&mut s);
    let extra = line(&mut s, &[(250.0, 250.0), (260.0, 260.0)], false);
    s.execute("select.set", &json!({"ids": [v1.0, extra.0]})).unwrap();
    let compound = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [h.0, compound.0]})).unwrap();
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let n = steps(&s);
    // A click on the clipping path's member changes nothing, and the rectangle over it neither.
    assert_eq!(s.execute(TRIM, &json!({"at": [100, 60]})).unwrap()["removed"], json!(0));
    assert_eq!(s.execute(TRIM, &json!({"rect": [245, 245, 20, 20]})).unwrap()["removed"], json!(0));
    assert_eq!((steps(&s), path(&s, v1).anchor_count(), path(&s, extra).anchor_count()), (n, 2, 2));
    // The clipped line across is crossed by the second upright only: its stretch from the start goes.
    assert_eq!(s.execute(TRIM, &json!({"at": [150, 100]})).unwrap()["removed"], json!(1));
    assert!(near(ends(&s, h).0, 200.0, 100.0) && near(ends(&s, h).1, 300.0, 100.0));
}

#[test]
fn in_isolation_mode_only_the_isolated_objects_paths_are_trimmed() {
    let mut s = session();
    let (h, v1, v2) = across_and_uprights(&mut s);
    s.execute("select.set", &json!({"ids": [v1.0, v2.0]})).unwrap();
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    // The line across is outside the group: a click or a rectangle on it leaves it whole.
    assert_eq!(s.execute(TRIM, &json!({"at": [150, 100]})).unwrap()["removed"], json!(0));
    assert_eq!(s.execute(TRIM, &json!({"rect": [140, 90, 20, 20]})).unwrap()["removed"], json!(0));
    assert!(near(ends(&s, h).0, 0.0, 100.0) && near(ends(&s, h).1, 300.0, 100.0));
    // It still crosses the uprights: a rectangle over the tops of both removes them down to it.
    assert_eq!(s.execute(TRIM, &json!({"rect": [90, 40, 120, 20]})).unwrap()["removed"], json!(2));
    assert!(near(ends(&s, v1).0, 100.0, 100.0) && near(ends(&s, v2).0, 200.0, 100.0));
    assert!(near(ends(&s, h).0, 0.0, 100.0) && near(ends(&s, h).1, 300.0, 100.0));
    // An isolated path is trimmed itself.
    s.execute("object.exitIsolation", &json!({})).unwrap();
    s.execute("object.isolate", &json!({"id": h.0})).unwrap();
    assert_eq!(s.execute(TRIM, &json!({"at": [150, 100]})).unwrap()["removed"], json!(1));
    assert!(near(ends(&s, h).1, 100.0, 100.0));
}

#[test]
fn a_rectangle_over_too_many_paths_to_check_is_refused() {
    let mut s = session();
    // 7,072 short lines apart from each other: each of them against every other is past the limit.
    s.edit("many", |d, _| {
        let layer = d.layers[0].id;
        for i in 0..7072 {
            let (x, y) = ((i % 100) as f64 * 3.0, (i / 100) as f64 * 3.0);
            let sp = vectorcraft_geom::SubPath::polyline(&[Point::new(x, y), Point::new(x + 1.0, y)], false);
            let id = d.alloc_id();
            let top = d.node(layer).unwrap().children().unwrap().len();
            d.insert(Some(layer), top, vectorcraft_doc::Node::path(id, PathData::single(sp), Default::default()))?;
        }
        Ok(())
    })
    .unwrap();
    let n = steps(&s);
    let err = s.execute(TRIM, &json!({"rect": [-10, -10, 400, 300]})).unwrap_err().to_string();
    assert!(err.contains("too many paths"), "{err}");
    assert_eq!((steps(&s), stack(&s).len()), (n, 7072));
    // A rectangle over a few of them is checked.
    assert_eq!(s.execute(TRIM, &json!({"rect": [-1, -1, 5, 2]})).unwrap()["removed"], json!(2));
}

#[test]
fn a_path_too_large_to_search_is_refused_only_when_touched() {
    let mut s = session();
    let (h, ..) = across_and_uprights(&mut s);
    // A border of 10,001 segments round all of them, more than a path to trim may have.
    let mut border: Vec<(f64, f64)> = (0..=10_001).map(|i| (-500.0 + i as f64 * 0.2, -500.0)).collect();
    border.push((-500.0, 1500.0));
    let b = line(&mut s, &border, true);
    // A rectangle and a line over the middle of the line across reach inside the border's box only.
    assert_eq!(s.execute(TRIM, &json!({"rect": [140, 90, 20, 20]})).unwrap()["removed"], json!(1));
    assert!(near(ends(&s, h).1, 100.0, 100.0));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.execute(TRIM, &json!({"points": [[150, 80], [150, 120]]})).unwrap()["removed"], json!(1));
    // A rectangle on the border is refused, and changes nothing.
    let n = steps(&s);
    let err = s.execute(TRIM, &json!({"rect": [-510, 0, 20, 20]})).unwrap_err().to_string();
    assert!(err.contains("too many or too tangled"), "{err}");
    assert_eq!((steps(&s), path(&s, b).anchor_count()), (n, 10_003));
}

#[test]
fn a_path_nothing_crosses_goes_whole_open_or_closed() {
    let mut s = session();
    let open = line(&mut s, &[(0.0, 10.0), (50.0, 10.0), (50.0, 40.0)], false);
    let closed = line(&mut s, &[(100.0, 10.0), (150.0, 10.0), (150.0, 40.0)], true);
    let n = steps(&s);
    assert_eq!(s.execute(TRIM, &json!({"at": [25, 10]})).unwrap(), json!({"ids": [], "removed": 1}));
    assert_eq!(s.execute(TRIM, &json!({"at": [125, 10]})).unwrap(), json!({"ids": [], "removed": 1}));
    assert!(!exists(&s, open) && !exists(&s, closed));
    assert_eq!(steps(&s), n + 2);
}

#[test]
fn a_path_that_ends_on_another_marks_a_place_and_paths_along_each_other_do_not() {
    let mut s = session();
    let h = line(&mut s, &[(0.0, 100.0), (300.0, 100.0)], false);
    let stub = line(&mut s, &[(150.0, 100.0), (150.0, 160.0)], false);
    // The stub ends on the line: the line's stretch stops there, and the stub is one stretch.
    s.execute(TRIM, &json!({"at": [50, 100]})).unwrap();
    assert!(near(ends(&s, h).0, 150.0, 100.0) && near(ends(&s, h).1, 300.0, 100.0));
    assert_eq!(s.execute(TRIM, &json!({"at": [150, 130]})).unwrap()["removed"], json!(1));
    assert!(!exists(&s, stub));
    // A second line along the first doesn't mark it: the lower one goes whole.
    let along = line(&mut s, &[(200.0, 100.0), (260.0, 100.0)], false);
    s.execute(TRIM, &json!({"rect": [270, 90, 10, 20]})).unwrap();
    assert!(!exists(&s, h) && exists(&s, along));
}

#[test]
fn a_self_crossing_path_loses_its_loop() {
    let mut s = session();
    // A bow: its first and third segments cross at (50, 50).
    let bow = line(&mut s, &[(0.0, 0.0), (100.0, 100.0), (100.0, 0.0), (0.0, 100.0)], false);
    let r = s.execute(TRIM, &json!({"at": [100, 50]})).unwrap();
    assert_eq!(r["removed"], json!(1));
    let made = ids(&r);
    assert_eq!(made.len(), 2);
    assert_eq!(made[0], bow);
    assert!(near(ends(&s, made[0]).1, 50.0, 50.0) && near(ends(&s, made[1]).0, 50.0, 50.0));
}

#[test]
fn compound_path_members_are_trimmed_and_an_emptied_compound_path_goes() {
    let mut s = session();
    let a = line(&mut s, &[(0.0, 100.0), (300.0, 100.0)], false);
    let b = line(&mut s, &[(100.0, 50.0), (100.0, 150.0)], false);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let compound = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.none", &json!({})).unwrap();
    let members = |s: &Session| s.doc().unwrap().doc.node(compound).map(|n| n.children().unwrap().len());
    assert_eq!(members(&s), Some(2));
    // The members cross each other: a middle stretch splits one member in the compound path.
    assert_eq!(s.execute(TRIM, &json!({"at": [100, 75]})).unwrap()["removed"], json!(1));
    assert_eq!(members(&s), Some(2));
    assert_eq!(s.execute(TRIM, &json!({"at": [100, 125]})).unwrap()["removed"], json!(1));
    assert_eq!(members(&s), Some(1));
    assert_eq!(s.execute(TRIM, &json!({"at": [200, 100]})).unwrap()["removed"], json!(1));
    assert_eq!(members(&s), None, "the emptied compound path is removed");
    assert!(stack(&s).is_empty());
}

#[test]
fn the_selection_stays_as_it_was_and_the_pieces_of_a_selected_path_are_selected() {
    let mut s = session();
    let (_, v1, _) = across_and_uprights(&mut s);
    s.execute("select.set", &json!({"ids": [v1.0]})).unwrap();
    s.execute(TRIM, &json!({"at": [150, 100]})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, [v1], "the trimmed path was not selected");
    let mut s = session();
    let (h, ..) = across_and_uprights(&mut s);
    s.execute("select.set", &json!({"ids": [h.0]})).unwrap();
    let made = ids(&s.execute(TRIM, &json!({"at": [150, 100]})).unwrap());
    assert_eq!(made.len(), 2);
    let sel = &s.doc().unwrap().selection;
    assert!(made.iter().all(|id| sel.contains(*id)), "{:?}", sel.objects);
    // The second piece now ends on an upright: nothing crosses it, and it goes whole.
    s.execute("select.set", &json!({"ids": [made[1].0]})).unwrap();
    assert_eq!(s.execute(TRIM, &json!({"at": [250, 100]})).unwrap(), json!({"ids": [], "removed": 1}));
    assert!(s.doc().unwrap().selection.is_empty(), "a removed path leaves the selection");
}

/// Run the command with `p`, and check that it leaves the selection exactly as changing it one
/// id at a time does: for each trimmed path in the plan's order, its anchors and segments let go,
/// a path with no pieces left removed, and each piece of a selected path added; then each emptied
/// compound path removed. Returns the selection.
fn same_selection_as_one_id_at_a_time(s: &mut Session, p: Value) -> Selection {
    let st = s.doc().unwrap();
    let pick = super::pick(&p).unwrap();
    let root = crate::cmd::ordinary_isolation(st);
    let plan: Vec<(NodeId, usize)> =
        plan::plan(&st.doc, root, &pick, Limits::COMMAND, &mut Stretches::default()).unwrap().into_iter().map(|t| (t.id, t.pieces.len())).collect();
    let mut compounds: Vec<NodeId> = plan
        .iter()
        .filter_map(|(id, _)| st.doc.parent_of(*id))
        .filter(|c| matches!(st.doc.node(*c).map(|n| &n.kind), Some(NodeKind::Compound { .. })))
        .collect();
    compounds.dedup();
    let mut want = st.selection.clone();
    let made = ids(&s.execute(TRIM, &p).unwrap());
    assert_eq!(made.len(), plan.iter().map(|(_, n)| n).sum::<usize>());
    let mut made = made.into_iter();
    for (id, n) in plan {
        let selected = want.contains(id);
        want.anchors.remove(&id);
        want.segments.remove(&id);
        let pieces: Vec<NodeId> = made.by_ref().take(n).collect();
        if pieces.is_empty() {
            want.remove(id);
        } else if selected {
            for piece in pieces {
                if !want.contains(piece) {
                    want.add(piece);
                }
            }
        }
    }
    let st = s.doc().unwrap();
    for c in compounds {
        if st.doc.node(c).is_none() {
            want.remove(c);
        }
    }
    want.prune(&st.doc);
    assert_eq!(st.selection, want);
    st.selection.clone()
}

/// Two lines across split by a rectangle, one of them selected; a selected line nothing crosses
/// and a selected compound path of two such lines, which the rectangle removes whole; and a
/// selected line elsewhere with an anchor and a segment direct-selected. Returns (the selected
/// line across, the line removed whole, the compound path, the line elsewhere).
fn trim_scene(s: &mut Session) -> (NodeId, NodeId, NodeId, NodeId) {
    let (h, ..) = across_and_uprights(s);
    line(s, &[(0.0, 120.0), (300.0, 120.0)], false);
    let alone = line(s, &[(145.0, 200.0), (155.0, 200.0)], false);
    let c1 = line(s, &[(145.0, 170.0), (155.0, 170.0)], false);
    let c2 = line(s, &[(145.0, 180.0), (155.0, 180.0)], false);
    s.execute("select.set", &json!({"ids": [c1.0, c2.0]})).unwrap();
    let compound = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let elsewhere = line(s, &[(0.0, 250.0), (10.0, 250.0), (20.0, 250.0)], false);
    s.execute("select.set", &json!({"ids": [elsewhere.0, h.0, alone.0, compound.0]})).unwrap();
    s.edit("direct", |_, sel| {
        sel.anchors.insert(elsewhere, [(0, 1)].into());
        sel.anchors.insert(h, [(0, 0)].into());
        sel.segments.insert(elsewhere, [(0, 0)].into());
        Ok(())
    })
    .unwrap();
    (h, alone, compound, elsewhere)
}

#[test]
fn the_selection_after_a_trim_is_the_one_changing_it_one_id_at_a_time_gives() {
    // A removed key object stops being the key.
    let mut s = session();
    let (h, alone, compound, elsewhere) = trim_scene(&mut s);
    s.edit("key", |_, sel| {
        sel.key = Some(alone);
        Ok(())
    })
    .unwrap();
    let sel = same_selection_as_one_id_at_a_time(&mut s, json!({"rect": [140, 90, 20, 120]}));
    assert_eq!(sel.key, None);
    assert!(!sel.contains(alone) && !sel.contains(compound));
    assert_eq!(&sel.objects[..2], [elsewhere, h]);
    assert_eq!(sel.objects.len(), 3, "{:?}", sel.objects);
    assert!(sel.anchors.contains_key(&elsewhere) && sel.segments.contains_key(&elsewhere) && !sel.anchors.contains_key(&h));
    // A key object that keeps its first piece stays the key.
    let mut s = session();
    let (h, alone, compound, _) = trim_scene(&mut s);
    s.edit("key", |_, sel| {
        sel.key = Some(h);
        Ok(())
    })
    .unwrap();
    let sel = same_selection_as_one_id_at_a_time(&mut s, json!({"rect": [140, 90, 20, 120]}));
    assert_eq!(sel.key, Some(h));
    assert!(!sel.contains(alone) && !sel.contains(compound));
    // Trimming only paths that are not selected leaves a targeted object targeted.
    let mut s = session();
    let (h, _, _, elsewhere) = trim_scene(&mut s);
    s.execute("select.set", &json!({"ids": [elsewhere.0]})).unwrap();
    s.edit("target", |_, sel| {
        sel.target = Some(elsewhere);
        Ok(())
    })
    .unwrap();
    let sel = same_selection_as_one_id_at_a_time(&mut s, json!({"rect": [140, 90, 20, 40]}));
    assert_eq!((sel.objects.as_slice(), sel.target), ([elsewhere].as_slice(), Some(elsewhere)));
    assert!(exists(&s, h));
    // A selected path that gains a piece changes the object selection, which lets go of the target.
    let mut s = session();
    let (h, _, _, elsewhere) = trim_scene(&mut s);
    s.execute("select.set", &json!({"ids": [elsewhere.0, h.0]})).unwrap();
    s.edit("target", |_, sel| {
        sel.target = Some(elsewhere);
        Ok(())
    })
    .unwrap();
    let sel = same_selection_as_one_id_at_a_time(&mut s, json!({"rect": [140, 90, 20, 40]}));
    assert_eq!((sel.objects.len(), sel.target), (3, None));
    // So does a selected path that is removed whole.
    let mut s = session();
    let (_, alone, _, elsewhere) = trim_scene(&mut s);
    s.execute("select.set", &json!({"ids": [elsewhere.0, alone.0]})).unwrap();
    s.edit("target", |_, sel| {
        sel.target = Some(elsewhere);
        Ok(())
    })
    .unwrap();
    let sel = same_selection_as_one_id_at_a_time(&mut s, json!({"rect": [140, 190, 20, 20]}));
    assert_eq!((sel.objects.as_slice(), sel.target), ([elsewhere].as_slice(), None));
}

#[test]
fn many_paths_far_from_the_trimmed_one_cost_no_crossing_search() {
    let mut s = session();
    let (h, ..) = across_and_uprights(&mut s);
    // 3,000 short lines far below, each crossing its neighbor.
    s.edit("many", |d, _| {
        let layer = d.layers[0].id;
        for i in 0..3000 {
            let x = (i % 100) as f64 * 3.0;
            let y = 1000.0 + (i / 100) as f64 * 3.0;
            let sp = vectorcraft_geom::SubPath::polyline(
                &[Point::new(x, y), Point::new(x + 5.0, y + 5.0), Point::new(x + 5.0, y), Point::new(x, y + 5.0)],
                false,
            );
            let id = d.alloc_id();
            let top = d.node(layer).unwrap().children().unwrap().len();
            d.insert(Some(layer), top, vectorcraft_doc::Node::path(id, PathData::single(sp), Default::default()))?;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(s.execute(TRIM, &json!({"at": [150, 100]})).unwrap()["removed"], json!(1));
    assert!(near(ends(&s, h).1, 100.0, 100.0));
    // A rectangle over all of them takes every stretch it touches.
    let r = s.execute(TRIM, &json!({"rect": [-10, 990, 400, 200]})).unwrap();
    assert!(r["removed"].as_u64().unwrap() >= 3000, "{}", r["removed"]);
    assert_eq!(stack(&s).len(), 4, "the line across in two pieces and the uprights");
}

#[test]
fn the_tool_trims_on_release_as_one_step_and_escape_drops_a_drag() {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};
    let mut s = session();
    let (h, v1, _) = across_and_uprights(&mut s);
    let h2 = line(&mut s, &[(0.0, 120.0), (300.0, 120.0)], false);
    let view = crate::ViewInfo { smart_guides: false, ..crate::ViewInfo::default() };
    s.select_tool("trimAtCrossings", view).unwrap();
    assert_eq!(s.tool_id(), "trimAtCrossings");
    let n = steps(&s);
    // A drag over the middle of both lines across: nothing changes until the release.
    s.pointer(&PointerEvent::new(PointerKind::Down, 140.0, 90.0), view).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 160.0, 130.0), view).unwrap();
    assert!(near(ends(&s, h).1, 300.0, 100.0));
    s.pointer(&PointerEvent::new(PointerKind::Up, 160.0, 130.0), view).unwrap();
    assert!(near(ends(&s, h).1, 100.0, 100.0) && near(ends(&s, h2).1, 100.0, 120.0));
    assert_eq!(steps(&s), n + 1, "one undo step for the drag");
    // Escape during a drag: the release removes nothing.
    s.pointer(&PointerEvent::new(PointerKind::Down, 90.0, 40.0), view).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 110.0, 60.0), view).unwrap();
    s.tool_key(ToolKey::Escape, Mods::default(), view).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 110.0, 60.0), view).unwrap();
    assert_eq!(steps(&s), n + 1);
    assert!(near(ends(&s, v1).0, 100.0, 50.0));
    // A click on the upright's top stretch removes it.
    s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 70.0), view).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 100.0, 70.0), view).unwrap();
    assert!(near(ends(&s, v1).0, 100.0, 100.0));
    assert_eq!(steps(&s), n + 2);
}

/// The ends of each red path the tool shows.
fn highlighted(s: &mut Session, view: crate::ViewInfo) -> Vec<(Point, Point)> {
    use vectorcraft_geom::kurbo::ParamCurve;
    s.overlays(view)
        .into_iter()
        .filter_map(|o| match o {
            vectorcraft_tools::Overlay::Path { path, color, .. } if color == vectorcraft_tools::builder::HIGHLIGHT_RED => {
                let segs: Vec<_> = path.segments().collect();
                Some((segs.first()?.start(), segs.last()?.end()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn hovering_with_the_tool_shows_the_stretch_its_click_removes() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut s = session();
    let (h, _, _) = across_and_uprights(&mut s);
    let hidden = line(&mut s, &[(0.0, 200.0), (300.0, 200.0)], false);
    set_flags(&mut s, hidden, false, false);
    let view = crate::ViewInfo { smart_guides: false, ..crate::ViewInfo::default() };
    s.select_tool("trimAtCrossings", view).unwrap();
    let n = steps(&s);
    // Over the middle of the line across: the stretch between the uprights, and nothing changes.
    s.pointer(&PointerEvent::new(PointerKind::Move, 150.0, 101.0), view).unwrap();
    let shown = highlighted(&mut s, view);
    assert!(matches!(shown.as_slice(), [(a, b)] if near(*a, 100.0, 100.0) && near(*b, 200.0, 100.0)), "{shown:?}");
    assert_eq!(steps(&s), n);
    // Over the hidden path and over empty canvas: nothing.
    s.pointer(&PointerEvent::new(PointerKind::Move, 150.0, 200.0), view).unwrap();
    assert!(highlighted(&mut s, view).is_empty());
    s.pointer(&PointerEvent::new(PointerKind::Move, 150.0, 250.0), view).unwrap();
    assert!(highlighted(&mut s, view).is_empty());
    // A click where the stretch was shown removes exactly it: the line keeps 0 to 100 and 200 to 300.
    s.pointer(&PointerEvent::new(PointerKind::Move, 150.0, 101.0), view).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 150.0, 101.0), view).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 150.0, 101.0), view).unwrap();
    assert_eq!(steps(&s), n + 1);
    assert!(near(ends(&s, h).0, 0.0, 100.0) && near(ends(&s, h).1, 100.0, 100.0));
    let rest = stack(&s).into_iter().find(|id| *id != h && path(&s, *id).subpaths.len() == 1 && near(ends(&s, *id).0, 200.0, 100.0));
    assert!(rest.is_some_and(|id| near(ends(&s, id).1, 300.0, 100.0)));
    assert!(highlighted(&mut s, view).is_empty(), "the edit drops the highlight");
}

/// The red paths the tool shows.
fn red(s: &mut Session, view: crate::ViewInfo) -> Vec<vectorcraft_geom::BezPath> {
    s.overlays(view)
        .into_iter()
        .filter_map(|o| match o {
            vectorcraft_tools::Overlay::Path { path, color, .. } if color == vectorcraft_tools::builder::HIGHLIGHT_RED => Some(path),
            _ => None,
        })
        .collect()
}

/// The length of every path in the first layer.
fn total_length(s: &Session) -> f64 {
    use vectorcraft_geom::kurbo::ParamCurveArclen;
    stack(s).into_iter().flat_map(|id| path(s, id).to_bezpath().segments().collect::<Vec<_>>()).map(|seg| seg.arclen(1e-9)).sum()
}

/// `act` removes exactly the red paths shown before it: the paths lose their length, and nothing
/// is left on the middle of any of them.
fn removes_what_was_shown(s: &mut Session, view: crate::ViewInfo, act: &dyn Fn(&mut Session)) {
    use vectorcraft_geom::kurbo::{ParamCurve, ParamCurveArclen};
    let shown = red(s, view);
    assert!(!shown.is_empty());
    let shown_length: f64 = shown.iter().flat_map(|p| p.segments().collect::<Vec<_>>()).map(|seg| seg.arclen(1e-9)).sum();
    let before = total_length(s);
    act(s);
    assert!((before - total_length(s) - shown_length).abs() < 1e-3, "{before} - {} != {shown_length}", total_length(s));
    for p in &shown {
        let segs: Vec<_> = p.segments().collect();
        let mid = segs[segs.len() / 2].eval(if segs.len() % 2 == 1 { 0.5 } else { 0.0 });
        let left = stack(s).into_iter().filter_map(|id| path(s, id).nearest(mid)).map(|n| n.4).fold(f64::INFINITY, f64::min);
        assert!(left > 0.5, "a path is left at {mid:?}");
    }
}

#[test]
fn what_the_tool_shows_before_a_click_or_a_release_is_what_the_command_removes() {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind};
    let view = crate::ViewInfo { smart_guides: false, ..crate::ViewInfo::default() };
    let ev = |kind, x, y| PointerEvent::new(kind, x, y);
    let alt = Mods { alt: true, ..Mods::default() };
    let setup = || {
        let mut s = session();
        s.execute("shape.ellipse", &json!({"x": 100, "y": 50, "width": 200, "height": 100})).unwrap();
        s.execute("select.none", &json!({})).unwrap();
        line(&mut s, &[(50.0, 100.0), (350.0, 100.0)], false);
        // A Z whose first and last sides cross.
        line(&mut s, &[(0.0, 160.0), (100.0, 260.0), (100.0, 160.0), (0.0, 260.0)], false);
        s.select_tool("trimAtCrossings", view).unwrap();
        s
    };
    // A click on the ellipse's top, on the line inside it and on the Z's loop.
    for (x, y) in [(200.0, 51.0), (200.0, 101.0), (101.0, 210.0)] {
        let mut s = setup();
        s.pointer(&ev(PointerKind::Move, x, y), view).unwrap();
        removes_what_was_shown(&mut s, view, &|s| {
            s.pointer(&ev(PointerKind::Down, x, y), view).unwrap();
            s.pointer(&ev(PointerKind::Up, x, y), view).unwrap();
        });
    }
    // A rectangle over the ellipse's left side and the line.
    let mut s = setup();
    s.pointer(&ev(PointerKind::Down, 90.0, 90.0), view).unwrap();
    s.pointer(&ev(PointerKind::Drag, 120.0, 110.0), view).unwrap();
    removes_what_was_shown(&mut s, view, &|s| {
        s.pointer(&ev(PointerKind::Up, 120.0, 110.0), view).unwrap();
    });
    // An Alt line through the ellipse's bottom and the Z's tail.
    let mut s = setup();
    s.pointer(&ev(PointerKind::Down, 200.0, 140.0).with_mods(alt), view).unwrap();
    s.pointer(&ev(PointerKind::Drag, 200.0, 170.0).with_mods(alt), view).unwrap();
    s.pointer(&ev(PointerKind::Drag, 10.0, 250.0).with_mods(alt), view).unwrap();
    removes_what_was_shown(&mut s, view, &|s| {
        s.pointer(&ev(PointerKind::Up, 10.0, 250.0).with_mods(alt), view).unwrap();
    });
}

#[test]
fn a_line_too_long_to_check_against_the_paths_is_refused() {
    let mut s = session();
    // A zigzag of 1,000 segments, and a line of 100,000 points inside its box that crosses none.
    let zigzag: Vec<(f64, f64)> = (0..=1000).map(|i| (i as f64 * 0.3, (i % 2) as f64 * 200.0)).collect();
    let z = line(&mut s, &zigzag, false);
    let (before, n) = (s.doc().unwrap().doc.clone(), steps(&s));
    let points: Vec<Value> = (0..100_000).map(|i| json!([150.0 + (i % 2) as f64 * 0.01, 150.0])).collect();
    let err = s.execute(TRIM, &json!({ "points": points })).unwrap_err().to_string();
    assert!(err.contains("too long"), "{err}");
    assert_eq!(steps(&s), n);
    assert!(*s.doc().unwrap().doc == *before);
    // A short line there is checked, and removes nothing.
    assert_eq!(s.execute(TRIM, &json!({"points": [[150.0, 150.0], [150.01, 150.0]]})).unwrap()["removed"], json!(0));
    assert!(exists(&s, z));
}

#[test]
fn bad_params_are_refused_and_change_nothing() {
    let mut s = session();
    across_and_uprights(&mut s);
    let (before, n) = (s.doc().unwrap().doc.clone(), steps(&s));
    for bad in [
        json!({}),
        json!({"at": null}),
        json!({"at": [150]}),
        json!({"at": [150, "y"]}),
        json!({"at": [150, 1e308]}),
        json!({"at": [150, 100], "tolerance": -1}),
        json!({"at": [150, 100], "tolerance": 101}),
        json!({"at": [150, 100], "tolerance": "3"}),
        json!({"at": [150, 100], "rect": [140, 90, 20, 40]}),
        json!({"rect": [140, 90, 20]}),
        json!({"rect": "all"}),
        json!({"rect": [0, 0, 1e9, 5]}),
        json!({"points": [[150, 80]]}),
        json!({"points": [[150, 80], [150]]}),
        json!({"points": "x"}),
        json!({"points": [[150, 80], [150, 130]], "at": [1, 2]}),
    ] {
        assert!(s.execute(TRIM, &bad).is_err(), "{bad}");
    }
    assert_eq!(steps(&s), n);
    assert!(*s.doc().unwrap().doc == *before);
}
