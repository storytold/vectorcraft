#![allow(clippy::unwrap_used, clippy::panic)]
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use crate::Session;

fn setup(effect: &str, params: Value) -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":400,"height":400})).unwrap();
    let id = NodeId(s.execute("path.create", &json!({"d":"M100 100L200 100L200 200L100 200Z"})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"color":"#338ad6"})).unwrap();
    s.execute("effect.apply", &json!({"effect":effect,"item":null,"params":params})).unwrap();
    (s, id)
}

fn effect(s: &Session, id: NodeId) -> (String, Value) {
    let e = &s.doc().unwrap().doc.node(id).unwrap().appearance.effects[0];
    (e.id.clone(), e.params.clone())
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn three_d_type_turns_the_effect_in_place_keeping_shared_settings_in_one_undo_step() {
    let types = [("plane", "threeD.rotate"), ("extrude", "threeD.extrude"), ("revolve", "threeD.revolve"), ("inflate", "threeD.inflate")];
    for (from_type, from) in types {
        for (to_type, to) in types {
            if from == to {
                continue;
            }
            let (mut s, id) = setup(from, json!({"rotationX": 12.0, "rotationY": -30.0, "rotationZ": 5.0, "ambient": 40.0, "lightIntensity": 55.0}));
            let before = s.doc().unwrap().doc.clone();
            let history = undo_len(&s);
            let p = json!({"index": 0, "item": null, "threeDType": to_type});
            s.check_params("effect.setParams", &p).unwrap();
            s.execute("effect.setParams", &p).unwrap();
            let (now, params) = effect(&s, id);
            assert_eq!(now, to, "{from_type} → {to_type}");
            assert_eq!(params["rotationX"], 12.0, "{from_type} → {to_type}");
            assert_eq!(params["rotationY"], -30.0);
            assert_eq!(params["ambient"], 40.0);
            assert_eq!(params["lightIntensity"], 55.0);
            // Settings only the new effect takes start at its defaults; others are dropped.
            let defaults = vectorcraft_render::effects::default_params(to).unwrap();
            for key in params.as_object().unwrap().keys() {
                assert!(defaults.get(key).is_some(), "{from_type} → {to_type} kept `{key}`");
            }
            if to == "threeD.extrude" {
                assert_eq!(params["bevel"], defaults["bevel"]);
            }
            assert_eq!(undo_len(&s), history + 1);
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(s.doc().unwrap().doc, before);
        }
    }
}

#[test]
fn three_d_type_carries_depth_between_extrude_and_inflate_and_merges_params_after() {
    let (mut s, id) = setup("threeD.extrude", json!({"depth": 80.0, "twist": 30.0, "bevel": "classic"}));
    s.execute("effect.setParams", &json!({"index": 0, "threeDType": "inflate", "params": {"volume": 70.0}})).unwrap();
    let (now, params) = effect(&s, id);
    assert_eq!(now, "threeD.inflate");
    assert_eq!(params["depth"], 80.0);
    assert_eq!(params["twist"], 30.0);
    assert_eq!(params["volume"], 70.0);
    assert!(params.get("bevel").is_none());
}

#[test]
fn revolve_angles_outside_half_a_turn_wrap_when_the_type_leaves_revolve() {
    let (mut s, id) = setup("threeD.revolve", json!({"rotationX": 270.0, "rotationY": -200.0}));
    s.execute("effect.setParams", &json!({"index": 0, "threeDType": "plane"})).unwrap();
    let (_, params) = effect(&s, id);
    assert_eq!(params["rotationX"], -90.0);
    assert_eq!(params["rotationY"], 160.0);
}

#[test]
fn the_current_type_alone_changes_nothing_and_records_no_undo_step() {
    let (mut s, id) = setup("threeD.extrude", json!({"depth": 80.0}));
    let before = s.doc().unwrap().doc.clone();
    let history = undo_len(&s);
    let r = s.execute("effect.setParams", &json!({"index": 0, "threeDType": "extrude"})).unwrap();
    assert_eq!(r["ids"], json!([]));
    assert_eq!(undo_len(&s), history);
    assert_eq!(s.doc().unwrap().doc, before);
    // With other changes the edit runs as usual.
    s.execute("effect.setParams", &json!({"index": 0, "threeDType": "extrude", "params": {"depth": 90.0}})).unwrap();
    assert_eq!(effect(&s, id).1["depth"], 90.0);
    assert_eq!(undo_len(&s), history + 1);
}

#[test]
fn the_current_type_with_an_object_without_effects_selected_too_records_no_undo_step() {
    let (mut s, a) = setup("threeD.extrude", json!({}));
    let b = NodeId(s.execute("path.create", &json!({"d":"M250 100L350 100L350 200L250 200Z"})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let history = undo_len(&s);
    s.execute("effect.setParams", &json!({"index": 0, "item": null, "aligned": true, "threeDType": "extrude"})).unwrap();
    assert_eq!(undo_len(&s), history);
    assert_eq!(s.doc().unwrap().doc, before);
}

#[test]
fn three_d_type_rejects_unknown_types_and_effects_that_are_not_3d() {
    let (mut s, _) = setup("threeD.extrude", json!({}));
    let before = s.doc().unwrap().doc.clone();
    for t in [json!("cube"), json!(""), json!(null), json!(2), json!("Plane"), json!("plane/../x")] {
        let p = json!({"index": 0, "threeDType": t});
        let err = s.execute("effect.setParams", &p).unwrap_err().to_string();
        assert!(err.contains("must be one of plane, extrude, revolve, inflate"), "{t}: {err}");
    }
    assert_eq!(s.doc().unwrap().doc, before);
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "item": null})).unwrap();
    let err = s.execute("effect.setParams", &json!({"index": 1, "threeDType": "plane"})).unwrap_err().to_string();
    assert!(err.contains("needs a 3D effect at index 1"), "{err}");
    let err = s.execute("effect.setParams", &json!({"index": 5, "threeDType": "plane"})).unwrap_err().to_string();
    assert!(err.contains("no effect at index 5"), "{err}");
}

#[test]
fn three_d_type_turns_every_selected_object_in_one_undo_step() {
    let (mut s, a) = setup("threeD.rotate", json!({}));
    let b = NodeId(s.execute("path.create", &json!({"d":"M250 100L350 100L350 200L250 200Z"})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"color":"#d63333"})).unwrap();
    s.execute("effect.apply", &json!({"effect":"threeD.rotate","item":null})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let history = undo_len(&s);
    s.execute("effect.setParams", &json!({"index": 0, "item": null, "aligned": true, "threeDType": "extrude"})).unwrap();
    assert_eq!(effect(&s, a).0, "threeD.extrude");
    assert_eq!(effect(&s, b).0, "threeD.extrude");
    assert_eq!(undo_len(&s), history + 1);
}

#[test]
fn params_for_the_new_type_pass_the_parameter_check() {
    let (s, _) = setup("threeD.rotate", json!({}));
    // `depth` is an Extrude & Bevel setting, checked against the effect the type makes.
    s.check_params("effect.setParams", &json!({"index": 0, "threeDType": "extrude", "params": {"depth": 20.0}})).unwrap();
}
