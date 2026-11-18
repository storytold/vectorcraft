//! An edit of one anchor of a long selected path does not measure the path's segments
//! ([`vectorcraft_geom::path::measured_segments`]) to check that it stays within the canvas, its
//! undo step keeps the chunk of anchors it changed rather than a copy of every anchor, and an edit
//! that leaves the anchor as it was records no undo step.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, Document, Node, NodeId};
use vectorcraft_geom::path::measured_segments;
use vectorcraft_geom::{Anchor, AnchorKind, PathData, Point, SubPath, Vec2};

use crate::*;

/// Most segments one anchor edit may measure, however many anchors the path has.
const MOST_MEASURED: u64 = 16;

/// A session whose document holds one open zigzag path of `n` anchors, selected, with its anchor 5
/// direct-selected.
fn long_path(n: usize) -> (Session, NodeId) {
    let mut d = Document::new(2000.0, 1000.0);
    let anchors = (0..n).map(|i| Anchor::corner(Point::new(i as f64 * 0.1, if i % 2 == 0 { 100.0 } else { 110.0 }))).collect();
    let id = d.alloc_id();
    let node = Node::path(id, PathData::single(SubPath::new(anchors, false)), Appearance::default_art());
    if let Some(ch) = Arc::make_mut(&mut d.layers[0]).children_mut() {
        ch.push(Arc::new(node));
    }
    let mut s = Session::new();
    s.add_document(d, None);
    let st = s.doc_mut().unwrap();
    st.selection.set([id]);
    st.selection.anchors.insert(id, BTreeSet::from([(0, 5)]));
    (s, id)
}

fn anchor(s: &Session, id: NodeId, ai: usize) -> Anchor {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0].anchors[ai]
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// Runs `cmd` with `p` and returns how many segments it measured.
fn measured(s: &mut Session, cmd: &str, p: Value) -> u64 {
    let before = measured_segments();
    s.execute(cmd, &p).unwrap();
    measured_segments() - before
}

#[test]
fn one_anchor_edit_of_a_long_path_measures_few_segments() {
    for n in [100, 10_000] {
        let (mut s, id) = long_path(n);
        let a = anchor(&s, id, 5);
        let m = measured(&mut s, "path.moveAnchor", json!({"id": id.0, "anchor": 5, "x": 3, "y": 50}));
        assert!(m <= MOST_MEASURED, "path.moveAnchor on {n} anchors measured {m} segments");
        assert_eq!(anchor(&s, id, 5).p, Point::new(3.0, 50.0));
        let m = measured(&mut s, "path.setHandle", json!({"id": id.0, "anchor": 5, "which": "out", "x": 4, "y": 40}));
        assert!(m <= MOST_MEASURED, "path.setHandle on {n} anchors measured {m} segments");
        assert_eq!(anchor(&s, id, 5).h_out, Point::new(4.0, 40.0));
        let m = measured(&mut s, "path.moveAnchors", json!({"dx": 1, "dy": 0}));
        assert!(m <= MOST_MEASURED, "path.moveAnchors on {n} anchors measured {m} segments");
        let m = measured(&mut s, "object.nudge", json!({"dx": 1, "dy": 0}));
        assert!(m <= MOST_MEASURED, "object.nudge on {n} anchors measured {m} segments");
        assert_eq!(anchor(&s, id, 5).p, Point::new(5.0, 50.0));
        // Only anchor 5 moved, and undoing the four edits gives it back.
        assert_eq!(anchor(&s, id, 4), Anchor::corner(Point::new(4.0 * 0.1, 100.0)));
        assert_eq!(anchor(&s, id, 6), Anchor::corner(Point::new(6.0 * 0.1, 100.0)));
        for _ in 0..4 {
            s.execute("edit.undo", &json!({})).unwrap();
        }
        assert_eq!(anchor(&s, id, 5), a);
    }
}

/// Most bytes the undo step of one anchor edit may keep, however many anchors the path has: about
/// one chunk of anchors and the node, with room for the chunk tables of 200,000 anchors.
const MOST_STEP_BYTES: usize = 48 * 1024;

