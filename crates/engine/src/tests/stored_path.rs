//! `select.storedPath`, the Actions panel's Select Path step: the step stores a path's id and a
//! fingerprint of it, and playback selects that path only while it is still there unchanged and
//! can be selected.

use std::io::Write;

use serde_json::{Value, json};

use crate::*;

/// A document with two rectangles and an ellipse, nothing selected → the session and their ids.
fn doc() -> (Session, [u64; 3]) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let mut add = |shape: &str, x: u32| s.execute(shape, &json!({"x": x, "y": 10, "width": 40, "height": 30})).unwrap()["id"].as_u64().unwrap();
    let ids = [add("shape.rectangle", 10), add("shape.rectangle", 80), add("shape.ellipse", 150)];
    s.execute("select.none", &json!({})).unwrap();
    (s, ids)
}

fn select(s: &mut Session, ids: &[u64]) {
    s.execute("select.set", &json!({"ids": ids})).unwrap();
}

fn selected(s: &Session) -> Vec<u64> {
    s.doc().unwrap().selection.objects.iter().map(|id| id.0).collect()
}

/// The Select Path step's params for `id`, selected alone.
fn step_for(s: &mut Session, id: u64) -> Value {
    select(s, &[id]);
    stored_path_step(s.doc().unwrap()).unwrap()
}

#[test]
fn the_hash_is_fnv_1a_64() {
    // The published FNV-1a 64-bit test values for "", "a" and "foobar".
    for (bytes, want) in [(&b""[..], 0xcbf2_9ce4_8422_2325_u64), (b"a", 0xaf63_dc4c_8601_ec8c), (b"foobar", 0x8594_4171_f739_67e8)] {
        let mut h = crate::cmd::select::Fnv::default();
        h.write_all(bytes).unwrap();
        assert_eq!(h.0, want, "{bytes:?}");
    }
}

#[test]
fn the_step_stores_the_id_and_a_fingerprint_and_plays_by_selecting_the_path() {
    let (mut s, [a, b, _]) = doc();
    let p = step_for(&mut s, a);
    assert_eq!(p["id"], json!(a));
    let fingerprint = p["fingerprint"].as_str().unwrap();
    assert!(fingerprint.starts_with("v1:") && fingerprint.len() == 19, "{fingerprint}");
    assert_eq!(step_for(&mut s, a), p, "the same path gives the same step");
    assert_ne!(step_for(&mut s, b)["fingerprint"], p["fingerprint"], "another path gives another fingerprint");
    select(&mut s, &[b]);
    let r = s.execute("select.storedPath", &p).unwrap();
    assert_eq!(r, json!({"count": 1, "ids": [a]}));
    assert_eq!(selected(&s), [a], "only the stored path is selected");
}

#[test]
fn a_compound_path_is_one_step_and_a_path_in_a_group_is_one_too() {
    let (mut s, [a, b, c]) = doc();
    select(&mut s, &[a, b]);
    let compound = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    let p = step_for(&mut s, compound);
    let member = s.doc().unwrap().doc.children(Some(vectorcraft_doc::NodeId(compound))).unwrap()[0].id.0;
    select(&mut s, &[member]);
    assert_eq!(stored_path_step(s.doc().unwrap()).unwrap_err(), "select one whole path or compound path", "a member alone is not stored");
    s.execute("select.none", &json!({})).unwrap();
    s.execute("select.storedPath", &p).unwrap();
    assert_eq!(selected(&s), [compound]);

    select(&mut s, &[c, compound]);
    let group = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(stored_path_step(s.doc().unwrap()).unwrap_err(), "select one whole path or compound path", "a group is not stored");
    let inner = step_for(&mut s, c);
    select(&mut s, &[group]);
    s.execute("select.storedPath", &inner).unwrap();
    assert_eq!(selected(&s), [c], "the path inside the group is selected alone");
}

#[test]
fn only_one_whole_path_that_can_be_selected_is_stored() {
    let (mut s, [a, b, _]) = doc();
    let whole = "select one whole path or compound path";
    assert_eq!(stored_path_step(s.doc().unwrap()).unwrap_err(), "select a path");
    select(&mut s, &[a, b]);
    assert_eq!(stored_path_step(s.doc().unwrap()).unwrap_err(), "select only one path");
    s.execute("guide.add", &json!({"vertical": true, "pos": 200})).unwrap();
    select(&mut s, &[a]);
    s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
    assert_eq!(s.doc().unwrap().selection.guides.len(), 1, "the guide is selected with the path");
    assert_eq!(stored_path_step(s.doc().unwrap()).unwrap_err(), "select only one path", "a path with a guide");
    s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
    assert_eq!(stored_path_step(s.doc().unwrap()).unwrap(), step_for(&mut s, a), "the path alone again");
    s.execute("select.anchors", &json!({"id": a, "anchors": [[0, 0]], "mode": "set"})).unwrap();
    assert_eq!(stored_path_step(s.doc().unwrap()).unwrap_err(), whole, "an anchor alone");
    let t = s.execute("text.create", &json!({"x": 10, "y": 200, "text": "Type"})).unwrap()["id"].as_u64().unwrap();
    select(&mut s, &[t]);
    assert_eq!(stored_path_step(s.doc().unwrap()).unwrap_err(), whole, "type");
    select(&mut s, &[a]);
    s.execute("object.lock", &json!({})).unwrap();
    let mut st = s.doc().unwrap().clone();
    st.selection.objects = vec![vectorcraft_doc::NodeId(a)];
    assert_eq!(stored_path_step(&st).unwrap_err(), "the path is hidden or locked, or cannot be selected here");
    let guide = {
        let mut d = (*st.doc).clone();
        if let Some(vectorcraft_doc::NodeKind::Path { guide, .. }) = d.node_mut(vectorcraft_doc::NodeId(a)).map(|n| &mut n.kind) {
            *guide = true;
        }
        d
    };
    assert!(path_fingerprint(guide.node(vectorcraft_doc::NodeId(a)).unwrap()).is_none(), "a guide is not stored");
}

