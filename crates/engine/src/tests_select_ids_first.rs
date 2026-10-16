//! #2130: selection-only commands take `ids`: the objects given are selected first and the
//! command runs on them, as `select.set` followed by the command does, in one undo step.

use serde_json::{Value, json};

use super::*;

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
    let one = |_: &mut Session, ids: &[u64]| vec![ids[1]];
    let cases: Vec<(&str, Value, Setup<'_>)> = vec![
        ("object.group", json!({}), &none),
        ("object.ungroup", json!({}), &grouped),
        ("object.clippingMask.make", json!({}), &none),
        ("object.clippingMask.release", json!({}), &clipped),
        ("object.compoundPath.make", json!({}), &none),
        ("object.compoundPath.release", json!({}), &compound),
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
