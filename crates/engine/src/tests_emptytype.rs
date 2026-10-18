//! Point type placed with the Type tool and left empty is discarded when editing ends, without a
//! trace in the history; frames (dragged area type, type in or on a path) stay.

use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind, TextKind};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use super::*;

fn session() -> Session {
    // Type placed by the Type tools starts empty (Fill New Type Objects With Placeholder Text off).
    let mut s = Session::new();
    s.prefs.placeholder_text = false;
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn drag(s: &mut Session, from: (f64, f64), to: (f64, f64)) {
    let v = ViewInfo::default();
    s.pointer(&PointerEvent::new(PointerKind::Down, from.0, from.1), v).unwrap();
    if from != to {
        s.pointer(&PointerEvent::new(PointerKind::Drag, to.0, to.1), v).unwrap();
    }
    s.pointer(&PointerEvent::new(PointerKind::Up, to.0, to.1), v).unwrap();
}

fn click(s: &mut Session, x: f64, y: f64) {
    drag(s, (x, y), (x, y));
}

fn key(s: &mut Session, k: ToolKey) {
    s.tool_key(k, Mods::default(), ViewInfo::default()).unwrap();
}

/// The text objects in the document: (id, kind is point type, plain text).
fn texts(s: &Session) -> Vec<(NodeId, bool, String)> {
    let mut v = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            v.push((n.id, matches!(t.kind, TextKind::Point), t.plain_text()));
        }
    });
    v
}

fn undo_labels(s: &Session) -> Vec<String> {
    s.doc().unwrap().history.undo.iter().map(|e| e.label.clone()).collect()
}

#[test]
fn clicking_without_typing_leaves_no_text_and_no_history() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    // Click, then Escape.
    click(&mut s, 100.0, 100.0);
    assert_eq!(texts(&s).len(), 1, "the click places point type to edit");
    assert!(s.tool_wants_text());
    key(&mut s, ToolKey::Escape);
    assert!(texts(&s).is_empty());
    // Click, then switch tools.
    click(&mut s, 150.0, 100.0);
    s.select_tool("rectangle", v).unwrap();
    assert!(texts(&s).is_empty());
    // Click in two places: the second click discards the first type and places none (#2871); a
    // third places new type.
    s.select_tool("type", v).unwrap();
    click(&mut s, 100.0, 300.0);
    click(&mut s, 400.0, 300.0);
    assert!(texts(&s).is_empty());
    assert!(!s.tool_wants_text());
    click(&mut s, 400.0, 300.0);
    assert_eq!(texts(&s).len(), 1);
    s.select_tool("selection", v).unwrap();
    assert!(texts(&s).is_empty());
    assert!(undo_labels(&s).is_empty(), "{:?}", undo_labels(&s));
    let st = s.doc().unwrap();
    assert!(!st.is_dirty() && st.selection.is_empty());
}

#[test]
fn typed_text_stays_and_typing_it_away_leaves_nothing() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    click(&mut s, 100.0, 100.0);
    s.tool_text("Hi", v).unwrap();
    key(&mut s, ToolKey::Escape);
    assert_eq!(texts(&s).iter().map(|t| t.2.as_str()).collect::<Vec<_>>(), ["Hi"]);
    let kept = undo_labels(&s);
    assert_eq!(kept, ["Type", "Typing"]);
    // Typed, the caret moved (a step of its own), then deleted: all of it goes.
    click(&mut s, 300.0, 300.0);
    s.tool_text("a", v).unwrap();
    key(&mut s, ToolKey::Left);
    key(&mut s, ToolKey::Delete);
    key(&mut s, ToolKey::Escape);
    assert_eq!(texts(&s).len(), 1);
    assert_eq!(undo_labels(&s), kept);
}

/// A click on blank canvas while type is being edited ends the editing and makes no new type; the
/// Type tool stays, and the next click places point type (#2871, #2854).
#[test]
fn a_click_away_from_typed_text_ends_editing_without_new_type() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    click(&mut s, 100.0, 100.0);
    s.tool_text("Hi", v).unwrap();
    click(&mut s, 400.0, 300.0);
    assert_eq!(texts(&s).iter().map(|t| t.2.as_str()).collect::<Vec<_>>(), ["Hi"]);
    assert_eq!(s.tool_id(), "type");
    assert!(!s.tool_wants_text());
    assert_eq!(undo_labels(&s), ["Type", "Typing"]);
    click(&mut s, 400.0, 300.0);
    assert_eq!(texts(&s).len(), 2);
    assert!(s.tool_wants_text());
}

#[test]
fn frames_stay_when_left_empty() {
    let mut s = session();
    let v = ViewInfo::default();
    // A dragged area-type frame.
    s.select_tool("type", v).unwrap();
    drag(&mut s, (100.0, 100.0), (300.0, 200.0));
    key(&mut s, ToolKey::Escape);
    assert!(matches!(texts(&s).as_slice(), [(_, false, t)] if t.is_empty()));
    // Area type made from a path.
    s.execute("shape.rectangle", &json!({"x": 400, "y": 300, "width": 200, "height": 100})).unwrap();
    s.select_tool("areaType", v).unwrap();
    click(&mut s, 500.0, 300.0);
    s.select_tool("selection", v).unwrap();
    assert_eq!(texts(&s).len(), 2);
    assert_eq!(undo_labels(&s), ["Type", "Rectangle", "Area Type"]);
}

#[test]
fn a_command_that_deselects_the_text_ends_the_edit() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    click(&mut s, 100.0, 100.0);
    // Commands on the edited text (the Character panel) keep editing it.
    let id = texts(&s)[0].0;
    s.execute("text.setRangeStyle", &json!({"id": id.0, "size": 30})).unwrap();
    s.execute("document.inspect", &json!({})).unwrap();
    assert!(s.tool_wants_text());
    s.execute("select.none", &json!({})).unwrap();
    assert!(!s.tool_wants_text());
    assert!(texts(&s).is_empty());
    assert!(undo_labels(&s).is_empty(), "{:?}", undo_labels(&s));
    // A command that edits something else as well: the text goes in a step of its own.
    click(&mut s, 100.0, 100.0);
    s.execute("shape.rectangle", &json!({"x": 400, "y": 300, "width": 20, "height": 10})).unwrap();
    assert!(!s.tool_wants_text());
    assert!(texts(&s).is_empty());
    assert_eq!(undo_labels(&s), ["Type", "Rectangle", "Discard Empty Type"]);
}

#[test]
fn switching_documents_discards_it_in_the_document_left() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    click(&mut s, 100.0, 100.0);
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s.set_active(0);
    assert!(texts(&s).is_empty() && undo_labels(&s).is_empty());
}

#[test]
fn discard_empty_command() {
    let mut s = session();
    let r = s.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi"})).unwrap();
    assert_eq!(s.execute("text.discardEmpty", &json!({"id": r["id"]})).unwrap(), json!({"removed": false}));
    let rect = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    assert!(s.execute("text.discardEmpty", &json!({"id": rect["id"]})).is_err(), "not text");
    assert!(s.execute("text.discardEmpty", &json!({})).is_err(), "id required");
    // Created by the newest step: that step goes.
    let e = s.execute("text.create", &json!({"x": 10, "y": 80, "text": ""})).unwrap();
    assert_eq!(s.execute("text.discardEmpty", &json!({"id": e["id"]})).unwrap(), json!({"removed": true}));
    assert_eq!(undo_labels(&s), ["Type", "Rectangle"]);
    assert_eq!(texts(&s).len(), 1);
}
