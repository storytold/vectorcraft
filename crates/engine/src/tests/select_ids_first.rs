//! #2130: selection-only commands take `ids`: the objects given are selected first and the
//! command runs on them, as `select.set` followed by the command does, in one undo step.

use serde_json::{Value, json};

use crate::*;

/// Three rectangles and an ellipse, nothing selected → the session and their ids.
fn doc() -> (Session, Vec<u64>) {
    let mut s = Session::new();
    // Separately created fixtures must compare equal across clock ticks.
    s.execute("file.new", &json!({"width": 400, "height": 300, "created": null})).unwrap();
    let mut ids = vec![];
    for (x, y) in [(10, 10), (50, 30), (80, 50)] {
        ids.push(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 80, "height": 60})).unwrap()["id"].as_u64().unwrap());
    }
    ids.push(s.execute("shape.ellipse", &json!({"x": 40, "y": 20, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.none", &json!({})).unwrap();
    (s, ids)
}

/// Runs `setup` (commands on the selection, given the fixture's ids), then `cmd` two ways: after
/// `select.set {ids}`, and with `ids` in its params and something else selected. Both must leave
/// the same document, selection and result.
/// Makes what a case runs on (given the fixture's ids) → the ids to give.
type Setup<'a> = &'a dyn Fn(&mut Session, &[u64]) -> Vec<u64>;

fn same_as_selecting_first(cmd: &str, params: Value, setup: Setup<'_>) {
    let run = |given: bool| {
        let (mut s, ids) = doc();
        let targets = setup(&mut s, &ids);
        let r = if given {
            // Another object selected: the given ones take its place.
            s.execute("select.set", &json!({"ids": [ids[3]]})).unwrap();
            let mut p = params.clone();
            p["ids"] = json!(targets);
            s.execute(cmd, &p)
        } else {
            s.execute("select.set", &json!({"ids": targets})).unwrap();
            s.execute(cmd, &params)
        };
        let st = s.doc().unwrap();
        (r.map_err(|e| e.to_string()), st.doc.clone(), st.selection.objects.clone())
    };
    let (by_selection, by_ids) = (run(false), run(true));
    assert!(by_selection.0.is_ok(), "{cmd}: {:?}", by_selection.0);
    assert_eq!(by_ids.0, by_selection.0, "{cmd}: result");
    assert!(by_ids.1 == by_selection.1, "{cmd}: the document differs");
    assert_eq!(by_ids.2, by_selection.2, "{cmd}: selection");
}

fn none(_: &mut Session, ids: &[u64]) -> Vec<u64> {
    ids[..3].to_vec()
}

#[test]
fn ids_are_selected_first() {
    let grouped = |s: &mut Session, ids: &[u64]| {
        s.execute("select.set", &json!({"ids": &ids[..2]})).unwrap();
        let g = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.none", &json!({})).unwrap();
        vec![g]
    };
    let clipped = |s: &mut Session, ids: &[u64]| {
        s.execute("select.set", &json!({"ids": &ids[..3]})).unwrap();
        let g = s.execute("object.clippingMask.make", &json!({})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.none", &json!({})).unwrap();
        vec![g]
    };
    let compound = |s: &mut Session, ids: &[u64]| {
        s.execute("select.set", &json!({"ids": &ids[..2]})).unwrap();
        let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.none", &json!({})).unwrap();
        vec![c]
    };
    let shape = |s: &mut Session, ids: &[u64]| {
        s.execute("select.set", &json!({"ids": &ids[..2]})).unwrap();
        let c = s.execute("object.compoundShape.make", &json!({})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.none", &json!({})).unwrap();
        c
    };
    let shaped = |s: &mut Session, ids: &[u64]| vec![shape(s, ids)];
    let member = |s: &mut Session, ids: &[u64]| {
        shape(s, ids);
        vec![ids[1]]
    };
    let one = |_: &mut Session, ids: &[u64]| vec![ids[1]];
    let cases: Vec<(&str, Value, Setup<'_>)> = vec![
        ("object.group", json!({}), &none),
        ("object.ungroup", json!({}), &grouped),
        ("object.clippingMask.make", json!({}), &none),
        ("object.clippingMask.release", json!({}), &clipped),
        ("object.compoundPath.make", json!({}), &none),
        ("object.compoundPath.release", json!({}), &compound),
        ("object.compoundShape.make", json!({"mode": "intersect"}), &none),
        ("object.compoundShape.setMode", json!({"mode": "subtract"}), &member),
        ("object.compoundShape.release", json!({}), &shaped),
        ("object.compoundShape.expand", json!({}), &shaped),
        ("object.arrange.bringToFront", json!({}), &one),
        ("object.arrange.bringForward", json!({}), &one),
        ("object.arrange.sendBackward", json!({}), &one),
        ("object.arrange.sendToBack", json!({}), &one),
        ("object.arrange.sendToCurrentLayer", json!({}), &one),
        ("object.lock", json!({}), &one),
        ("object.hide", json!({}), &one),
        ("edit.duplicate", json!({"dx": 5, "dy": 5}), &none),
        ("object.move", json!({"dx": 12, "dy": -4}), &none),
        ("object.rotate", json!({"angle": 30}), &none),
        ("object.scale", json!({"sx": 150}), &none),
        ("object.reflect", json!({"axis": "vertical"}), &none),
        ("object.shear", json!({"angle": 20}), &none),
        ("object.pathfinder.unite", json!({}), &none),
        ("object.pathfinder.minusFront", json!({}), &none),
        ("object.pathfinder.intersect", json!({}), &none),
        ("object.pathfinder.exclude", json!({}), &none),
        ("object.pathfinder.divide", json!({}), &none),
        ("object.pathfinder.trim", json!({}), &none),
        ("object.pathfinder.merge", json!({}), &none),
        ("object.pathfinder.crop", json!({}), &none),
        ("object.pathfinder.outline", json!({}), &none),
        ("object.pathfinder.minusBack", json!({}), &none),
    ];
    for (cmd, params, setup) in cases {
        same_as_selecting_first(cmd, params, setup);
    }
}

/// Every command that selects its `ids` first says so in its params description, so `command_list`
/// and the params audit (#1442) know the key.
#[test]
fn the_descriptions_name_ids() {
    for id in SELECTS_GIVEN_IDS {
        let spec = cmd::find_command(id).unwrap_or_else(|| panic!("{id} is not a command"));
        assert!(cmd::params::takes_key(spec.params, "ids"), "{id}: {}", spec.params);
    }
}

#[test]
fn one_undo_step_takes_it_back() {
    let (mut s, ids) = doc();
    let before = s.doc().unwrap().doc.clone();
    let g = s.execute("object.group", &json!({"ids": &ids[..3]})).unwrap()["id"].as_u64().unwrap();
    assert!(matches!(s.doc().unwrap().doc.node(NodeId(g)).unwrap().kind, vectorcraft_doc::NodeKind::Group { .. }));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc == before);
}

#[test]
fn bad_ids_are_refused_and_change_nothing() {
    let (mut s, ids) = doc();
    s.execute("select.set", &json!({"ids": [ids[3]]})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    for bad in [json!([999]), json!([ids[0], -1]), json!(["a"])] {
        let r = s.execute("object.group", &json!({"ids": bad}));
        assert!(r.is_err(), "{bad}: {r:?}");
        assert!(s.doc().unwrap().doc == before, "{bad}");
        assert_eq!(s.doc().unwrap().selection.objects, vec![NodeId(ids[3])], "{bad}: the selection stays");
    }
}

/// A layer or sublayer is not an object these commands act on: its id is refused, alone or with an
/// object's, and the document, the selection and the undo history stay as they were.
#[test]
fn layer_ids_are_refused_and_change_nothing() {
    let (mut s, ids) = doc();
    let layer = s.execute("layer.new", &json!({"name": "Top"})).unwrap()["id"].as_u64().unwrap();
    let sub = s.execute("layer.newSublayer", &json!({"parent": layer})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [ids[3]]})).unwrap();
    let st = s.doc().unwrap();
    let (before, undo) = (st.doc.clone(), st.history.undo.len());
    let params = |cmd: &str| match cmd {
        "edit.duplicate" => json!({"dx": 5, "dy": 5}),
        "object.move" => json!({"dx": 12, "dy": -4}),
        "object.rotate" => json!({"angle": 30}),
        "object.scale" => json!({"sx": 150}),
        "object.reflect" => json!({"axis": "vertical"}),
        "object.shear" => json!({"angle": 20}),
        _ => json!({}),
    };
    for cmd in SELECTS_GIVEN_IDS {
        for given in [json!([layer]), json!([sub]), json!([ids[0], layer])] {
            let mut p = params(cmd);
            p["ids"] = given.clone();
            let r = s.execute(cmd, &p);
            let Err(EngineError::BadParams { msg, .. }) = &r else { panic!("{cmd} {given}: {r:?}") };
            assert!(msg.contains("is a layer, not an object"), "{cmd} {given}: {msg}");
            assert_eq!(msg.contains("layer.move"), cmd.starts_with("object.arrange."), "{cmd} {given}: {msg}");
            let st = s.doc().unwrap();
            assert!(st.doc == before, "{cmd} {given}: the document changed");
            assert_eq!(st.selection.objects, vec![NodeId(ids[3])], "{cmd} {given}: the selection stays");
            assert_eq!(st.history.undo.len(), undo, "{cmd} {given}: an undo step was recorded");
        }
    }
    // An unknown id before the layer's is still reported as unknown.
    let r = s.execute("object.arrange.sendToBack", &json!({"ids": [999, layer]}));
    assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg == "no such object id 999"), "{r:?}");
}

/// An id that names no object is reported by the command that was run, not by the `select.set`
/// that checks the ids.
#[test]
fn id_errors_name_the_command_run() {
    let (mut s, ids) = doc();
    let layer = s.execute("layer.new", &json!({"name": "Top"})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [ids[3]]})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    for cmd in SELECTS_GIVEN_IDS {
        for (given, want) in [
            (json!([999]), "no such object id 999".to_string()),
            (json!([999, layer]), "no such object id 999".to_string()),
            (json!([ids[0], ids[1], -1]), "invalid object id -1: expected a non-negative integer".to_string()),
            (json!(ids[0]), "ids must be an array of non-negative integer object ids".to_string()),
        ] {
            let mut p = match *cmd {
                "edit.duplicate" => json!({"dx": 5, "dy": 5}),
                "object.move" => json!({"dx": 12, "dy": -4}),
                "object.rotate" => json!({"angle": 30}),
                "object.scale" => json!({"sx": 150}),
                "object.reflect" => json!({"axis": "vertical"}),
                "object.shear" => json!({"angle": 20}),
                _ => json!({}),
            };
            p["ids"] = given.clone();
            let r = s.execute(cmd, &p);
            let Err(EngineError::BadParams { cmd: named, msg }) = &r else { panic!("{cmd} {given}: {r:?}") };
            assert_eq!((named.as_str(), msg), (*cmd, &want), "{cmd} {given}");
            assert!(s.doc().unwrap().doc == before, "{cmd} {given}: the document changed");
            assert_eq!(s.doc().unwrap().selection.objects, vec![NodeId(ids[3])], "{cmd} {given}: the selection stays");
        }
    }
}

