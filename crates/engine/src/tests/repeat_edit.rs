//! Editing a repeat's source art (#5054): a double-click on a repeat with the Selection tool
//! enters isolation mode on it; there a double-click on its type edits the text, and the repeat
//! keeps its settings and draws the new text in every copy.

use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::*;

fn click(s: &mut Session, kind: PointerKind, x: f64, y: f64) -> Vec<UiRequest> {
    s.pointer(&PointerEvent::new(kind, x, y), ViewInfo { smart_guides: false, ..ViewInfo::default() }).unwrap()
}

#[test]
fn a_double_click_on_a_repeat_isolates_it_to_edit_its_text() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let text = NodeId(s.execute("text.create", &json!({"x": 50, "y": 60, "text": "Alice", "size": 24})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [text.0]})).unwrap();
    let repeat = NodeId(s.execute("object.repeat.grid", &json!({})).unwrap()["id"].as_u64().unwrap());
    let spec = |s: &Session| match &s.doc().unwrap().doc.node(repeat).unwrap().kind {
        NodeKind::Repeat(r) => serde_json::to_value(r).unwrap(),
        k => panic!("not a repeat: {k:?}"),
    };
    let before = spec(&s);
    s.select_tool("selection", ViewInfo::default()).unwrap();
    let b = s.doc().unwrap().doc.node(text).unwrap().geometric_bounds().unwrap();
    let (x, y) = (b.center().x, b.center().y);
    click(&mut s, PointerKind::DoubleClick, x, y);
    assert_eq!(s.doc().unwrap().isolation, Some(repeat), "the double-click isolates the repeat");
    // Inside it, a double-click on the type asks for the Type tool, which edits that text.
    let asked = click(&mut s, PointerKind::DoubleClick, x, y);
    assert!(asked.iter().any(|r| matches!(r, UiRequest::SwitchTool(t) if t == "type")), "{asked:?}");
    s.execute("text.setText", &json!({"id": text.0, "text": "Bob"})).unwrap();
    let NodeKind::Text(t) = &s.doc().unwrap().doc.node(text).unwrap().kind else { panic!("not type") };
    assert_eq!(t.plain_text(), "Bob");
    assert_eq!(s.doc().unwrap().doc.position(text).and_then(|(p, ..)| p), Some(repeat), "still the repeat's source");
    assert_eq!(spec(&s)["rows"], before["rows"], "the repeat keeps its settings");
}
