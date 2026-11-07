//! `document.move` (#5663): a document tab dragged along the tab strip moves its document to
//! another place among the open documents, and the current document stays the current one.

use serde_json::{Value, json};

use crate::*;

/// A session with documents A, B, C and D, in that order, D current.
fn four() -> Session {
    let mut s = Session::new();
    for name in ["A", "B", "C", "D"] {
        s.execute("file.new", &json!({"name": name, "created": null})).unwrap();
    }
    s
}

fn order(s: &Session) -> String {
    s.documents().iter().map(DocState::title).collect::<Vec<_>>().join("")
}

fn current(s: &Session) -> String {
    s.active().map(DocState::title).unwrap_or_default()
}

fn moved(s: &mut Session, p: Value) -> Value {
    s.execute("document.move", &p).unwrap_or_else(|e| panic!("document.move {p}: {e}"))
}

#[test]
fn a_document_moves_to_its_new_tab_and_the_others_shift() {
    let mut s = four();
    assert_eq!(order(&s), "ABCD");
    assert_eq!(moved(&mut s, json!({"from": 0, "to": 2})), json!({"index": 2}));
    assert_eq!(order(&s), "BCAD", "forward: the documents between shift left");
    assert_eq!(moved(&mut s, json!({"from": 3, "to": 0})), json!({"index": 0}));
    assert_eq!(order(&s), "DBCA", "backward: the documents between shift right");
    assert_eq!(moved(&mut s, json!({"from": 1, "to": 1})), json!({"index": 1}));
    assert_eq!(order(&s), "DBCA", "onto itself: nothing changes");
}

#[test]
fn the_current_document_stays_current_wherever_the_move_puts_it() {
    let mut s = four();
    assert!(s.set_active(1));
    // The current document itself moves.
    moved(&mut s, json!({"from": 1, "to": 3}));
    assert_eq!((order(&s), current(&s)), ("ACDB".into(), "B".into()));
    assert_eq!(s.active_index(), Some(3));
    // A document moves past the current one, which shifts by one.
    moved(&mut s, json!({"from": 3, "to": 0}));
    moved(&mut s, json!({"from": 3, "to": 0}));
    assert_eq!((order(&s), current(&s)), ("DBAC".into(), "B".into()));
    assert_eq!(s.active_index(), Some(1));
    // A move on one side of the current document leaves it where it is.
    moved(&mut s, json!({"from": 2, "to": 3}));
    assert_eq!((order(&s), current(&s)), ("DBCA".into(), "B".into()));
    assert_eq!(s.active_index(), Some(1));
}

#[test]
fn without_from_the_current_document_moves() {
    let mut s = four();
    assert!(s.set_active(2));
    moved(&mut s, json!({"to": 0}));
    assert_eq!((order(&s), current(&s)), ("CABD".into(), "C".into()));
}

#[test]
fn a_move_past_the_last_tab_ends_there_and_bad_indices_change_nothing() {
    let mut s = four();
    assert_eq!(moved(&mut s, json!({"from": 0, "to": 99})), json!({"index": 3}));
    assert_eq!(order(&s), "BCDA");
    assert_eq!(moved(&mut s, json!({"from": 0, "to": u64::MAX})), json!({"index": 3}));
    assert_eq!(order(&s), "CDAB");
    for p in [
        json!({"from": 4, "to": 0}),
        json!({"from": u64::MAX, "to": 0}),
        json!({"from": 0}),
        json!({"from": -1, "to": 0}),
        json!({"from": 0, "to": "1"}),
    ] {
        assert!(s.execute("document.move", &p).is_err(), "{p} is refused");
        assert_eq!(order(&s), "CDAB", "{p} changes nothing");
    }
    assert!(!s.move_document(0, 4));
    assert!(!s.move_document(4, 0));
    assert_eq!(order(&s), "CDAB");
}

#[test]
fn moving_needs_an_open_document_and_one_document_stays_put() {
    let mut s = Session::new();
    assert!(s.execute("document.move", &json!({"to": 0})).is_err(), "no document open");
    s.execute("file.new", &json!({"name": "A", "created": null})).unwrap();
    assert_eq!(moved(&mut s, json!({"to": 5})), json!({"index": 0}));
    assert_eq!((order(&s), current(&s)), ("A".into(), "A".into()));
}

#[test]
fn a_move_keeps_the_tool_and_the_documents_edits() {
    let mut s = four();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let resets = s.tool_resets;
    let nodes = s.doc().unwrap().doc.node_count();
    let history = super::history(&s);
    moved(&mut s, json!({"from": 3, "to": 0}));
    assert_eq!(super::history(&s), history, "a move is no undo step, and it keeps Redo");
    assert_eq!(s.tool_resets, resets, "no document was switched to, so the tool is not reset");
    assert_eq!(s.doc().unwrap().doc.node_count(), nodes);
    assert!(s.doc().unwrap().is_dirty(), "the document keeps its unsaved edit");
}

/// A recording replays the move before the steps after it, so an `index` they name is the same
/// document as when they were recorded.
#[test]
fn a_journaled_move_replays_in_order() {
    let mut s = four();
    for (id, p) in [
        ("document.move", json!({"from": 0, "to": 3})),
        ("document.activate", json!({"index": 3})),
        ("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})),
    ] {
        s.execute(id, &p).unwrap();
    }
    assert_eq!((order(&s), current(&s)), ("BCDA".into(), "A".into()));
    let mut r = Session::new();
    for (id, p) in &s.journal {
        r.execute(id, p).unwrap();
    }
    assert_eq!((order(&r), current(&r)), ("BCDA".into(), "A".into()));
    let counts = |s: &Session| s.documents().iter().map(|d| d.doc.node_count()).collect::<Vec<_>>();
    assert_eq!(counts(&r), counts(&s), "the rectangle is in the same document after the replay");
}
