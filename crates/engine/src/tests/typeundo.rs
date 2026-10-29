//! Edit › Undo and Redo while the Type tool edits: the caret goes to where the step changed the text.

use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.prefs.placeholder_text = false;
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn plain(s: &Session, id: NodeId) -> String {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("not text"),
    }
}

fn caret(s: &Session) -> (u64, u64) {
    let o = s.tool_options();
    (o["caret"].as_u64().unwrap(), o["anchor"].as_u64().unwrap())
}

/// "abc" typed before "XYZ": Undo puts the caret before "XYZ" and Redo after "abc", from the
/// menu command and from the History panel's steps alike (both run `edit.undo` and `edit.redo`).
#[test]
fn undo_and_redo_put_the_caret_where_the_step_changed_the_text() {
    let v = ViewInfo::default();
    let mut s = session();
    let r = s.execute("text.create", &json!({"x": 100, "y": 100, "text": "XYZ", "size": 20})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.select_tool("type", v).unwrap();
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, 101.0, 95.0), v).unwrap();
    }
    s.set_tool_option("select", &json!({"start": 0, "end": 0}));
    s.tool_text("abc", v).unwrap();
    assert_eq!((plain(&s, id).as_str(), caret(&s)), ("abcXYZ", (3, 3)));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((plain(&s, id).as_str(), caret(&s)), ("XYZ", (0, 0)));
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!((plain(&s, id).as_str(), caret(&s)), ("abcXYZ", (3, 3)));
    // A deleted character comes back with the caret after it, wherever the caret went since.
    s.tool_key(ToolKey::End, Mods::default(), v).unwrap();
    s.tool_key(ToolKey::Backspace, Mods::default(), v).unwrap();
    assert_eq!(plain(&s, id), "abcXY");
    s.tool_key(ToolKey::Home, Mods::default(), v).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((plain(&s, id).as_str(), caret(&s)), ("abcXYZ", (6, 6)));
}

/// Undo and Redo run as steps of `command.batch` put the caret where they do run alone.
#[test]
fn undo_and_redo_in_a_batch_put_the_caret_where_the_step_changed_the_text() {
    let v = ViewInfo::default();
    let mut s = session();
    let r = s.execute("text.create", &json!({"x": 100, "y": 100, "text": "XYZ", "size": 20})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.select_tool("type", v).unwrap();
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, 101.0, 95.0), v).unwrap();
    }
    s.set_tool_option("select", &json!({"start": 0, "end": 0}));
    s.tool_text("abc", v).unwrap();
    s.execute("command.batch", &json!({"commands": [{"command": "edit.undo"}]})).unwrap();
    assert_eq!((plain(&s, id).as_str(), caret(&s)), ("XYZ", (0, 0)));
    s.execute("command.batch", &json!({"commands": [{"command": "edit.redo"}]})).unwrap();
    assert_eq!((plain(&s, id).as_str(), caret(&s)), ("abcXYZ", (3, 3)));
    // A batch whose later step fails is rolled back: the text and the caret are as they were, and
    // Undo still takes back the typing.
    let failed = json!({"commands": [{"command": "edit.undo"}, {"command": "no.suchCommand"}]});
    assert!(s.execute("command.batch", &failed).is_err());
    assert_eq!((plain(&s, id).as_str(), caret(&s)), ("abcXYZ", (3, 3)));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((plain(&s, id).as_str(), caret(&s)), ("XYZ", (0, 0)));
}
