//! Isolation mode ends when an edit removes the isolated object, and comes back when that edit is
//! taken back by a canceled undo group or scope or by an interaction's next preview.

use serde_json::{Value, json};

use super::{rect, session};
use crate::*;

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn isolation(s: &Session) -> Option<NodeId> {
    s.doc().unwrap().isolation
}

fn exists(s: &Session, id: NodeId) -> bool {
    s.doc().unwrap().doc.node(id).is_some()
}

/// A group of two rectangles, isolated.
fn isolated_group(s: &mut Session) -> NodeId {
    let a = rect(s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(s, 20.0, 0.0, 10.0, 10.0);
    run(s, "select.set", json!({"ids": [a.0, b.0]}));
    let g = NodeId(run(s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(s, "object.isolate", json!({"id": g.0}));
    g
}

/// Clear and Ungroup on the isolated group end isolation mode; Undo brings the group back without
/// isolating it, and Redo removes it again with isolation mode still off.
#[test]
fn an_edit_that_removes_the_isolated_object_ends_isolation_mode() {
    for cmd in ["edit.clear", "object.ungroup"] {
        let mut s = session();
        let g = isolated_group(&mut s);
        run(&mut s, cmd, json!({"ids": [g.0]}));
        assert!(!exists(&s, g), "{cmd}: the group is gone");
        assert_eq!(isolation(&s), None, "{cmd}");
        run(&mut s, "edit.undo", json!({}));
        assert!(exists(&s, g), "{cmd}: undone");
        assert_eq!(isolation(&s), None, "{cmd}: undone");
        run(&mut s, "edit.redo", json!({}));
        assert_eq!(isolation(&s), None, "{cmd}: redone");
    }
    // So does a script that removes it.
    let mut s = session();
    let g = isolated_group(&mut s);
    run(&mut s, "script.run", json!({"source": "app.activeDocument.pageItems[0].remove();", "name": "t.js"}));
    assert!(!exists(&s, g), "script: the group is gone");
    assert_eq!(isolation(&s), None, "script");
    // Clearing the art inside the isolated group keeps the group and isolation mode.
    let mut s = session();
    let g = isolated_group(&mut s);
    let inside = s.doc().unwrap().doc.node(g).unwrap().children().unwrap().iter().map(|n| n.id.0).collect::<Vec<_>>();
    run(&mut s, "edit.clear", json!({"ids": inside}));
    assert!(exists(&s, g));
    assert_eq!(isolation(&s), Some(g));
}

/// While a symbol is edited, removing a group isolated inside the symbol's editing layer isolates
/// that layer again, as Exit Isolation Mode from the group does, and symbol editing goes on.
#[test]
fn removing_a_group_isolated_in_symbol_editing_isolates_the_editing_layer() {
    for cmd in ["edit.clear", "object.ungroup"] {
        let mut s = session();
        rect(&mut s, 0.0, 0.0, 80.0, 40.0);
        let instance = NodeId(run(&mut s, "symbol.new", json!({"name": "Box"}))["id"].as_u64().unwrap());
        let layer = NodeId(run(&mut s, "symbol.edit", json!({"id": instance.0}))["layer"].as_u64().unwrap());
        rect(&mut s, 100.0, 100.0, 10.0, 10.0);
        let art = s.doc().unwrap().doc.node(layer).unwrap().children().unwrap().iter().map(|n| n.id.0).collect::<Vec<_>>();
        run(&mut s, "select.set", json!({ "ids": art }));
        let inner = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
        run(&mut s, "object.isolate", json!({"id": inner.0}));
        assert_eq!(isolation(&s), Some(inner), "{cmd}");
        run(&mut s, cmd, json!({"ids": [inner.0]}));
        assert!(!exists(&s, inner), "{cmd}: the group is gone");
        assert!(s.doc().unwrap().doc.symbol_edit.is_some(), "{cmd}: still editing the symbol");
        assert_eq!(isolation(&s), Some(layer), "{cmd}");
    }
}

/// A canceled undo group, and a canceled undo scope that joined an open group, put back the
/// isolated group and isolation mode with it; a kept group leaves isolation mode off.
#[test]
fn canceled_undo_groups_and_scopes_isolate_the_object_again() {
    for cancel in [true, false] {
        let mut s = session();
        let g = isolated_group(&mut s);
        s.begin_undo_group();
        run(&mut s, "edit.clear", json!({"ids": [g.0]}));
        assert_eq!(isolation(&s), None);
        s.end_undo_group(cancel);
        assert_eq!(exists(&s, g), cancel, "cancel: {cancel}");
        assert_eq!(isolation(&s), cancel.then_some(g), "cancel: {cancel}");
    }

    let mut s = session();
    let g = isolated_group(&mut s);
    s.begin_undo_group();
    run(&mut s, "object.move", json!({"ids": [g.0], "dx": 5, "dy": 0}));
    let scope = s.begin_undo_scope().unwrap();
    run(&mut s, "edit.clear", json!({"ids": [g.0]}));
    assert_eq!(isolation(&s), None);
    s.end_undo_scope(scope, true);
    assert!(exists(&s, g));
    assert_eq!(isolation(&s), Some(g), "joined scope canceled");
    s.end_undo_group(false);
    assert_eq!(isolation(&s), Some(g));
}

/// An interaction's preview that removes the isolated object ends isolation mode; the next preview,
/// which keeps it, starts again from isolation mode, and so does a cancel. A commit of the removing
/// preview leaves isolation mode off.
#[test]
fn previews_put_back_isolation_mode_with_the_document() {
    let mut s = session();
    let g = isolated_group(&mut s);
    s.begin_interaction("Test").unwrap();
    s.preview("edit.clear", &json!({"ids": [g.0]})).unwrap();
    assert_eq!(isolation(&s), None, "the preview removed the group");
    s.preview("object.move", &json!({"ids": [g.0], "dx": 5, "dy": 0})).unwrap();
    assert_eq!(isolation(&s), Some(g), "the next preview keeps it");
    s.preview("edit.clear", &json!({"ids": [g.0]})).unwrap();
    s.cancel_interaction().unwrap();
    assert_eq!(isolation(&s), Some(g), "canceled");

    s.begin_interaction("Test").unwrap();
    s.preview("edit.clear", &json!({"ids": [g.0]})).unwrap();
    s.commit_interaction().unwrap();
    assert!(!exists(&s, g));
    assert_eq!(isolation(&s), None, "committed");
}