#[test]
fn one_anchor_edit_of_a_long_path_keeps_one_chunk_in_its_undo_step() {
    for n in [10_000, 200_000] {
        let (mut s, id) = long_path(n);
        let original = s.doc().unwrap().doc.clone();
        let edits = [
            ("path.moveAnchor", json!({"id": id.0, "anchor": 5, "x": 3, "y": 50})),
            ("path.setHandle", json!({"id": id.0, "anchor": 5, "which": "out", "x": 4, "y": 40})),
            ("path.moveAnchors", json!({"dx": 1, "dy": 0})),
            ("object.nudge", json!({"dx": 1, "dy": 0})),
        ];
        let mut docs = vec![];
        for (cmd, p) in &edits {
            s.execute(cmd, p).unwrap();
            let step = s.doc().unwrap().history.undo.last().unwrap().bytes;
            assert!(step <= MOST_STEP_BYTES, "{cmd} on {n} anchors kept {step} bytes in its undo step");
            docs.push(s.doc().unwrap().doc.clone());
        }
        assert_eq!(anchor(&s, id, 5).p, Point::new(5.0, 50.0));
        assert_eq!(anchor(&s, id, 5).h_out, Point::new(6.0, 40.0));
        assert_eq!(anchor(&s, id, n - 1), original.node(id).unwrap().path_data().unwrap().subpaths[0].anchors[n - 1]);
        // Undo gives back each version exactly, and redo the edits, each step as small.
        for k in (0..edits.len()).rev() {
            s.execute("edit.undo", &json!({})).unwrap();
            let expected = if k == 0 { &original } else { &docs[k - 1] };
            assert!(s.doc().unwrap().doc == *expected, "undo of {} on {n} anchors", edits[k].0);
            let redo = s.doc().unwrap().history.redo.last().unwrap().bytes;
            assert!(redo <= MOST_STEP_BYTES, "redo of {} on {n} anchors keeps {redo} bytes", edits[k].0);
        }
        for (k, doc) in docs.iter().enumerate() {
            s.execute("edit.redo", &json!({})).unwrap();
            assert!(s.doc().unwrap().doc == *doc, "redo of {} on {n} anchors", edits[k].0);
        }
    }
}

/// A handle past the canvas is measured: the edit stands while the curve stays within the canvas,
/// and is refused once the curve leaves it.
#[test]
fn a_handle_past_the_canvas_is_measured() {
    let (mut s, id) = long_path(10_000);
    // The curve from anchor 5 reaches 4/9 of the way to its outgoing handle.
    let x = 2.0 * MAX_COORD;
    s.execute("path.setHandle", &json!({"id": id.0, "anchor": 5, "which": "out", "x": x, "y": 110})).unwrap();
    assert_eq!(anchor(&s, id, 5).h_out, Point::new(x, 110.0));
    let before = s.doc().unwrap().doc.clone();
    let undo = undo_steps(&s);
    let x = 3.0 * MAX_COORD;
    assert!(s.execute("path.setHandle", &json!({"id": id.0, "anchor": 5, "which": "out", "x": x, "y": 110})).is_err());
    assert!(s.doc().unwrap().doc == before, "the refused edit changed the document");
    assert_eq!(undo_steps(&s), undo);
}

/// `path.moveAnchor` to where the anchor is and `path.setHandle` to where the handle is change
/// nothing and record no undo step.
#[test]
fn an_anchor_edit_that_changes_nothing_records_no_undo_step() {
    let (mut s, id) = long_path(100);
    let a = anchor(&s, id, 5);
    s.execute("path.moveAnchor", &json!({"id": id.0, "anchor": 5, "x": a.p.x, "y": a.p.y})).unwrap();
    assert_eq!(undo_steps(&s), 0, "path.moveAnchor recorded an undo step");
    s.execute("path.setHandle", &json!({"id": id.0, "anchor": 5, "which": "out", "x": a.h_out.x, "y": a.h_out.y})).unwrap();
    assert_eq!(undo_steps(&s), 0, "path.setHandle recorded an undo step");
    assert_eq!(anchor(&s, id, 5), a);
    // An anchor that is not there is still refused.
    assert!(s.execute("path.moveAnchor", &json!({"id": id.0, "anchor": 100, "x": 0, "y": 0})).is_err());
    assert!(s.execute("path.setHandle", &json!({"id": id.0, "anchor": 100, "which": "out", "x": 0, "y": 0})).is_err());
    // A change still records one.
    s.execute("path.moveAnchor", &json!({"id": id.0, "anchor": 5, "x": a.p.x + 1.0, "y": a.p.y})).unwrap();
    assert_eq!(undo_steps(&s), 1);
}

/// `path.setHandle` to where the handle already is, on a smooth anchor whose other handle is not in
/// line with it, leaves both handles as they are and records no undo step. With `independent` it
/// makes the anchor a corner, which records one.
#[test]
fn a_handle_already_in_place_on_a_smooth_anchor_records_no_undo_step() {
    let (mut s, id) = long_path(100);
    let p = anchor(&s, id, 5).p;
    let smooth = Anchor { p, h_in: p + Vec2::new(-3.0, 0.0), h_out: p + Vec2::new(0.0, 4.0), kind: AnchorKind::Smooth };
    let path = Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().path_data_mut().unwrap();
    path.subpaths[0].anchors[5] = smooth;
    for which in ["out", "in"] {
        let h = if which == "out" { smooth.h_out } else { smooth.h_in };
        s.execute("path.setHandle", &json!({"id": id.0, "anchor": 5, "which": which, "x": h.x, "y": h.y})).unwrap();
        assert_eq!(undo_steps(&s), 0, "path.setHandle on the {which} handle recorded an undo step");
        assert_eq!(anchor(&s, id, 5), smooth, "path.setHandle on the {which} handle changed the anchor");
    }
    s.execute("path.setHandle", &json!({"id": id.0, "anchor": 5, "which": "out", "x": smooth.h_out.x, "y": smooth.h_out.y, "independent": true}))
        .unwrap();
    assert_eq!(undo_steps(&s), 1);
    assert_eq!(anchor(&s, id, 5).kind, AnchorKind::Corner);
    // On a corner, the same independent request changes nothing.
    s.execute("path.setHandle", &json!({"id": id.0, "anchor": 5, "which": "out", "x": smooth.h_out.x, "y": smooth.h_out.y, "independent": true}))
        .unwrap();
    assert_eq!(undo_steps(&s), 1);
}

