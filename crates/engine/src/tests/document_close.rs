//! `file.close {index}`: closing a document tab other than the current one keeps the current
//! document current, and closing the current one makes its neighbor current.

use serde_json::json;

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

fn close(s: &mut Session, index: usize) {
    s.execute("file.close", &json!({"index": index})).unwrap_or_else(|e| panic!("file.close {index}: {e}"));
}

#[test]
fn closing_a_tab_before_the_current_one_keeps_the_current_document() {
    let mut s = four();
    close(&mut s, 0);
    assert_eq!((order(&s), current(&s)), ("BCD".into(), "D".into()));
    assert_eq!(s.active_index(), Some(2));
    assert!(s.set_active(1));
    close(&mut s, 0);
    assert_eq!((order(&s), current(&s)), ("CD".into(), "C".into()));
    assert_eq!(s.active_index(), Some(0));
}

#[test]
fn closing_a_tab_after_the_current_one_keeps_the_current_document() {
    let mut s = four();
    assert!(s.set_active(1));
    close(&mut s, 3);
    assert_eq!((order(&s), current(&s)), ("ABC".into(), "B".into()));
    close(&mut s, 2);
    assert_eq!((order(&s), current(&s)), ("AB".into(), "B".into()));
    assert_eq!(s.active_index(), Some(1));
}

#[test]
fn closing_the_current_tab_makes_its_neighbor_current() {
    let mut s = four();
    assert!(s.set_active(1));
    close(&mut s, 1);
    assert_eq!((order(&s), current(&s)), ("ACD".into(), "C".into()), "the tab that took its place");
    assert!(s.set_active(2));
    close(&mut s, 2);
    assert_eq!((order(&s), current(&s)), ("AC".into(), "C".into()), "the last tab: the one before it");
    s.execute("file.close", &json!({})).unwrap();
    assert_eq!((order(&s), current(&s)), ("A".into(), "A".into()), "without an index, the current one closes");
    close(&mut s, 0);
    assert!(s.active().is_none());
    assert!(s.execute("file.close", &json!({"index": 0})).is_err(), "no document left");
}

#[test]
fn closing_another_tab_keeps_the_tool_and_the_documents_edits() {
    let mut s = four();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let resets = s.tool_resets;
    let nodes = s.doc().unwrap().doc.node_count();
    let history = super::history(&s);
    close(&mut s, 1);
    assert_eq!(s.tool_resets, resets, "no document was switched to, so the tool is not reset");
    assert_eq!(super::history(&s), history);
    assert_eq!(s.doc().unwrap().doc.node_count(), nodes);
    assert!(s.doc().unwrap().is_dirty(), "the document keeps its unsaved edit");
    close(&mut s, 2);
    assert_ne!(s.tool_resets, resets, "closing the current document resets the tool");
}

#[test]
fn a_bad_index_closes_nothing() {
    let mut s = four();
    assert!(s.set_active(2));
    assert!(s.execute("file.close", &json!({"index": 4})).is_err());
    assert!(!s.close_document(4));
    assert_eq!((order(&s), current(&s)), ("ABCD".into(), "C".into()));
}
