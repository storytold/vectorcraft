//! Scale Strokes & Effects on effects: the catalogue's distance parameters and `scale_effect`.

use serde_json::json;
use vectorcraft_doc::Effect;

use crate::*;

#[test]
fn every_distance_parameter_is_a_parameter() {
    for e in effect_catalog() {
        for k in e.lengths.always.iter().chain(e.lengths.absolute) {
            assert!(e.defaults.get(*k).is_some() || e.params.contains(&format!("{k}?:")), "{}: `{k}` is not a parameter", e.id);
        }
        if !e.lengths.absolute.is_empty() {
            assert!(e.defaults.get("relative").is_some(), "{}: absolute lengths need a `relative` param", e.id);
        }
    }
}

#[test]
fn scale_effect_scales_distances_only() {
    let mut e = new_effect("stylize.dropShadow", &json!({"x": 3, "y": "4 pt"})).unwrap();
    scale_effect(&mut e, 2.0);
    assert_eq!((&e.params["x"], &e.params["y"], &e.params["blur"]), (&json!(6.0), &json!(8.0), &json!(10.0)));
    assert_eq!(e.params["opacity"], json!(75.0));
    // Rasterize's Add Around Object is a distance; its resolution isn't.
    let mut r = new_effect(RASTERIZE, &json!({"addAround": 10, "resolution": 150})).unwrap();
    scale_effect(&mut r, 2.0);
    assert_eq!((&r.params["addAround"], &r.params["resolution"]), (&json!(20.0), &json!(150)));
    // Outline Stroke without a weight follows its stroke's: nothing to scale.
    let mut o = Effect { id: "path.outlineStroke".into(), params: json!(null), visible: true };
    scale_effect(&mut o, 2.0);
    assert_eq!(o.params, json!(null));
    // Relative tweaks are percentages of the object's size.
    let mut t = new_effect("distort.tweak", &json!({"h": 10, "relative": true})).unwrap();
    scale_effect(&mut t, 2.0);
    assert_eq!(t.params["h"], json!(10));
}
