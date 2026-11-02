//! The Gradient panel's presets (`gradient.presets.*`): the built-in gradient libraries by theme,
//! applied in place to the gradient behind a proxy, and gradients saved beside them with the
//! preferences.

use serde_json::{Value, json};
use vectorcraft_color::{Color, GradientKind, GradientMethod, Paint};

use crate::*;

/// A document with a selected rectangle filled with `fill` (`paint.setFill` params).
fn session_with(fill: Value) -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 50})).unwrap();
    s.execute("paint.setFill", &fill).unwrap();
    (s, NodeId(r["id"].as_u64().unwrap()))
}

fn paint(s: &Session, id: NodeId, stroke: bool) -> vectorcraft_color::GradientPaint {
    let n = s.doc().unwrap().doc.node(id).unwrap();
    match n.appearance.paint_at(None, !stroke).unwrap().clone() {
        Paint::Gradient(g) => *g,
        p => panic!("expected a gradient, got {p:?}"),
    }
}

fn names(list: &Value) -> Vec<String> {
    list["presets"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn built_in_presets_are_the_gradient_libraries_by_theme_with_unique_names() {
    let mut s = Session::new();
    let list = s.execute("gradient.presets.list", &json!({})).unwrap();
    let rows = list["presets"].as_array().unwrap();
    assert!(rows.len() > 40, "{} presets", rows.len());
    assert!(rows.iter().all(|r| r["builtIn"] == true && r["gradient"]["stops"].as_array().is_some_and(|a| a.len() >= 2)));
    // Grouped by library, the gradient libraries first, then the swatch libraries holding gradients.
    let mut libraries: Vec<&str> = rows.iter().map(|r| r["library"].as_str().unwrap()).collect();
    libraries.dedup();
    assert_eq!(libraries, ["Spectrum Gradients", "Sky Gradients", "Neutral Gradients", "Duotone Gradients", "Radial Glows", "Metallic Gradients"]);
    let mut seen = std::collections::HashSet::new();
    assert!(names(&list).iter().all(|n| seen.insert(n.to_lowercase())), "a name names one preset");
    let vignette = rows.iter().find(|r| r["name"] == "Vignette").unwrap();
    assert_eq!((vignette["library"].as_str(), vignette["gradient"]["kind"].as_str()), (Some("Neutral Gradients"), Some("radial")));
}

#[test]
fn applying_a_preset_edits_the_gradient_in_place_as_one_undo_step() {
    let (mut s, id) = session_with(json!({"gradient": {"angle": 30}}));
    s.execute("paint.editGradient", &json!({"dither": true})).unwrap();
    let before = paint(&s, id, false);
    let undo = s.doc().unwrap().history.undo.len();
    let r = s.execute("gradient.presets.apply", &json!({"name": "vignette"})).unwrap();
    assert_eq!((r["name"].as_str(), r["ids"].clone()), (Some("Vignette"), json!([id.0])));
    let g = paint(&s, id, false);
    let preset = s.gradient_preset("Vignette").unwrap().gradient;
    assert_eq!((g.gradient.kind, &g.gradient.stops), (GradientKind::Radial, &preset.stops));
    assert!(!g.gradient.dither, "the fill takes the preset's dither");
    assert_eq!(g.angle, before.angle, "the placement stays");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    assert_eq!(s.gradient_preset_name(&g.gradient).as_deref(), Some("Vignette"), "the gradient is that preset now");
    assert_eq!(s.gradient_preset_name(&before.gradient), None);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(paint(&s, id, false), before);
}

#[test]
fn a_preset_applies_to_the_stroke_and_to_a_solid_paint() {
    let (mut s, id) = session_with(json!({"color": "#336699"}));
    s.execute("paint.setStroke", &json!({"color": "#000000", "focus": false})).unwrap();
    s.execute("gradient.presets.apply", &json!({"name": "Gold"})).unwrap();
    let gold = s.gradient_preset("Gold").unwrap();
    assert_eq!((gold.library, paint(&s, id, false).gradient.stops), (Some("Metallic Gradients"), gold.gradient.stops.clone()));
    // With the Stroke proxy in front (or `stroke: true`) the stroke takes it; dither isn't sent.
    s.execute("paint.toggleActive", &json!({"fill": false})).unwrap();
    s.execute("gradient.presets.apply", &json!({"name": "Dawn"})).unwrap();
    assert_eq!(paint(&s, id, true).gradient.stops, s.gradient_preset("Dawn").unwrap().gradient.stops);
    assert_eq!(paint(&s, id, false).gradient.stops, gold.gradient.stops, "the fill keeps its gradient");
    for bad in [json!({}), json!({"name": 3}), json!({"name": "No Such Preset"})] {
        assert!(s.execute("gradient.presets.apply", &bad).is_err(), "{bad}");
    }
}

#[test]
fn saving_listing_applying_and_deleting_a_preset() {
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 0.4, "color": "#00ff00", "opacity": 0.5, "midpoint": 0.3}, {"offset": 1, "color": "#0000ff"}]);
    let (mut s, id) = session_with(json!({"gradient": {"kind": "radial", "stops": stops, "method": "perceptual"}}));
    // A stop linked to a document swatch is saved with its colour, unlinked.
    s.execute("swatch.new", &json!({"name": "Brand", "color": "#123456", "global": true})).unwrap();
    s.execute("gradient.selectStop", &json!({"index": 1})).unwrap();
    let mut linked = paint(&s, id, false).gradient.stops.clone();
    linked[1].set_color(Color::rgb8(0x12, 0x34, 0x56), Some(("Brand".into(), 1.0)));
    s.execute("paint.editGradient", &json!({"stops": vectorcraft_tools::params::stops_json(&linked)})).unwrap();
    assert_eq!(paint(&s, id, false).gradient.stops[1].swatch.as_deref(), Some("Brand"));
    assert_eq!(s.next_gradient_preset_name(), "Gradient Preset 1");
    assert_eq!(s.execute("gradient.presets.save", &json!({})).unwrap()["name"], "Gradient Preset 1");
    let saved = &s.prefs.gradient_presets[0].gradient;
    assert_eq!((saved.kind, saved.method, saved.stops.len()), (GradientKind::Radial, GradientMethod::Perceptual, 3));
    assert_eq!((saved.stops[1].swatch.as_ref(), saved.stops[1].color), (None, Color::rgb8(0x12, 0x34, 0x56)));
    assert_eq!(s.active_saved_gradient_preset(), Some("Gradient Preset 1"), "the shown gradient is that preset");
    // Listed after the built-in ones, without a library.
    let list = s.execute("gradient.presets.list", &json!({})).unwrap();
    let last = list["presets"].as_array().unwrap().last().unwrap().clone();
    assert_eq!((last["name"].as_str(), last["builtIn"].as_bool(), last["library"].clone()), (Some("Gradient Preset 1"), Some(false), Value::Null));
    // Names are unique, any case, and never a built-in preset's.
    for (name, why) in [("gradient preset 1", "taken"), ("Sunset", "built in"), ("  ", "empty")] {
        assert!(s.execute("gradient.presets.save", &json!({"name": name})).is_err(), "{why}");
    }
    s.execute("paint.editGradient", &json!({"reverse": true})).unwrap();
    assert_eq!(s.execute("gradient.presets.save", &json!({"name": " Mine "})).unwrap()["name"], "Mine");
    // A saved preset applies to other art like a built-in one.
    let r = s.execute("shape.ellipse", &json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    let other = NodeId(r["id"].as_u64().unwrap());
    s.execute("gradient.presets.apply", &json!({"name": "Mine"})).unwrap();
    assert_eq!(paint(&s, other, false).gradient, s.prefs.gradient_presets[1].gradient);
    // Delete: built-in ones stay; without a name, the saved preset the active proxy shows.
    assert!(s.execute("gradient.presets.delete", &json!({"name": "Sunset"})).is_err());
    assert!(s.execute("gradient.presets.delete", &json!({"name": "Nothing"})).is_err());
    assert_eq!(s.execute("gradient.presets.delete", &json!({})).unwrap()["deleted"], "Mine");
    assert!(s.execute("gradient.presets.delete", &json!({})).is_err(), "the ellipse's gradient is no saved preset now");
    assert_eq!(s.execute("gradient.presets.delete", &json!({"name": "GRADIENT PRESET 1"})).unwrap()["deleted"], "Gradient Preset 1");
    assert!((crate::find_command("gradient.presets.delete").unwrap().enabled)(&s).is_err(), "nothing left to delete");
}

