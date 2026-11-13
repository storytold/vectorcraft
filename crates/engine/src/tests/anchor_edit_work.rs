//! An edit of one anchor of a long selected path does not measure the path's segments
//! ([`vectorcraft_geom::path::measured_segments`]) to check that it stays within the canvas, and
//! an edit that leaves the anchor as it was records no undo step.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, Document, Node, NodeId};
use vectorcraft_geom::path::measured_segments;
use vectorcraft_geom::{Anchor, PathData, Point, SubPath};

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
