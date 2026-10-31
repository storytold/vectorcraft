//! The width profile library (`stroke.widthProfile.*`): saved profiles kept with the preferences
//! next to the built-in ones, applied by name with `stroke.set {profile}`.

use serde_json::{Value, json};
use vectorcraft_doc::WidthProfile;

use crate::*;

fn session_with_line() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let r = s.execute("shape.line", &json!({"x1": 10, "y1": 100, "x2": 150, "y2": 100})).unwrap();
    (s, NodeId(r["id"].as_u64().unwrap()))
}

fn profile(s: &Session, id: NodeId) -> Option<WidthProfile> {
    s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().unwrap().profile.clone()
}

fn custom() -> WidthProfile {
    WidthProfile { points: vec![(0.0, 0.2, 0.2), (0.3, 1.6, 0.8), (1.0, 0.5, 0.5)] }
}

/// Give the line `p` as its variable width (as the Width tool would).
fn set_profile(s: &mut Session, id: NodeId, p: WidthProfile) {
    s.edit("Width", |d, _| {
        d.node_mut(id).unwrap().appearance.stroke_mut().unwrap().profile = Some(p);
        Ok(())
    })
    .unwrap();
}

fn ids(list: &Value) -> Vec<&str> {
    list["profiles"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect()
}

#[test]
fn add_list_apply_delete_reset() {
    let (mut s, id) = session_with_line();
    let built_in: Vec<&str> = WidthProfile::PRESETS.iter().map(|p| p.id).collect();
    let list = s.execute("stroke.widthProfile.list", &json!({})).unwrap();
    assert_eq!(ids(&list), built_in);
    assert_eq!(list["current"], "uniform");
    // Nothing to save from a plain stroke, or from a built-in profile.
    assert!(s.execute("stroke.widthProfile.add", &json!({})).is_err());
    s.execute("stroke.set", &json!({"profile": "lens"})).unwrap();
    assert!(s.execute("stroke.widthProfile.add", &json!({})).is_err());

    set_profile(&mut s, id, custom());
    assert_eq!(s.execute("stroke.widthProfile.list", &json!({})).unwrap()["current"], "custom");
    let undo_depth = s.doc().unwrap().history.undo.len();
    assert_eq!(s.execute("stroke.widthProfile.add", &json!({})).unwrap()["name"], "Width Profile 1");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_depth, "the library is not part of the document");
    // Listed after the built-ins, and now the stroke's profile.
    let list = s.execute("stroke.widthProfile.list", &json!({})).unwrap();
    assert_eq!(ids(&list).last(), Some(&"Width Profile 1"));
    assert_eq!(list["current"], "Width Profile 1");
    let row = list["profiles"].as_array().unwrap().last().unwrap().clone();
    assert_eq!((row["builtIn"].clone(), row["points"][1].clone()), (json!(false), json!([0.3, 1.6, 0.8])));
    // The same profile can't be saved twice.
    assert!(s.execute("stroke.widthProfile.add", &json!({"name": "Again"})).is_err());

    // Apply it by name to another stroke.
    let other = NodeId(s.execute("shape.line", &json!({"x1": 10, "y1": 150, "x2": 150, "y2": 150})).unwrap()["id"].as_u64().unwrap());
    assert!(profile(&s, other).is_none());
    s.execute("stroke.set", &json!({"profile": "Width Profile 1"})).unwrap();
    assert_eq!(profile(&s, other), Some(custom()));
    assert!(s.execute("stroke.set", &json!({"profile": "Nope"})).is_err());

    // Delete it (the selected stroke's): strokes keep their widths.
    assert_eq!(s.execute("stroke.widthProfile.delete", &json!({})).unwrap()["deleted"], "Width Profile 1");
    assert_eq!(ids(&s.execute("stroke.widthProfile.list", &json!({})).unwrap()), built_in);
    assert_eq!(profile(&s, other), Some(custom()));
    assert!(s.execute("stroke.set", &json!({"profile": "Width Profile 1"})).is_err());

    // Reset removes every saved profile.
    s.execute("stroke.widthProfile.add", &json!({"name": " Ribbon "})).unwrap();
    set_profile(&mut s, other, WidthProfile { points: vec![(0.0, 1.0, 0.0), (1.0, 0.0, 1.0)] });
    s.execute("stroke.widthProfile.add", &json!({"name": "Twist"})).unwrap();
    assert_eq!(s.prefs.width_profiles.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Ribbon", "Twist"]);
    assert_eq!(s.execute("stroke.widthProfile.reset", &json!({})).unwrap()["removed"], 2);
    assert!(s.prefs.width_profiles.is_empty());
    assert!(s.execute("stroke.widthProfile.reset", &json!({})).is_err(), "nothing left to reset");
}

#[test]
fn built_ins_cant_be_deleted_and_names_stay_unique() {
    let (mut s, id) = session_with_line();
    set_profile(&mut s, id, custom());
    for taken in ["lens", "Taper Start", "UNIFORM"] {
        assert!(s.execute("stroke.widthProfile.add", &json!({"name": taken})).is_err(), "{taken}");
    }
    assert!(s.execute("stroke.widthProfile.add", &json!({"name": "  "})).is_err());
    s.execute("stroke.widthProfile.add", &json!({"name": "Mine"})).unwrap();
    for b in ["lens", "Lens", "uniform", "wave"] {
        let e = s.execute("stroke.widthProfile.delete", &json!({"name": b})).unwrap_err().to_string();
        assert!(e.contains("built in"), "{b}: {e}");
    }
    // With a built-in profile selected, Delete has nothing to delete.
    s.execute("stroke.set", &json!({"profile": "pinch"})).unwrap();
    assert!(s.execute("stroke.widthProfile.delete", &json!({})).is_err());
    assert!(s.execute("stroke.widthProfile.delete", &json!({"name": "Other"})).is_err());
    assert_eq!(s.next_profile_name(), "Width Profile 1");
    assert_eq!(s.execute("stroke.widthProfile.delete", &json!({"name": "Mine"})).unwrap()["deleted"], "Mine");
}

#[test]
fn saved_profiles_round_trip_with_the_preferences() {
    let (mut s, id) = session_with_line();
    set_profile(&mut s, id, custom());
    s.execute("stroke.widthProfile.add", &json!({"name": "Mine"})).unwrap();
    let json = s.prefs.to_json();
    assert_eq!(json["widthProfiles"][0]["name"], "Mine");
    let back: Prefs = serde_json::from_value(json).unwrap();
    assert_eq!(back, s.prefs);
    // Without saved profiles nothing is written, and older preference files load with none.
    assert!(Prefs::default().to_json().get("widthProfiles").is_none());
    let old: Prefs = serde_json::from_value(json!({"keyboardIncrement": 2})).unwrap();
    assert!(old.width_profiles.is_empty());
    // Setting and resetting other preferences keeps them.
    s.execute("prefs.set", &json!({"key": "keyboardIncrement", "value": 3})).unwrap();
    s.execute("prefs.reset", &json!({})).unwrap();
    assert_eq!(s.prefs.width_profiles.len(), 1);
    // A restarted session (preferences applied from the saved file) lists them again.
    let mut fresh = Session::new();
    fresh.apply_prefs(back);
    assert!(fresh.resolve_profile("Mine").is_some_and(|p| p == Some(custom())));
}

#[test]
fn every_built_in_applies_and_the_params_doc_names_them() {
    let (mut s, id) = session_with_line();
    let spec = command_specs().iter().find(|c| c.id == "stroke.set").unwrap();
    for p in WidthProfile::PRESETS {
        assert!(spec.params.contains(&format!("\"{}\"", p.id)), "{}", p.id);
        s.execute("stroke.set", &json!({"profile": p.id})).unwrap();
        assert_eq!(s.execute("stroke.widthProfile.list", &json!({})).unwrap()["current"], p.id);
        assert_eq!(profile(&s, id).map(|w| w.points), (p.id != "uniform").then(|| p.points.to_vec()));
    }
}