#[test]
fn saving_needs_a_gradient_behind_the_proxy() {
    let (mut s, _) = session_with(json!({"color": "#336699"}));
    assert!((crate::find_command("gradient.presets.save").unwrap().enabled)(&s).is_err());
    assert!(s.execute("gradient.presets.save", &json!({})).is_err());
    // A gradient stroke is saved with the Stroke proxy in front.
    s.execute("paint.setStroke", &json!({"gradient": {"kind": "radial"}, "focus": false})).unwrap();
    assert!(s.execute("gradient.presets.save", &json!({})).is_err(), "the Fill is in front");
    s.execute("paint.toggleActive", &json!({"fill": false})).unwrap();
    assert_eq!(s.execute("gradient.presets.save", &json!({"name": "Ring"})).unwrap()["name"], "Ring");
    assert_eq!(s.prefs.gradient_presets[0].gradient.kind, GradientKind::Radial);
    assert!(s.execute("gradient.presets.save", &json!({"name": 7})).is_err(), "a name is text");
}

#[test]
fn saved_presets_round_trip_with_the_preferences() {
    let (mut s, _) = session_with(json!({"gradient": {}}));
    s.execute("gradient.presets.save", &json!({"name": "Mine"})).unwrap();
    let json = s.prefs.to_json();
    assert_eq!(json["gradientPresets"][0]["name"], "Mine");
    let back: Prefs = serde_json::from_value(json).unwrap();
    assert_eq!(back, s.prefs);
    // Without saved presets nothing is written, and older preference files load with none.
    assert!(Prefs::default().to_json().get("gradientPresets").is_none());
    let old: Prefs = serde_json::from_value(json!({"keyboardIncrement": 2})).unwrap();
    assert!(old.gradient_presets.is_empty());
    // Resetting the preferences keeps them.
    s.execute("prefs.reset", &json!({})).unwrap();
    assert_eq!(s.prefs.gradient_presets.len(), 1);
    // A restarted session (preferences applied from the saved file) lists them again.
    let mut fresh = Session::new();
    fresh.apply_prefs(back);
    assert!(fresh.gradient_preset("mine").is_some_and(|e| e.library.is_none()));
}
