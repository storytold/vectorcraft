//! Selection command results and validation for agent callers (#784).

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_engine::{EngineError, Session};

fn session() -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let mut rectangle =
        |x| NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
    let a = rectangle(0);
    let b = rectangle(20);
    (s, a, b)
}

#[test]
fn selection_commands_reject_invalid_ids_without_changing_state() {
    for command in ["select.set", "select.add", "select.toggle"] {
        let (mut s, a, b) = session();
        s.execute("select.set", &json!({"ids": [b.0, a.0]})).unwrap();
        s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 0]], "mode": "add"})).unwrap();
        s.execute("select.key", &json!({"id": b.0})).unwrap();
        for invalid in [json!(999), json!(-1), json!(2.5), json!("2"), Value::Null, json!(true), json!(u64::MAX)] {
            let selection = s.active().unwrap().selection.clone();
            let revision = s.active().unwrap().revision;
            let journal = s.journal.clone();
            let error = s.execute(command, &json!({"ids": [b.0, invalid]})).unwrap_err();
            assert!(matches!(error, EngineError::BadParams { .. }), "{command}: {error}");
            assert!(error.to_string().contains(&invalid.to_string()), "the error must identify {invalid}: {error}");
            assert_eq!(s.active().unwrap().selection, selection, "{command}: {invalid}");
            assert_eq!(s.active().unwrap().revision, revision);
            assert_eq!(s.journal, journal);
        }
    }
}

#[test]
fn selection_commands_require_an_id_array() {
    for command in ["select.set", "select.add", "select.toggle"] {
        let (mut s, _, _) = session();
        for params in [json!({}), json!({"ids": null}), json!({"ids": 2}), json!({"ids": "2"}), json!({"ids": true})] {
            let selection = s.active().unwrap().selection.clone();
            assert!(matches!(s.execute(command, &params), Err(EngineError::BadParams { .. })), "{command}: {params}");
            assert_eq!(s.active().unwrap().selection, selection);
        }
    }
}

#[test]
fn selection_commands_report_the_resulting_selection() {
    let (mut s, a, b) = session();
    assert_eq!(s.execute("select.set", &json!({"ids": [b.0, a.0, b.0]})).unwrap(), json!({"count": 2, "ids": [b.0, a.0]}));
    assert_eq!(s.execute("select.set", &json!({"ids": []})).unwrap(), json!({"count": 0, "ids": []}));
    assert_eq!(s.execute("select.add", &json!({"ids": [a.0, a.0]})).unwrap(), json!({"count": 1, "ids": [a.0]}));
    assert_eq!(s.execute("select.add", &json!({"ids": [b.0]})).unwrap(), json!({"count": 2, "ids": [a.0, b.0]}));
    assert_eq!(s.execute("select.toggle", &json!({"id": a.0})).unwrap(), json!({"count": 1, "ids": [b.0]}));
    assert_eq!(s.execute("select.toggle", &json!({"ids": [b.0, a.0]})).unwrap(), json!({"count": 1, "ids": [a.0]}));
    for command in ["select.add", "select.toggle"] {
        assert_eq!(s.execute(command, &json!({"ids": []})).unwrap(), json!({"count": 1, "ids": [a.0]}));
    }
}

#[test]
fn single_id_toggle_validates_before_mutating() {
    let (mut s, a, _) = session();
    for invalid in [json!(999), json!(-1), json!(2.5), json!("2"), Value::Null, json!(true)] {
        let selection = s.active().unwrap().selection.clone();
        assert!(matches!(s.execute("select.toggle", &json!({"id": invalid})), Err(EngineError::BadParams { .. })));
        assert_eq!(s.active().unwrap().selection, selection);
    }
    // A malformed array must not silently fall back to a valid single id.
    assert!(s.execute("select.toggle", &json!({"ids": "bad", "id": a.0})).is_err());
}

