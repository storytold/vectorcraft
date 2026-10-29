//! Object › Path › Smooth (`object.path.smooth`): the slider from minimum to maximum smoothing and
//! Auto-Smooth, as the Smooth slider help page describes
//! (https://helpx.adobe.com/illustrator/desktop/draw-shapes-and-paths/modify-paths/refine-path-segments-with-the-smooth-slider.html).

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_geom::kurbo::ParamCurveNearest;
use vectorcraft_geom::{ParamCurve, PathData, Point};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn path_of(s: &Session, id: NodeId) -> PathData {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Path { path, .. } => path.clone(),
        _ => panic!("not a path"),
    }
}

/// A path through `points`, selected: its id.
fn polyline(s: &mut Session, points: &[(f64, f64)], closed: bool) -> NodeId {
    let anchors: Vec<Value> = points.iter().map(|&(x, y)| json!({"x": x, "y": y})).collect();
    NodeId(s.execute("path.create", &json!({"anchors": anchors, "closed": closed})).unwrap()["id"].as_u64().unwrap())
}

/// A sine wave 300 pt long with an anchor every 3 pt, its anchors 1.5 pt above and below it in turn.
fn jagged_wave(s: &mut Session) -> NodeId {
    let pts: Vec<(f64, f64)> = (0..=100u32)
        .map(|i| (f64::from(i) * 3.0, 200.0 + 40.0 * (f64::from(i) * 3.0 / 50.0).sin() + if i.is_multiple_of(2) { 1.5 } else { -1.5 }))
        .collect();
    polyline(s, &pts, false)
}

/// A closed ring of radius `r` around (`cx`, 300) with 120 anchors, `jag` pt in and out in turn.
fn jagged_ring(s: &mut Session, cx: f64, r: f64, jag: f64) -> NodeId {
    let pts: Vec<(f64, f64)> = (0..120u32)
        .map(|i| {
            let a = f64::from(i) * std::f64::consts::TAU / 120.0;
            let rr = r + if i.is_multiple_of(2) { jag } else { -jag };
            (cx + rr * a.cos(), 300.0 + rr * a.sin())
        })
        .collect();
    polyline(s, &pts, true)
}

/// The farthest that 11 points on each segment of `from` lie from the nearest point of `to`.
fn farthest(from: &PathData, to: &PathData) -> f64 {
    let off = |p: Point| {
        to.subpaths
            .iter()
            .flat_map(|sp| (0..sp.segment_count()).map(move |i| sp.segment(i).nearest(p, 1e-9).distance_sq.sqrt()))
            .fold(f64::INFINITY, f64::min)
    };
    from.subpaths
        .iter()
        .flat_map(|sp| (0..sp.segment_count()).flat_map(move |i| (0..=10).map(move |k| sp.segment(i).eval(f64::from(k) / 10.0))))
        .map(off)
        .fold(0.0, f64::max)
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn minimum_smoothing_leaves_the_path_where_it_was() {
    let mut s = session();
    let id = jagged_wave(&mut s);
    let before = path_of(&s, id);
    let out = s.execute("object.path.smooth", &json!({"level": 0})).unwrap();
    assert_eq!(out["level"], json!(0.0));
    let after = path_of(&s, id);
    // 0.1 % of the longer side (300 pt), both ways.
    assert!(farthest(&before, &after) <= 0.3 + 1e-6 && farthest(&after, &before) <= 0.3 + 1e-6);
}

#[test]
fn more_smoothing_removes_the_jags_and_keeps_the_shape() {
    let mut s = session();
    let id = jagged_wave(&mut s);
    let before = path_of(&s, id);
    let mut counts = vec![];
    for level in [25, 50, 100] {
        let out = s.execute("object.path.smooth", &json!({ "level": level })).unwrap();
        let after = path_of(&s, id);
        assert_eq!(out["after"].as_u64().unwrap() as usize, after.anchor_count());
        counts.push(after.anchor_count());
        // Within the tolerance (2.5 % of 300 pt at most) plus the jag the refit smooths away.
        assert!(farthest(&before, &after) <= 7.5 + 1.5, "level {level}: {}", farthest(&before, &after));
        // The end points of an open path stay put.
        let (a, b) = (&before.subpaths[0].anchors, &after.subpaths[0].anchors);
        assert_eq!((a[0].p, a[a.len() - 1].p), (b[0].p, b[b.len() - 1].p));
        assert!(!after.subpaths[0].closed);
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert!(counts[0] < 101 / 2 && counts[2] <= 10, "{counts:?}");
    assert!(counts.windows(2).all(|w| w[0] >= w[1]), "{counts:?}");
}

#[test]
fn auto_smooth_uses_the_middle_level() {
    let mut s = session();
    let id = jagged_wave(&mut s);
    let out = s.execute("object.path.smooth", &json!({"auto": true, "level": 0})).unwrap();
    assert_eq!(out["level"], json!(50.0));
    let auto = path_of(&s, id);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("object.path.smooth", &json!({"level": 50})).unwrap();
    assert_eq!(path_of(&s, id), auto);
}

#[test]
fn closed_paths_and_compound_path_members_smooth_in_one_undo_step() {
    let mut s = session();
    let outer = jagged_ring(&mut s, 300.0, 100.0, 2.0);
    let inner = jagged_ring(&mut s, 300.0, 40.0, 1.0);
    s.execute("select.set", &json!({"ids": [outer.0, inner.0]})).unwrap();
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    let (o, i) = (path_of(&s, outer), path_of(&s, inner));
    let steps = undo_len(&s);
    let out = s.execute("object.path.smooth", &json!({"level": 100})).unwrap();
    assert_eq!(out["ids"].as_array().unwrap().len(), 2);
    assert_eq!(undo_len(&s), steps + 1);
    for id in [outer, inner] {
        let now = path_of(&s, id);
        assert!(now.subpaths[0].closed);
        assert!(now.anchor_count() < 120 / 4, "{}", now.anchor_count());
    }
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((path_of(&s, outer), path_of(&s, inner)), (o, i));
}

#[test]
fn smooths_the_objects_given() {
    let mut s = session();
    let a = jagged_wave(&mut s);
    let b = jagged_ring(&mut s, 300.0, 50.0, 1.0);
    let ring = path_of(&s, b);
    // `b` is selected; `ids` names `a` instead.
    let out = s.execute("object.path.smooth", &json!({"ids": [a.0], "level": 100})).unwrap();
    assert_eq!(out["ids"], json!([a.0]));
    assert_eq!(path_of(&s, b), ring);
}

#[test]
fn bad_params_are_refused_and_change_nothing() {
    let mut s = session();
    let id = jagged_wave(&mut s);
    let before = path_of(&s, id);
    let steps = undo_len(&s);
    for p in [json!({"level": "50"}), json!({"auto": true, "level": "max"}), json!({"auto": "yes"}), json!({"ids": "all"}), json!({"ids": [999_999]})]
    {
        assert!(s.execute("object.path.smooth", &p).is_err(), "{p}");
    }
    assert_eq!((path_of(&s, id), undo_len(&s)), (before, steps));
    // Levels past the ends count as the ends.
    assert_eq!(s.execute("object.path.smooth", &json!({"level": 250})).unwrap()["level"], json!(100.0));
}
