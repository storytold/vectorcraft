//! Minimal reproductions of engine bugs found by the property tests. Each is `#[ignore]`d with a
//! `BUG:` reason until fixed; remove the attribute when the fix lands.

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_testkit::fixtures::{exec, rect, select, session};
use vectorcraft_testkit::invariants::{check_native_roundtrip_exact, check_session, doc_json, first_diff};

#[test]
fn bug_group_inside_compound() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    select(&mut s, &[a, b]);
    exec(&mut s, "object.compoundPath.make", json!({}));
    select(&mut s, &[a]);
    let _ = s.execute("object.group", &json!({}));
    check_session(&s).unwrap();
}

#[test]
fn bug_nested_commands_double_journal() {
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    exec(&mut s, "transparency.set", json!({"opacity": 0}));
    exec(&mut s, "edit.undo", json!({}));
    let ids: Vec<&str> = s.journal.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["file.new", "shape.rectangle", "transparency.set", "edit.undo"], "journal: {ids:?}");
    let mut r = Session::new();
    for (id, p) in &s.journal.clone() {
        let _ = r.execute(id, p);
    }
    let (want, got) = (doc_json(&s.doc().unwrap().doc), doc_json(&r.doc().unwrap().doc));
    assert_eq!(want, got, "{}", first_diff(&want, &got, "$"));
}

#[test]
fn bug_native_roundtrip_not_bit_exact() {
    let mut s = session();
    exec(&mut s, "shape.ellipse", json!({"x": 100.1, "y": 60.3, "width": 120.7, "height": 120.9}));
    exec(&mut s, "object.rotate", json!({"angle": 33.3}));
    check_native_roundtrip_exact(&s.doc().unwrap().doc).unwrap();
}

#[test]
fn bug_bring_to_front_across_layers_underflows() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    exec(&mut s, "layer.new", json!({}));
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    for cmd in ["object.arrange.bringToFront", "object.arrange.bringForward"] {
        select(&mut s, &[a, b]);
        let r = vectorcraft_testkit::catch_quiet(|| s.execute(cmd, &json!({})));
        assert!(r.is_ok(), "{cmd} panicked: {:?}", r.err());
        check_session(&s).unwrap();
    }
}