#[test]
fn a_command_that_fails_after_selecting_its_ids_puts_the_selection_back() {
    let (mut s, ids) = doc();
    let far = s.execute("shape.rectangle", &json!({"x": 300, "y": 200, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [ids[3]]})).unwrap();
    let (before, layer) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().active_layer);
    // The ids are fine, so they are selected; the shapes don't overlap, so Intersect then fails.
    let e = s.execute("object.pathfinder.intersect", &json!({"ids": [ids[0], far]})).unwrap_err().to_string();
    assert!(e.contains("empty result"), "{e}");
    assert!(s.doc().unwrap().doc == before);
    assert_eq!(s.doc().unwrap().selection.objects, vec![NodeId(ids[3])], "the selection stays");
    assert_eq!(s.doc().unwrap().active_layer, layer);
    // Without ids the failing command leaves the selection it ran on, as before.
    s.execute("select.set", &json!({"ids": [ids[0], far]})).unwrap();
    assert!(s.execute("object.pathfinder.intersect", &json!({})).is_err());
    assert_eq!(s.doc().unwrap().selection.objects, vec![NodeId(ids[0]), NodeId(far)]);
}

#[test]
fn without_ids_the_selection_is_used_as_before() {
    let (mut s, ids) = doc();
    s.execute("select.set", &json!({"ids": &ids[..2]})).unwrap();
    let g = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(s.doc().unwrap().doc.node(NodeId(g)).unwrap().children().unwrap().len(), 2);
    // Nothing selected and no ids: still disabled.
    s.execute("select.none", &json!({})).unwrap();
    assert!(matches!(s.execute("object.group", &json!({})), Err(EngineError::Disabled(..))));
}