#[test]
fn a_changed_missing_hidden_or_other_path_is_an_error_and_keeps_the_selection() {
    let (mut s, [a, b, c]) = doc();
    let p = step_for(&mut s, a);
    let fails = |s: &mut Session, p: &Value, want: &str| {
        select(s, &[c]);
        let e = s.execute("select.storedPath", p).unwrap_err().to_string();
        assert!(e.contains(want), "{e}");
        assert_eq!(selected(s), [c], "the selection stays as it was");
    };
    select(&mut s, &[a]);
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    fails(&mut s, &p, "is not the stored path");
    let moved = step_for(&mut s, a);
    s.execute("object.hide", &json!({})).unwrap();
    fails(&mut s, &moved, "hidden or locked");
    s.execute("object.showAll", &json!({})).unwrap();
    let shown = step_for(&mut s, b);
    s.execute("edit.clear", &json!({})).unwrap();
    fails(&mut s, &shown, "no such object id");
    // Another document: the id names another path there, or nothing.
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 1, "y": 1, "width": 7, "height": 9})).unwrap();
    let c2 = s.execute("shape.ellipse", &json!({"x": 20, "y": 20, "width": 9, "height": 7})).unwrap()["id"].as_u64().unwrap();
    select(&mut s, &[c2]);
    assert!(s.execute("select.storedPath", &moved).is_err());
    assert_eq!(selected(&s), [c2]);
}

#[test]
fn a_path_on_a_locked_artboard_is_refused_in_isolation_mode_too() {
    let (mut s, [a, b, _]) = doc();
    select(&mut s, &[a, b]);
    let group = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    let p = step_for(&mut s, a);
    s.execute("select.none", &json!({})).unwrap();
    s.execute("object.isolate", &json!({"id": group})).unwrap();
    s.execute("artboard.new", &json!({"x": 0, "y": 0, "width": 300, "height": 200})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "locked": true})).unwrap();
    assert!(s.doc().unwrap().isolation.is_some(), "the group stays isolated");
    assert!(s.doc().unwrap().doc.on_locked_artboard(vectorcraft_doc::NodeId(a)), "the path is on the locked artboard");
    let mut st = s.doc().unwrap().clone();
    st.selection.objects = vec![vectorcraft_doc::NodeId(a)];
    assert_eq!(stored_path_step(&st).unwrap_err(), "the path is hidden or locked, or cannot be selected here", "capture");
    let before = selected(&s);
    let e = s.execute("select.storedPath", &p).unwrap_err().to_string();
    assert!(e.contains("hidden or locked"), "{e}");
    assert_eq!(selected(&s), before, "the selection stays as it was");
}

#[test]
fn a_failing_step_in_a_batch_undoes_the_steps_before_it() {
    let (mut s, [a, _, c]) = doc();
    let p = step_for(&mut s, a);
    select(&mut s, &[a]);
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    select(&mut s, &[c]);
    let before = s.doc().unwrap().doc.clone();
    let steps = [json!({"command": "object.move", "params": {"dx": 1, "dy": 0}}), json!({"command": "select.storedPath", "params": p})];
    assert!(s.execute("command.batch", &json!({"commands": steps})).is_err());
    assert!(s.doc().unwrap().doc == before, "the move before the step is undone");
}

#[test]
fn params_are_an_id_and_a_v1_fingerprint() {
    let (mut s, [a, ..]) = doc();
    let p = step_for(&mut s, a);
    let f = p["fingerprint"].clone();
    for bad in [
        json!({"fingerprint": f}),
        json!({"id": -1, "fingerprint": f}),
        json!({"id": "1", "fingerprint": f}),
        json!({"id": a}),
        json!({"id": a, "fingerprint": 5}),
        json!({"id": a, "fingerprint": "v2:0123456789abcdef"}),
        json!({"id": a, "fingerprint": "v1:0123456789ABCDEF"}),
        json!({"id": a, "fingerprint": "v1:0123456789abcde"}),
        json!({"id": a, "fingerprint": f, "extra": 1}),
        json!([a]),
    ] {
        assert!(stored_path_params(&bad).is_err(), "{bad}");
        s.execute("select.none", &json!({})).unwrap();
        assert!(s.execute("select.storedPath", &bad).is_err(), "{bad}");
        assert!(selected(&s).is_empty(), "{bad}");
    }
    assert!(stored_path_params(&p).is_ok());
}
