//! Simplify of the direct-selected part of a path: with anchors or segments of a path
//! direct-selected, `object.path.simplify` refits only each run of selected segments between its end
//! anchors, as the Simplify help page describes for part of a path picked with Direct Selection
//! (https://helpx.adobe.com/illustrator/desktop/draw-shapes-and-paths/modify-paths/manually-simplify-paths.html).

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_geom::kurbo::ParamCurveNearest;
use vectorcraft_geom::{ParamCurve, PathData, SubPath};

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

/// A path through `points`, closed or open, selected whole: its id.
fn polyline(s: &mut Session, points: &[(f64, f64)], closed: bool) -> NodeId {
    let anchors: Vec<Value> = points.iter().map(|&(x, y)| json!({"x": x, "y": y})).collect();
    NodeId(s.execute("path.create", &json!({"anchors": anchors, "closed": closed})).unwrap()["id"].as_u64().unwrap())
}

/// A sine wave of 101 anchors, 3 pt apart, 40 pt high.
fn wave(s: &mut Session) -> NodeId {
    let pts: Vec<(f64, f64)> = (0..=100).map(|i| (f64::from(i) * 3.0, 200.0 + 40.0 * (f64::from(i) * 3.0 / 50.0).sin())).collect();
    polyline(s, &pts, false)
}