#[test]
fn select_key_rejects_ids_it_cannot_use_and_keeps_the_key() {
    let (mut s, a, b) = session();
    let rectangle = |s: &mut Session, x: u64| {
        NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap())
    };
    let [c, d, e, f, unselected] = [40, 60, 80, 100, 120].map(|x| rectangle(&mut s, x));
    s.execute("select.set", &json!({"ids": [c.0, d.0]})).unwrap();
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [e.0]})).unwrap();
    let inner = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [inner.0, f.0]})).unwrap();
    let group = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    // c is a member of a compound path, and e is in a group inside the selected group.
    s.execute("select.set", &json!({"ids": [b.0, a.0, c.0, group.0, e.0]})).unwrap();
    s.execute("select.key", &json!({"id": b.0})).unwrap();
    let invalid_ids =
        [json!(999), json!("x"), json!(-1), json!(2.5), Value::Null, json!(true), json!(u64::MAX), json!(unselected.0), json!(c.0), json!(e.0)];
    for invalid in invalid_ids {
        let selection = s.active().unwrap().selection.clone();
        let revision = s.active().unwrap().revision;
        let journal = s.journal.clone();
        let error = s.execute("select.key", &json!({"id": invalid})).unwrap_err();
        assert!(matches!(error, EngineError::BadParams { .. }), "{invalid}: {error}");
        assert!(error.to_string().contains(&invalid.to_string()), "the error must identify {invalid}: {error}");
        assert_eq!(s.active().unwrap().selection, selection, "{invalid}");
        assert_eq!(s.active().unwrap().selection.key, Some(b), "{invalid}");
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(s.journal, journal);
    }
    assert_eq!(s.execute("select.key", &json!({"id": group.0})).unwrap(), json!({"key": group.0}), "the group is top-level");
    // A selected layer is not a top-level object.
    let layer = s.active().unwrap().doc.layers[0].id;
    s.execute("select.add", &json!({"ids": [layer.0]})).unwrap();
    assert!(s.active().unwrap().selection.contains(layer));
    let error = s.execute("select.key", &json!({"id": layer.0})).unwrap_err();
    assert!(matches!(error, EngineError::BadParams { .. }), "{error}");
    assert_eq!(s.active().unwrap().selection.key, Some(group));
    // Inside the isolated group, f is top-level: the Selection tool's clicks there send its id.
    s.execute("object.isolate", &json!({"id": group.0})).unwrap();
    s.execute("select.set", &json!({"ids": [inner.0, f.0]})).unwrap();
    assert_eq!(s.execute("select.key", &json!({"id": f.0})).unwrap(), json!({"key": f.0}), "an object in an unselected group");
}

#[test]
fn select_key_and_document_inspect_report_the_key_object() {
    let (mut s, a, b) = session();
    let inspected = |s: &mut Session| s.execute("document.inspect", &json!({})).unwrap().get("key").cloned();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    assert_eq!(inspected(&mut s), Some(Value::Null), "no key yet");
    assert_eq!(s.execute("select.key", &json!({"id": b.0})).unwrap(), json!({"key": b.0}));
    assert_eq!(inspected(&mut s), Some(json!(b.0)));
    assert_eq!(s.execute("select.key", &json!({})).unwrap(), json!({"key": null}));
    assert_eq!(inspected(&mut s), Some(Value::Null));
    // A single selected object is not a key object.
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    assert_eq!(s.execute("select.key", &json!({"id": b.0})).unwrap(), json!({"key": null}));
    assert_eq!(inspected(&mut s), Some(Value::Null));
}

#[test]
fn select_segments_mode_toggle_and_add() {
    let (mut s, a, _) = session();
    // Select segment 0 with mode set
    s.execute("select.segments", &json!({"id": a.0, "segments": [[0, 0]], "mode": "set"})).unwrap();
    let sel = &s.active().unwrap().selection;
    assert_eq!(sel.segments.get(&a), Some(&[(0, 0)].into()));
    assert_eq!(sel.anchors.get(&a), Some(&[(0, 0), (0, 1)].into()));

    // Toggle segment 1 on (add)
    s.execute("select.segments", &json!({"id": a.0, "segments": [[0, 1]], "mode": "toggle"})).unwrap();
    let sel = &s.active().unwrap().selection;
    assert_eq!(sel.segments.get(&a), Some(&[(0, 0), (0, 1)].into()));
    assert_eq!(sel.anchors.get(&a), Some(&[(0, 0), (0, 1), (0, 2)].into()));

    // Toggle segment 0 off
    s.execute("select.segments", &json!({"id": a.0, "segments": [[0, 0]], "mode": "toggle"})).unwrap();
    let sel = &s.active().unwrap().selection;
    assert_eq!(sel.segments.get(&a), Some(&[(0, 1)].into()));
    assert_eq!(sel.anchors.get(&a), Some(&[(0, 1), (0, 2)].into()));

    // Toggle segment 1 off -> clears selection for this path
    s.execute("select.segments", &json!({"id": a.0, "segments": [[0, 1]], "mode": "toggle"})).unwrap();
    let sel = &s.active().unwrap().selection;
    assert!(!sel.contains(a));
    assert!(sel.segments.is_empty());
    assert!(sel.anchors.is_empty());
}
