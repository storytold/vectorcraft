//! Pattern editing: the Tile Edge Color preference and Pattern Options' Show Swatch Bounds.

use serde_json::{Value, json};
use vectorcraft_doc::pattern::PatternDef;

use crate::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// A session editing pattern "Dots" made from a 10×10 rectangle.
fn editing() -> Session {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}));
    run(&mut s, "object.pattern.make", json!({"name": "Dots", "width": 20, "height": 20}));
    s
}

fn def(s: &Session) -> PatternDef {
    s.doc().unwrap().doc.pattern("Dots").unwrap().clone()
}

#[test]
fn show_swatch_bounds_is_a_pattern_option_that_saves() {
    let mut s = editing();
    assert!(!def(&s).show_swatch_bounds);
    run(&mut s, "pattern.options", json!({"showSwatchBounds": true}));
    assert!(def(&s).show_swatch_bounds);
    assert_eq!(run(&mut s, "pattern.list", json!({}))["patterns"][0]["showSwatchBounds"], true);
    // One undo step.
    run(&mut s, "edit.undo", json!({}));
    assert!(!def(&s).show_swatch_bounds);
    run(&mut s, "edit.redo", json!({}));
    // Saved with the document; files from before the option load with it off.
    let bytes = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert_eq!(back.patterns, s.doc().unwrap().doc.patterns);
    assert!(back.pattern("Dots").unwrap().show_swatch_bounds);
    let mut old = serde_json::to_value(def(&s)).unwrap();
    old.as_object_mut().unwrap().remove("showSwatchBounds");
    assert!(!serde_json::from_value::<PatternDef>(old).unwrap().show_swatch_bounds);
}

#[test]
fn tile_edge_color_is_a_preference_that_redraws_pattern_editing() {
    let mut s = editing();
    let default = run(&mut s, "prefs.get", json!({"key": "patternTileEdgeColor"}));
    let [r, g, b] = vectorcraft_doc::LAYER_COLORS[0].1;
    assert_eq!(default, vectorcraft_color::Color::rgb8(r, g, b).to_hex());
    let rev = s.doc().unwrap().revision;
    run(&mut s, "prefs.set", json!({"key": "patternTileEdgeColor", "value": "FF0000"}));
    assert_eq!(s.prefs.pattern_tile_edge_color, "#ff0000");
    assert!(s.doc().unwrap().revision > rev, "the canvas redraws the tile edge");
    assert!(s.execute("prefs.set", &json!({"key": "patternTileEdgeColor", "value": "red"})).is_err());
    // Saved with the preferences; older preferences get the default.
    let back: Prefs = serde_json::from_value(s.prefs.to_json()).unwrap();
    assert_eq!(back.pattern_tile_edge_color, "#ff0000");
    let mut old = s.prefs.to_json();
    old.as_object_mut().unwrap().remove("patternTileEdgeColor");
    assert_eq!(json!(serde_json::from_value::<Prefs>(old).unwrap().pattern_tile_edge_color), default);
}
