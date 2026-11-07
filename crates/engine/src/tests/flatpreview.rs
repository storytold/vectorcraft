//! Flattener Preview (M3.94): `flattener.preview` reports what flattening the document would touch,
//! without changing it.

use serde_json::{Value, json};

use crate::cmd::flatten::Highlight;
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

/// An unstroked rectangle filled with `color`.
fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64, color: &str) -> u64 {
    let id = run(s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h}))["id"].as_u64().unwrap();
    run(s, "paint.setFill", json!({"color": color, "ids": [id]}));
    run(s, "paint.setStroke", json!({"none": true, "ids": [id]}));
    id
}

fn preview(s: &mut Session, p: Value) -> Value {
    run(s, "flattener.preview", p)
}

fn ids(v: &Value) -> Vec<u64> {
    v["regions"].as_array().unwrap().iter().filter_map(|r| r["id"].as_u64()).collect()
}

#[test]
fn type_under_a_transparent_rectangle_counts_as_affected() {
    let mut s = session();
    let text = run(&mut s, "text.create", json!({"x": 20, "y": 80, "text": "Under", "size": 48}))["id"].as_u64().unwrap();
    let top = rect(&mut s, 10.0, 30.0, 200.0, 80.0, "#3366cc");
    run(&mut s, "transparency.set", json!({"ids": [top], "opacity": 50}));
    // Apart from them: an opaque stroked square nothing overlaps.
    let alone = rect(&mut s, 220.0, 220.0, 40.0, 40.0, "#20a040");
    run(&mut s, "paint.setStroke", json!({"color": "#000000", "ids": [alone]}));
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let v = preview(&mut s, json!({"highlight": "allAffected"}));
    assert_eq!(ids(&v), [text, top], "the type and the rectangle over it");
    assert_eq!(v["counts"]["transparentObjects"], 1);
    assert_eq!(ids(&preview(&mut s, json!({"highlight": "transparentObjects"}))), [top]);
    assert_eq!(ids(&preview(&mut s, json!({"highlight": "outlinedText"}))), [text]);
    assert!(v["counts"]["vectorRegions"].as_u64().unwrap() > 1);
    assert_eq!(v["counts"]["allRasterized"], 0, "flat colours only");
    // The options decide what else changes: outlining every stroke takes in the lone square.
    let v = preview(&mut s, json!({"highlight": "outlinedStrokes", "strokesToOutlines": true}));
    assert_eq!(ids(&v), [alone]);
    assert_eq!(v["counts"]["allAffected"], 3);
    assert_eq!(v["options"]["strokesToOutlines"], true);
    // Nothing changed.
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
}

#[test]
fn an_opaque_document_reports_nothing() {
    let mut s = session();
    rect(&mut s, 10.0, 10.0, 100.0, 100.0, "#ff0000");
    rect(&mut s, 50.0, 50.0, 100.0, 100.0, "#0000ff");
    run(&mut s, "text.create", json!({"x": 20, "y": 200, "text": "Solid", "size": 30}));
    for h in Highlight::ALL {
        let v = preview(&mut s, json!({ "highlight": h.id(), "preset": "low" }));
        assert!(v["regions"].as_array().unwrap().is_empty(), "{}", h.id());
        assert!(v["counts"].as_object().unwrap().values().all(|c| c == 0), "{}: {}", h.id(), v["counts"]);
        assert_eq!(v["highlight"], h.id());
    }
}

#[test]
fn rasterized_areas_follow_the_balance_and_raster_effects() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 100.0, 100.0, "#ff0000");
    let b = rect(&mut s, 60.0, 60.0, 100.0, 100.0, "#0000ff");
    run(&mut s, "transparency.set", json!({"ids": [b], "opacity": 50}));
    // Balance 0 rasterizes the group whole: a complex region, also among all rasterized ones.
    let v = preview(&mut s, json!({"highlight": "rasterizedRegions", "balance": 0}));
    assert_eq!((v["counts"]["rasterizedRegions"].as_u64(), v["counts"]["allRasterized"].as_u64()), (Some(1), Some(1)));
    let r = &v["regions"][0]["bounds"];
    assert!(r["x"].as_f64().unwrap() <= 10.0 && r["width"].as_f64().unwrap() >= 150.0, "{r}");
    // A raster effect rasterizes where it reaches, whatever the balance.
    let e = rect(&mut s, 200.0, 200.0, 50.0, 50.0, "#20a040");
    run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "ids": [e]}));
    let v = preview(&mut s, json!({"highlight": "allRasterized", "preset": "high"}));
    assert_eq!((v["counts"]["rasterizedRegions"].as_u64(), v["counts"]["allRasterized"].as_u64()), (Some(0), Some(1)));
    assert_eq!(v["counts"]["transparentObjects"], 2);
    // Only some objects (`ids`), and the options in force.
    let v = preview(&mut s, json!({"ids": [a], "highlight": "allAffected"}));
    assert_eq!(v["counts"]["allAffected"], 0, "alone, the red square has no transparency");
    assert!(s.execute("flattener.preview", &json!({"highlight": "sparkles"})).is_err());
    assert!(s.execute("flattener.preview", &json!({"overprints": "maybe"})).is_err());
}

#[test]
fn overprints_and_patterns_are_reported() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0, "#ff0000");
    run(&mut s, "select.set", json!({"ids": [r]}));
    run(&mut s, "object.pattern.make", json!({"name": "Dots", "width": 20, "height": 20}));
    run(&mut s, "object.pattern.done", json!({}));
    let p = rect(&mut s, 100.0, 100.0, 100.0, 100.0, "#000000");
    run(&mut s, "paint.setFill", json!({"ids": [p], "swatch": "Dots"}));
    let top = rect(&mut s, 150.0, 150.0, 100.0, 100.0, "#ffcc00");
    run(&mut s, "transparency.set", json!({"ids": [top], "opacity": 60}));
    let v = preview(&mut s, json!({"highlight": "expandedPatterns"}));
    assert_eq!(ids(&v), [p]);
    // An overprinting object alone: affected once overprints are discarded.
    let op = rect(&mut s, 10.0, 220.0, 40.0, 40.0, "#00ffff");
    run(&mut s, "object.setOverprint", json!({"ids": [op], "fill": true}));
    assert_eq!(preview(&mut s, json!({}))["counts"]["allAffected"], 2);
    for o in ["discard", "simulate"] {
        let v = preview(&mut s, json!({ "overprints": o, "highlight": "allAffected" }));
        assert!(ids(&v).contains(&op), "{o}");
        assert_eq!(v["options"]["preserveOverprints"], false);
    }
}