/// Direct-select anchors `ranges` (inclusive) of the first subpath of `id`.
fn select_anchors(s: &mut Session, id: NodeId, ranges: &[(usize, usize)]) {
    let anchors: Vec<Value> = ranges.iter().flat_map(|&(a, b)| (a..=b).map(|i| json!([0, i]))).collect();
    s.execute("select.anchors", &json!({"id": id.0, "anchors": anchors})).unwrap();
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// The farthest any of 11 points along each of `original`'s segments `segs` lies from `fitted`'s
/// segments `fit`.
fn worst_offset(original: &SubPath, segs: impl Iterator<Item = usize>, fitted: &SubPath, fit: std::ops::Range<usize>) -> f64 {
    let off = |p| fit.clone().map(|i| fitted.segment(i % fitted.segment_count()).nearest(p, 1e-9).distance_sq.sqrt()).fold(f64::INFINITY, f64::min);
    segs.flat_map(|i| (0..=10).map(move |k| original.segment(i).eval(f64::from(k) / 10.0))).map(off).fold(0.0, f64::max)
}

/// With anchors 20 to 60 of a 101-anchor wave direct-selected, Simplify refits only the 40
/// segments between them: anchors 0–19 and 61–100 and the segments outside the run stay exactly as
/// they were, anchors 20 and 60 stay where they were with their handles on the side away from the
/// run, the run stays within the tolerance of the original, and it stays direct-selected. It is one
/// undo step, and undo brings back the path and the selection.
#[test]
fn simplify_refits_only_the_run_between_direct_selected_anchors() {
    let mut s = session();
    let id = wave(&mut s);
    let original = path_of(&s, id).subpaths[0].clone();
    select_anchors(&mut s, id, &[(20, 60)]);
    let (doc, selection, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().selection.clone(), undo_len(&s));
    let out = s.execute("object.path.simplify", &json!({"tolerance": 1})).unwrap();
    let p = path_of(&s, id);
    let sp = &p.subpaths[0];
    let len = sp.anchors.len();
    assert_eq!(out["after"].as_u64(), Some(len as u64));
    assert!(len < 101 - 30, "{len}");
    let end = len - 41;
    assert_eq!(sp.anchors[..20], original.anchors[..20], "the anchors before the run");
    assert_eq!(sp.anchors[end + 1..], original.anchors[61..], "the anchors after the run");
    assert_eq!((sp.anchors[20].p, sp.anchors[20].h_in), (original.anchors[20].p, original.anchors[20].h_in));
    assert_eq!((sp.anchors[end].p, sp.anchors[end].h_out), (original.anchors[60].p, original.anchors[60].h_out));
    for i in 0..20 {
        assert_eq!(sp.segment(i), original.segment(i));
        assert_eq!(sp.segment(end + i), original.segment(60 + i));
    }
    assert!(worst_offset(&original, 20..60, sp, 20..end) <= 1.0 + 1e-6);
    let sel = &s.doc().unwrap().selection;
    assert_eq!(sel.anchors.get(&id), Some(&(20..=end).map(|i| (0, i)).collect()), "the run stays direct-selected");
    assert_eq!(undo_len(&s), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, doc);
    assert_eq!(s.doc().unwrap().selection, selection);
}

/// Two runs of one path simplify independently, in one undo step: the result is the first run as
/// Simplify makes it alone, the anchors between the runs as they were, and the second run as
/// Simplify makes it alone.
#[test]
fn separate_runs_on_one_path_simplify_independently_in_one_step() {
    let alone = |ranges: &[(usize, usize)]| {
        let mut s = session();
        let id = wave(&mut s);
        select_anchors(&mut s, id, ranges);
        s.execute("object.path.simplify", &json!({"tolerance": 1})).unwrap();
        path_of(&s, id).subpaths[0].anchors.clone()
    };
    let (first, second) = (alone(&[(10, 30)]), alone(&[(60, 80)]));
    let mut s = session();
    let id = wave(&mut s);
    let original = path_of(&s, id).subpaths[0].clone();
    select_anchors(&mut s, id, &[(10, 30), (60, 80)]);
    let undo = undo_len(&s);
    s.execute("object.path.simplify", &json!({"tolerance": 1})).unwrap();
    assert_eq!(undo_len(&s), undo + 1, "one undo step");
    let both = path_of(&s, id).subpaths[0].anchors.clone();
    // Where old anchor 30 is in the first result, and old anchor 80 in the second.
    let (a30, a80) = (first.len() - 71, second.len() - 21);
    assert!(a30 < 30 && a80 < 80, "both runs lose anchors");
    let mid = a30 + 30;
    assert_eq!(both[..=a30], first[..=a30]);
    assert_eq!(both[a30 + 1..mid], original.anchors[31..60], "the anchors between the runs");
    assert_eq!(both[mid..], second[60..]);
    let sel: BTreeSet<_> = (10..=a30).chain(mid..=mid + a80 - 60).map(|i| (0, i)).collect();
    assert_eq!(s.doc().unwrap().selection.anchors.get(&id), Some(&sel));
}

/// On a closed 80-gon, a run from anchor 70 through the start point to anchor 10 simplifies as one
/// run: anchor 0 goes, anchors 11–69 stay exactly as they were (the path now starts at anchor 10),
/// anchors 10 and 70 stay where they were with their outer handles, and the path stays closed.
#[test]
fn a_run_across_the_start_of_a_closed_path_simplifies_as_one() {
    let mut s = session();
    let pts: Vec<(f64, f64)> =
        (0..80).map(|i| (f64::from(i) * std::f64::consts::TAU / 80.0).sin_cos()).map(|(y, x)| (300.0 + 100.0 * x, 300.0 + 100.0 * y)).collect();
    let id = polyline(&mut s, &pts, true);
    let original = path_of(&s, id).subpaths[0].clone();
    select_anchors(&mut s, id, &[(70, 79), (0, 10)]);
    s.execute("object.path.simplify", &json!({"tolerance": 1})).unwrap();
    let sp = path_of(&s, id).subpaths[0].clone();
    assert!(sp.closed);
    assert!(sp.anchors.len() < 80 - 10, "{}", sp.anchors.len());
    assert!(!sp.anchors.iter().any(|a| a.p == original.anchors[0].p), "anchor 0 was inside the run");
    assert_eq!(sp.anchors[1..60], original.anchors[11..70]);
    assert_eq!((sp.anchors[0].p, sp.anchors[0].h_out), (original.anchors[10].p, original.anchors[10].h_out));
    assert_eq!((sp.anchors[60].p, sp.anchors[60].h_in), (original.anchors[70].p, original.anchors[70].h_in));
    for i in 0..60 {
        assert_eq!(sp.segment(i), original.segment(10 + i));
    }
    let n = sp.anchors.len();
    assert!(worst_offset(&original, (70..80).chain(0..10), &sp, 60..n) <= 1.0 + 1e-6);
    let sel: BTreeSet<_> = std::iter::once((0, 0)).chain((60..n).map(|i| (0, i))).collect();
    assert_eq!(s.doc().unwrap().selection.anchors.get(&id), Some(&sel));
}

/// Segments picked with the Direct Selection tool simplify the same way, and stay picked as
/// segments.
#[test]
fn picked_segments_simplify_and_stay_picked() {
    let mut s = session();
    let id = wave(&mut s);
    let original = path_of(&s, id).subpaths[0].clone();
    let segs: Vec<Value> = (20..60).map(|i| json!([0, i])).collect();
    s.execute("select.segments", &json!({"id": id.0, "segments": segs})).unwrap();
    s.execute("object.path.simplify", &json!({"tolerance": 1})).unwrap();
    let sp = path_of(&s, id).subpaths[0].clone();
    let end = sp.anchors.len() - 41;
    assert_eq!(sp.anchors[..20], original.anchors[..20]);
    assert_eq!(sp.anchors[end + 1..], original.anchors[61..]);
    let sel = &s.doc().unwrap().selection;
    assert_eq!(sel.segments.get(&id), Some(&(20..end).map(|i| (0, i)).collect()));
    assert_eq!(sel.anchors.get(&id), Some(&(20..=end).map(|i| (0, i)).collect()));
}

/// `wholePaths` simplifies a partly selected path whole, as Simplify with it selected whole does.
#[test]
fn whole_paths_ignores_the_direct_selection() {
    let mut s = session();
    let id = wave(&mut s);
    let original = path_of(&s, id);
    select_anchors(&mut s, id, &[(20, 60)]);
    s.execute("object.path.simplify", &json!({"tolerance": 1, "wholePaths": true})).unwrap();
    let opts = vectorcraft_pathops::SimplifyOptions { tolerance: 1.0, ..Default::default() };
    assert_eq!(path_of(&s, id), vectorcraft_pathops::simplify_with(&original, &opts));
    assert!(s.doc().unwrap().selection.anchors.is_empty());
}

/// The Simplify slider's preview, at a level or at Auto-Simplify's, simplifies only the run too, and
/// committing it is what the command does.
#[test]
fn slider_preview_and_auto_simplify_only_the_run() {
    for p in [json!({"level": 40}), json!({"auto": true})] {
        let mut s = session();
        let id = wave(&mut s);
        let original = path_of(&s, id).subpaths[0].clone();
        select_anchors(&mut s, id, &[(20, 60)]);
        let undo = undo_len(&s);
        s.begin_interaction("Simplify").unwrap();
        s.preview("object.path.simplify", &json!({"level": 90})).unwrap();
        s.preview("object.path.simplify", &p).unwrap();
        s.commit_interaction().unwrap();
        assert_eq!(undo_len(&s), undo + 1, "{p}");
        let previewed = path_of(&s, id);
        let sp = &previewed.subpaths[0];
        assert!(sp.anchors.len() < 101, "{p}");
        assert_eq!(sp.anchors[..20], original.anchors[..20], "{p}");
        assert_eq!(sp.anchors[sp.anchors.len() - 40..], original.anchors[61..], "{p}");
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("object.path.simplify", &p).unwrap();
        assert_eq!(path_of(&s, id), previewed, "{p}");
    }
}

/// A run Simplify can't make simpler (two sides of a rectangle, which meet at a corner) is left as
/// it is: no undo step, and the rectangle stays a live shape.
#[test]
fn a_run_that_cannot_be_simpler_changes_nothing() {
    let mut s = session();
    let id = NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 60})).unwrap()["id"].as_u64().unwrap());
    let before = s.doc().unwrap().doc.clone();
    select_anchors(&mut s, id, &[(0, 2)]);
    let undo = undo_len(&s);
    s.execute("object.path.simplify", &json!({"tolerance": 5})).unwrap();
    assert_eq!(undo_len(&s), undo);
    assert_eq!(s.doc().unwrap().doc, before);
}
