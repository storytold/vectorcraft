#![allow(clippy::unwrap_used, clippy::panic)]
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_render::effects::{LIGHTING_PRESETS, matching_lighting_preset};

use crate::Session;

fn setup(effect: &str, item: Value) -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":400,"height":400})).unwrap();
    let id = NodeId(s.execute("path.create", &json!({"d":"M100 100L200 100L200 200L100 200Z"})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"color":"#338ad6"})).unwrap();
    s.execute(
        "effect.apply",
        &json!({"effect":effect,"item":item,"params":{
            "rotationX":13,"rotationY":17,"rotationZ":9,"shade":false,"ambient":17,
            "lights":[{"color":"#ff8040"},{"color":"#4080ff","rotation":-35}]
        }}),
    )
    .unwrap();
    (s, id)
}

#[test]
fn lighting_presets_preserve_source_and_view_and_round_trip_in_one_undo_step() {
    for effect in ["threeD.revolve", "threeD.extrude", "threeD.inflate", "threeD.rotate"] {
        for (i, preset) in LIGHTING_PRESETS.iter().enumerate() {
            let (mut s, id) = setup(effect, Value::Null);
            s.doc_mut().unwrap().mark_saved();
            let before = s.doc().unwrap().doc.clone();
            let history = s.doc().unwrap().history.undo.len();
            let p = json!({"index":0,"item":null,"preset":preset.id});
            s.check_params("effect.lightingPreset", &p).unwrap();
            s.execute("effect.lightingPreset", &p).unwrap();
            let after = s.doc().unwrap().doc.clone();
            let actual = &after.node(id).unwrap().appearance.effects[0].params;
            let mut expected = before.node(id).unwrap().appearance.effects[0].params.clone();
            expected.as_object_mut().unwrap().extend(preset.params().as_object().unwrap().clone());
            assert_eq!(*actual, expected);
            assert_eq!(matching_lighting_preset(actual), Some(i));
            assert_eq!(after.node(id).unwrap().path_data(), before.node(id).unwrap().path_data());
            assert_eq!(actual["rotationX"], 13);
            assert_eq!(actual["shade"], false);
            assert_eq!(s.doc().unwrap().history.undo.len(), history + 1);
            let reopened = vectorcraft_format::load(&vectorcraft_format::save(&after, false)).unwrap();
            assert_eq!(reopened.node(id).unwrap().appearance.effects[0].params, *actual);
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(s.doc().unwrap().doc, before);
            assert!(!s.doc().unwrap().is_dirty());
            s.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(s.doc().unwrap().doc, after);
        }
    }
}

#[test]
fn lighting_presets_target_fill_effects_and_reject_invalid_requests_atomically() {
    let (mut s, id) = setup("threeD.rotate", json!(0));
    for p in [
        json!({"index":0,"item":0,"preset":"unknown"}),
        json!({"index":0,"item":0,"preset":12}),
        json!({"index":1,"item":0,"preset":"right"}),
        json!({"index":0,"item":null,"preset":"right"}),
        json!({"index":0,"item":0,"preset":"right","params":{"ambient":200}}),
    ] {
        s.doc_mut().unwrap().mark_saved();
        let before = s.doc().unwrap().doc.clone();
        let history = s.doc().unwrap().history.undo.len();
        assert!(s.execute("effect.lightingPreset", &p).is_err(), "{p}");
        assert_eq!(s.doc().unwrap().doc, before);
        assert_eq!(s.doc().unwrap().history.undo.len(), history);
        assert!(!s.doc().unwrap().is_dirty());
    }
    s.execute("effect.lightingPreset", &json!({"ids":[id.0],"index":0,"item":0,"preset":"topLeft"})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap();
    assert!(n.appearance.effects.is_empty());
    assert_eq!(n.appearance.effects_at(Some(0)).unwrap()[0].params["lightRotation"], 135.0);
}

#[test]
fn lighting_presets_reject_a_mixed_selection_before_changing_any_effect() {
    let (mut s, first) = setup("threeD.rotate", Value::Null);
    let second = s.execute("shape.rectangle", &json!({"x":250,"y":100,"width":80,"height":80})).unwrap()["id"].clone();
    s.execute("effect.apply", &json!({"effect":"path.offsetPath","item":null})).unwrap();
    s.execute("select.set", &json!({"ids":[first.0,second]})).unwrap();
    s.doc_mut().unwrap().mark_saved();
    let before = s.doc().unwrap().doc.clone();
    let history = s.doc().unwrap().history.undo.len();
    assert!(s.execute("effect.lightingPreset", &json!({"index":0,"item":null,"preset":"right"})).is_err());
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    assert!(!s.doc().unwrap().is_dirty());
}