/// `path.setHandle` with `link` makes a corner smooth: the other handle turns in line at its own
/// length, a retracted one is pulled out to the moved handle's length where a segment leaves the
/// anchor on its side and stays retracted at an open end. A handle already in place on a corner
/// still links; a request that changes nothing records no undo step; `link` that is not a boolean
/// or goes with `independent` is refused.
#[test]
fn a_linked_handle_makes_the_anchor_smooth() {
    let (mut s, id) = long_path(100);
    let p = anchor(&s, id, 5).p;
    let corner = Anchor { p, h_in: p + Vec2::new(-6.0, 8.0), h_out: p + Vec2::new(3.0, 0.0), kind: AnchorKind::Corner };
    let path = Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().path_data_mut().unwrap();
    path.subpaths[0].anchors[5] = corner;
    let link = |x: f64, y: f64, ai: usize| json!({"id": id.0, "anchor": ai, "which": "out", "x": x, "y": y, "link": true});
    let near = |a: Point, b: Point| a.distance(b) < 1e-9;

    // The out handle stays where it is; the corner becomes smooth, the in handle turned opposite it.
    s.execute("path.setHandle", &link(corner.h_out.x, corner.h_out.y, 5)).unwrap();
    assert_eq!(undo_steps(&s), 1);
    let a = anchor(&s, id, 5);
    assert_eq!((a.kind, a.h_out), (AnchorKind::Smooth, corner.h_out));
    assert!(near(a.h_in, p + Vec2::new(-10.0, 0.0)), "{a:?}");
    // Moved: the in handle keeps its length of 10.
    let to = p + Vec2::new(4.0, 3.0);
    s.execute("path.setHandle", &link(to.x, to.y, 5)).unwrap();
    assert_eq!(undo_steps(&s), 2);
    let a = anchor(&s, id, 5);
    assert!(a.kind == AnchorKind::Smooth && a.h_out == to && near(a.h_in, p + Vec2::new(-8.0, -6.0)), "{a:?}");
    // The same request again changes nothing.
    s.execute("path.setHandle", &link(to.x, to.y, 5)).unwrap();
    assert_eq!(undo_steps(&s), 2);

    // A retracted in handle on an inner anchor is pulled out; at the open start it stays retracted.
    let q = anchor(&s, id, 7).p;
    s.execute("path.setHandle", &link(q.x, q.y + 5.0, 7)).unwrap();
    let a = anchor(&s, id, 7);
    assert!(a.kind == AnchorKind::Smooth && near(a.h_in, q + Vec2::new(0.0, -5.0)), "{a:?}");
    let q = anchor(&s, id, 0).p;
    s.execute("path.setHandle", &link(q.x, q.y + 5.0, 0)).unwrap();
    let a = anchor(&s, id, 0);
    assert!(a.kind == AnchorKind::Smooth && a.h_in == q && a.h_out == q + Vec2::new(0.0, 5.0), "{a:?}");

    let before = s.doc().unwrap().doc.clone();
    let undo = undo_steps(&s);
    for bad in [
        json!({"id": id.0, "anchor": 5, "which": "out", "x": 0, "y": 0, "link": "yes"}),
        json!({"id": id.0, "anchor": 5, "which": "out", "x": 0, "y": 0, "link": 1}),
        json!({"id": id.0, "anchor": 5, "which": "out", "x": 0, "y": 0, "link": true, "independent": true}),
        json!({"id": id.0, "anchor": 5, "which": "out", "x": 0, "y": 0, "link": true, "independent": "no"}),
        json!({"id": id.0, "anchor": 100, "which": "out", "x": 0, "y": 0, "link": true}),
        json!({"id": id.0, "anchor": 7, "which": "out", "x": 1.7e308, "y": 1.7e308, "link": true}),
        json!({"id": id.0, "anchor": 7, "which": "out", "x": 0, "y": -1e9, "link": true}),
    ] {
        assert!(s.execute("path.setHandle", &bad).is_err(), "{bad}");
    }
    assert!(s.doc().unwrap().doc == before, "a refused request changed the document");
    assert_eq!(undo_steps(&s), undo);
}
