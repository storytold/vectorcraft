//! Preferences ▸ Units: the General unit is the open document's units (Preferences and Document
//! Setup set it, new documents start in the preference), Stroke and Type are preferences, and the
//! canvas measurement labels follow them.

use serde_json::json;
use vectorcraft_doc::Unit;
use vectorcraft_tools::{Mods, Overlay, PointerEvent, PointerKind};

use crate::cmd::prefscmds::{PREF_SPECS, PrefKind, UNITS};
use crate::units::Measure;
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn units(s: &Session) -> Unit {
    s.doc().unwrap().doc.units
}

#[test]
fn preferences_general_sets_the_open_documents_units_as_one_undo_step() {
    let mut s = session();
    assert_eq!(s.general_unit(), Unit::Points);
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("prefs.set", &json!({"key": "unitsGeneral", "value": "Millimeters"})).unwrap();
    assert_eq!((units(&s), s.general_unit(), s.default_units()), (Unit::Millimeters, Unit::Millimeters, Unit::Millimeters));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    // Setting it again (the Preferences dialog's OK sends every value) adds no step.
    s.execute("prefs.set", &json!({"values": {"unitsGeneral": "millimeters", "keyboardIncrement": 2}})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(units(&s), Unit::Points);
    assert_eq!(s.prefs.units_general, "millimeters", "undo is the document's; the preference stays");
}

#[test]
fn new_documents_start_in_the_preference_and_document_setup_only_changes_the_document() {
    let mut s = Session::new();
    assert_eq!(s.general_unit(), Unit::Points, "no document: the preference");
    s.execute("prefs.set", &json!({"key": "unitsGeneral", "value": "inches"})).unwrap();
    assert_eq!(s.general_unit(), Unit::Inches);
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    assert_eq!(units(&s), Unit::Inches);
    s.execute("file.new", &json!({"width": 100, "height": 100, "units": "Pixels"})).unwrap();
    assert_eq!((units(&s), s.general_unit()), (Unit::Pixels, Unit::Pixels), "given units win");
    s.execute("document.setUnits", &json!({"units": "cm"})).unwrap();
    assert_eq!((units(&s), s.default_units()), (Unit::Centimeters, Unit::Inches));
    // Each document keeps its own units.
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    assert_eq!(s.general_unit(), Unit::Inches);
    assert!(s.execute("file.new", &json!({"units": "furlongs"})).is_err());
}

#[test]
fn document_setup_without_a_document_fails() {
    let mut s = Session::new();
    assert!(s.execute("document.setUnits", &json!({"units": "mm"})).is_err());
    // The preference alone still changes.
    s.execute("prefs.set", &json!({"key": "unitsGeneral", "value": "Millimeters"})).unwrap();
    assert_eq!(s.prefs.units_general, "millimeters");
}

#[test]
fn every_unit_is_a_units_choice() {
    let want: Vec<(&str, &str)> = Unit::ALL.iter().map(|u| (u.key(), u.label())).collect();
    assert_eq!(UNITS, want.as_slice());
    let mut s = session();
    for u in Unit::ALL {
        s.execute("prefs.set", &json!({"values": {"unitsGeneral": u.label(), "unitsStroke": u.key(), "unitsType": u.key()}})).unwrap();
        assert_eq!((s.general_unit(), s.type_unit()), (u, u));
        assert_eq!(s.stroke_unit(), u);
    }
}

#[test]
fn stroke_and_type_units_are_preferences() {
    let mut s = session();
    s.execute("prefs.set", &json!({"values": {"unitsStroke": "millimeters", "unitsType": "pixels"}})).unwrap();
    assert_eq!((s.unit(Measure::General), s.unit(Measure::Stroke), s.unit(Measure::Type)), (Unit::Points, Unit::Millimeters, Unit::Pixels));
    assert_eq!(units(&s), Unit::Points, "they leave the document alone");
}

#[test]
fn length_preferences_take_typed_units_and_say_what_they_follow() {
    let mut s = session();
    s.execute("prefs.set", &json!({"values": {"keyboardIncrement": "1 mm", "gridlineEvery": "1 in", "typeSizeIncrement": 3}})).unwrap();
    assert!((s.prefs.keyboard_increment - 72.0 / 25.4).abs() < 1e-9);
    assert_eq!((s.prefs.gridline_every, s.prefs.type_size_increment), (72.0, 3.0));
    assert!(s.execute("prefs.set", &json!({"key": "pasteOffset", "value": "1 yd"})).is_err(), "out of range");
    let list = s.execute("prefs.list", &json!({})).unwrap();
    let row = |k: &str| list.as_array().unwrap().iter().find(|r| r["key"] == k).unwrap().clone();
    assert_eq!((row("keyboardIncrement")["measure"].clone(), row("keyboardIncrement")["unit"].clone()), (json!("general"), json!("pt")));
    assert_eq!(row("baselineShiftIncrement")["measure"], json!("type"));
    assert!(row("snappingTolerance")["measure"].is_null(), "a screen distance");
    let lengths = PREF_SPECS.iter().filter(|p| matches!(p.kind, PrefKind::Length { .. })).count();
    assert_eq!(lengths, 6);
}

#[test]
fn canvas_measurement_labels_follow_the_units() {
    let mut s = session();
    s.execute("prefs.set", &json!({"values": {"unitsGeneral": "millimeters", "unitsStroke": "inches"}})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("rectangle", v).unwrap();
    for (kind, x, y) in [(PointerKind::Down, 0.0, 0.0), (PointerKind::Drag, 72.0, 36.0)] {
        s.pointer(&PointerEvent::new(kind, x, y).with_mods(Mods::default()), v).unwrap();
    }
    let labels: Vec<String> =
        s.overlays(v).into_iter().filter_map(|o| if let Overlay::Measure { text, .. } = o { Some(text) } else { None }).collect();
    assert_eq!(labels, ["W: 25.40 mm\nH: 12.70 mm"]);
}

#[test]
fn units_survive_a_native_round_trip() {
    let mut s = session();
    s.execute("document.setUnits", &json!({"units": "Feet & Inches"})).unwrap();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(back.units, Unit::FeetInches);
}

#[test]
fn distance_params_take_a_unit() {
    let mut s = session();
    let id = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "Hi"})).unwrap()["id"].clone();
    s.execute("select.set", &json!({ "ids": [id] })).unwrap();
    s.execute("text.setStyle", &json!({"size": "0.5 in", "leading": "1in", "tracking": "20"})).unwrap();
    let n = s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().clone();
    let NodeKind::Text(t) = &n.kind else { panic!("not text") };
    let st = t.first_style();
    assert_eq!((st.size, st.leading, st.tracking), (36.0, Some(72.0), 20.0));
}
