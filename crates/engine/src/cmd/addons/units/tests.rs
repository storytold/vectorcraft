//! Unit Converter's commands: a length in every unit, read as number fields read it, the selection's
//! size as the Transform panel measures it, bad params refused, and nothing in the document changed.

use serde_json::{Value, json};
use vectorcraft_doc::Unit;

use crate::{EngineError, Session};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn convert(s: &mut Session, p: Value) -> Value {
    s.execute("addon.units.convert", &p).unwrap()
}

/// The row of `r` for the unit with key `key`.
fn row<'a>(r: &'a Value, key: &str) -> &'a Value {
    r["units"].as_array().unwrap().iter().find(|u| u["unit"] == key).unwrap_or_else(|| panic!("{key} in {r}"))
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn a_length_comes_back_in_every_unit_in_order() {
    let mut s = session();
    let r = convert(&mut s, json!({"value": "10mm"}));
    assert!(near(r["points"].as_f64().unwrap(), 720.0 / 25.4));
    let keys: Vec<&str> = r["units"].as_array().unwrap().iter().map(|u| u["unit"].as_str().unwrap()).collect();
    assert_eq!(keys, Unit::EAST_ASIAN.map(Unit::key));
    assert_eq!(row(&r, "millimeters")["number"], "10");
    assert_eq!(row(&r, "millimeters")["text"], "10 mm");
    assert_eq!(row(&r, "centimeters")["text"], "1 cm");
    assert_eq!(row(&r, "inches")["text"], "0.3937 in");
    assert_eq!(row(&r, "points")["text"], "28.346 pt");
    assert_eq!(row(&r, "q")["text"], "40 Q");
    assert_eq!(row(&r, "ha")["text"], "40 H");
    assert_eq!(row(&r, "pixels")["number"], row(&r, "points")["number"], "pixels are points");
    assert_eq!(row(&r, "feetInches")["label"], "Feet & Inches");
    // Each row is what a field of that unit shows for the length.
    for u in Unit::EAST_ASIAN {
        let pt = 720.0 / 25.4;
        assert_eq!(row(&r, u.key())["text"], u.format(pt), "{u:?}");
        assert_eq!(row(&r, u.key())["number"], u.number(pt), "{u:?}");
    }
}

#[test]
fn values_read_as_number_fields_read_them() {
    let mut s = session();
    let pt = |s: &mut Session, p: Value| convert(s, p)["points"].as_f64().unwrap();
    assert!(near(pt(&mut s, json!({"value": "1in/2"})), 36.0));
    assert!(near(pt(&mut s, json!({"value": "2p6"})), 30.0));
    assert!(near(pt(&mut s, json!({"value": "1080/2 px"})), 540.0));
    assert!(near(pt(&mut s, json!({"value": "3cm*50%"})), 1.5 * 72.0 / 2.54));
    assert!(near(pt(&mut s, json!({"value": "13q"})), 13.0 * 72.0 / 101.6));
    // A number without a unit is in `unit`, points by default.
    assert!(near(pt(&mut s, json!({"value": 12})), 12.0));
    assert!(near(pt(&mut s, json!({"value": 2, "unit": "in"})), 144.0));
    assert!(near(pt(&mut s, json!({"value": "2", "unit": "Inches"})), 144.0));
    assert!(near(pt(&mut s, json!({"value": "3+1", "unit": "cm"})), 4.0 * 72.0 / 2.54));
    assert!(near(pt(&mut s, json!({"value": 6, "unit": "ha"})), 6.0 * 72.0 / 101.6));
    assert!(near(pt(&mut s, json!({"value": 1, "unit": "feetInches"})), 864.0));
    // A suffix wins over `unit`.
    assert!(near(pt(&mut s, json!({"value": "1in", "unit": "mm"})), 72.0));
    // A decimal comma reads as a point, as in the number fields.
    assert!(near(pt(&mut s, json!({"value": "0,5 in"})), 36.0));
    assert!(near(pt(&mut s, json!({"value": "1,5", "unit": "in"})), 108.0));
    // Each number of a sum reads its own separator.
    assert!(near(pt(&mut s, json!({"value": "100.5 mm-0,5", "unit": "mm"})), 100.0 * 72.0 / 25.4));
    // A comma in a sum is a decimal comma, as in the number fields: 1,500+2 is 3.5.
    assert!(near(pt(&mut s, json!({"value": "1,500+2", "unit": "mm"})), 3.5 * 72.0 / 25.4));
    // A sum of 128 signs still reads.
    assert!(near(pt(&mut s, json!({"value": format!("1{}", "+1".repeat(128))})), 129.0));
}

#[test]
fn keys_the_commands_do_not_take_are_refused() {
    let s = session();
    s.check_params("addon.units.convert", &json!({"value": 1, "unit": "mm"})).unwrap();
    s.check_params("addon.units.selectionSize", &json!({})).unwrap();
    for (id, p) in [
        ("addon.units.convert", json!({"value": 1, "units": "mm"})),
        ("addon.units.convert", json!({"value": 1, "ids": [1]})),
        ("addon.units.selectionSize", json!({"ids": [1]})),
        ("addon.units.selectionSize", json!({"value": 1})),
    ] {
        match s.check_params(id, &p) {
            Err(EngineError::BadParams { cmd, .. }) => assert_eq!(cmd, id, "{p}"),
            other => panic!("{id} {p}: expected BadParams, got {other:?}"),
        }
    }
}

