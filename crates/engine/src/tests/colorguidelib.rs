//! The Color Guide's Limit to Library: `color.harmony {limitTo}` snaps every colour to a library or
//! to the document's swatches, as Recolor Artwork's `limitTo` does.

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::*;

fn hexes(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|h| h.as_str().unwrap().to_string()).collect()
}

/// Every colour `color.harmony` answers: the harmony colours and the whole grid.
fn all_colours(r: &Value) -> Vec<String> {
    let mut out = hexes(&r["colors"]);
    for row in r["grid"].as_array().unwrap() {
        out.extend(hexes(row));
    }
    out
}

fn library_hexes(s: &mut Session, library: &str) -> HashSet<String> {
    let lib = s.execute("swatch.library.get", &json!({ "library": library })).unwrap();
    lib["swatches"].as_array().unwrap().iter().filter_map(|w| w["hex"].as_str()).map(str::to_string).collect()
}

#[test]
fn every_guide_colour_is_a_library_member() {
    let mut s = Session::new();
    for library in ["web-safe-216", "earth-tones"] {
        let members = library_hexes(&mut s, library);
        for rule in ["triad", "monochromatic", "highContrast3", "pentagram"] {
            let r = s.execute("color.harmony", &json!({"color": "#3a7bd5", "rule": rule, "steps": 6, "limitTo": library})).unwrap();
            let out = all_colours(&r);
            assert_eq!(out.len(), r["colors"].as_array().unwrap().len() * 14);
            assert!(out.iter().all(|h| members.contains(h)), "{library} {rule}: {out:?}");
        }
    }
    // A name works as well as an id; "" and no limit leave the colours free.
    let by_name = s.execute("color.harmony", &json!({"color": "#3a7bd5", "rule": "triad", "limitTo": "Web Safe 216"})).unwrap();
    let by_id = s.execute("color.harmony", &json!({"color": "#3a7bd5", "rule": "triad", "limitTo": "web-safe-216"})).unwrap();
    assert_eq!(by_name, by_id);
    let free = s.execute("color.harmony", &json!({"color": "#3a7bd5", "rule": "triad", "limitTo": ""})).unwrap();
    assert_eq!(free["colors"][0], "#3a7bd5");
    assert_ne!(by_id["colors"][0], "#3a7bd5", "a library without the base snaps it too");
    assert!(s.execute("color.harmony", &json!({"color": "#3a7bd5", "rule": "triad", "limitTo": "nope"})).is_err());
    assert!(s.execute("color.harmony", &json!({"color": "#3a7bd5", "rule": "triad", "limitTo": "document"})).is_err(), "no document");
}

/// A `limitTo` that is not a string names no library: `color.harmony` refuses it with the error it
/// gives for a name it does not know, and reads a null as no limit.
#[test]
fn a_limit_that_is_not_a_string_names_no_library() {
    let mut s = Session::new();
    for (limit, shown) in [(json!(5), "`5`"), (json!(true), "`true`"), (json!(["web-safe-216"]), "`[\"web-safe-216\"]`")] {
        let e = s.execute("color.harmony", &json!({"color": "#3a7bd5", "rule": "triad", "limitTo": limit})).unwrap_err().to_string();
        assert!(e.contains(&format!("no swatch library {shown}")), "{limit}: {e}");
    }
    let free = s.execute("color.harmony", &json!({"color": "#3a7bd5", "rule": "triad", "limitTo": null})).unwrap();
    assert_eq!(free["colors"][0], "#3a7bd5");
}

#[test]
fn document_swatches_limit_the_guide_and_recolor() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let doc_hexes =
        |s: &Session| -> HashSet<String> { s.doc().unwrap().doc.swatches_iter().filter_map(|w| w.paint.color()).map(|c| c.to_hex()).collect() };
    s.execute("swatch.newGroup", &json!({"name": "Mine", "colors": ["#123456", "#fedcba"]})).unwrap();
    let members = doc_hexes(&s);
    assert!(members.contains("#123456"), "colour groups count");
    let r = s.execute("color.harmony", &json!({"color": "#103050", "rule": "splitComplementary", "limitTo": "document"})).unwrap();
    assert!(all_colours(&r).iter().all(|h| members.contains(h)), "{r}");
    assert_eq!(r["colors"][0], "#123456", "the base snaps to the nearest swatch");
    // Recolor Artwork limits to the document's swatches the same way.
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#112255"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let r = s.execute("recolor.reduce", &json!({"limitTo": "document"})).unwrap();
    let to = cmd::color_value(&r["map"][0]["to"]).unwrap().to_hex();
    assert!(members.contains(&to) && to != "#112255", "{r}");
}