#[test]
fn the_selection_size_follows_preview_bounds_and_selected_anchors() {
    let mut s = session();
    let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 72, "height": 36})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [id], "color": "#000000"})).unwrap();
    s.execute("stroke.set", &json!({"ids": [id], "weight": 10})).unwrap();
    let wh = |s: &mut Session| {
        let r = s.execute("addon.units.selectionSize", &json!({})).unwrap();
        (r["width"]["points"].as_f64().unwrap(), r["height"]["points"].as_f64().unwrap())
    };
    assert_eq!(wh(&mut s), (72.0, 36.0), "geometric bounds");
    // Use Preview Bounds: the stroke's outer half on every side.
    s.execute("prefs.set", &json!({"key": "usePreviewBounds", "value": true})).unwrap();
    let (w, h) = wh(&mut s);
    assert!(near(w, 82.0) && near(h, 46.0), "{w} × {h}");
    s.execute("prefs.set", &json!({"key": "usePreviewBounds", "value": false})).unwrap();
    // Two neighboring anchors selected (Direct Selection): the box around them, one edge of the
    // rectangle.
    s.execute("select.anchors", &json!({"id": id, "anchors": [[0, 0], [0, 1]], "mode": "set"})).unwrap();
    let (w, h) = wh(&mut s);
    assert!((near(w, 72.0) && near(h, 0.0)) || (near(w, 0.0) && near(h, 36.0)), "{w} × {h}");
}

#[test]
fn it_needs_no_document_and_changes_nothing() {
    let mut s = Session::new();
    assert!(near(convert(&mut s, json!({"value": "1in"}))["points"].as_f64().unwrap(), 72.0));
    let mut s = session();
    let (before, steps) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    convert(&mut s, json!({"value": "5mm"}));
    assert_eq!(s.doc().unwrap().history.undo.len(), steps);
    assert!(*s.doc().unwrap().doc == *before);
}

#[test]
fn bad_values_and_units_are_refused() {
    let mut s = session();
    for p in [
        json!({}),
        json!({"value": null}),
        json!({"value": "abc"}),
        json!({"value": "10 furlongs"}),
        json!({"value": "1in/0"}),
        json!({"value": "1e400"}),
        // Finite in points, too large in Q and Ha.
        json!({"value": 1.5e308}),
        json!({"value": "1.5e308"}),
        // A thousands comma is refused, as the number fields refuse it.
        json!({"value": "1,000 mm"}),
        json!({"value": "1,000.5"}),
        json!({"value": "+1,000"}),
        json!({"value": "1,000\""}),
        json!({"value": "1,000'"}),
        // A long expression is refused instead of exhausting the stack.
        json!({"value": format!("1{}", "+1".repeat(200_000))}),
        json!({"value": format!("1{}", "+1".repeat(129))}),
        json!({"value": true}),
        json!({"value": [1]}),
        json!({"value": 1, "unit": "furlongs"}),
        json!({"value": 1, "unit": 5}),
        json!({"value": 1, "unit": ["mm"]}),
        // Only an omitted `unit` is points.
        json!({"value": 1, "unit": null}),
    ] {
        match s.execute("addon.units.convert", &p) {
            Err(EngineError::BadParams { cmd, .. }) => assert_eq!(cmd, "addon.units.convert", "{p}"),
            other => panic!("{p}: expected BadParams, got {other:?}"),
        }
    }
}

#[test]
fn a_long_value_or_unit_is_refused_with_a_short_message() {
    let mut s = session();
    let long = "9".repeat(100_000);
    for (p, says) in [
        (json!({"value": long.clone()}), format!("`value` must be a number, not \"{}…\"", "9".repeat(64))),
        (json!({"value": [long.clone()]}), "`value` must be a number, not […]".to_string()),
        (json!({"value": 1, "unit": long.clone()}), format!("unknown units `{}…`", "9".repeat(64))),
        (json!({"value": 1, "unit": "furlongs"}), "unknown units `furlongs`".to_string()),
        (json!({"value": "a\"b\nc"}), "`value` must be a number, not \"a\\\"b\\nc\"".to_string()),
    ] {
        match s.execute("addon.units.convert", &p) {
            Err(e @ EngineError::BadParams { .. }) => assert!(e.to_string().ends_with(&says), "{e}"),
            other => panic!("expected BadParams, got {other:?}"),
        }
    }
}

#[test]
fn the_selection_size_is_the_transform_panels() {
    let mut s = session();
    assert!(s.execute("addon.units.selectionSize", &json!({})).is_err(), "nothing selected");
    let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 72, "height": 36})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let steps = s.doc().unwrap().history.undo.len();
    let r = s.execute("addon.units.selectionSize", &json!({})).unwrap();
    assert!(near(r["width"]["points"].as_f64().unwrap(), 72.0));
    assert!(near(r["height"]["points"].as_f64().unwrap(), 36.0));
    assert_eq!(row(&r["width"], "inches")["text"], "1 in");
    assert_eq!(row(&r["height"], "picas")["text"], "3 p");
    // A turned object keeps its own sides, as the Transform panel's W and H show them.
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    let r = s.execute("addon.units.selectionSize", &json!({})).unwrap();
    let b = s.transform_box(&s.doc().unwrap().selection.objects).unwrap();
    assert!(near(r["width"]["points"].as_f64().unwrap(), b.rect.width()));
    assert!(near(r["width"]["points"].as_f64().unwrap(), 72.0));
    assert!(near(r["height"]["points"].as_f64().unwrap(), 36.0));
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "only the rotation is a step");
}
