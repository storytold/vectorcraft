//! Stroke panel reach: `stroke.set` on groups, what the panel shows for a selection (mixed values,
//! Align Stroke), Units > Stroke, and the stroke options `document.inspect` reports.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind, Unit};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 10, "width": 40, "height": 40}))["id"].as_u64().unwrap())
}

fn select(s: &mut Session, ids: &[NodeId]) {
    run(s, "select.set", json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }));
}

#[test]
fn stroke_set_on_a_group_skips_images_and_symbol_instances() {
    let mut s = session();
    let (a, b, c) = (rect(&mut s, 10.0), rect(&mut s, 60.0), rect(&mut s, 110.0));
    select(&mut s, &[b]);
    let image = NodeId(run(&mut s, "object.rasterize", json!({}))["id"].as_u64().unwrap());
    select(&mut s, &[c]);
    let instance = NodeId(run(&mut s, "symbol.new", json!({"name": "Box"}))["id"].as_u64().unwrap());
    select(&mut s, &[a, image, instance]);
    run(&mut s, "object.group", json!({}));
    run(&mut s, "stroke.set", json!({"weight": 5}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.node(a).unwrap().appearance.stroke_width(), 5.0);
    for id in [image, instance] {
        let n = d.node(id).unwrap();
        assert!(matches!(n.kind, NodeKind::Image(_) | NodeKind::SymbolInstance { .. }));
        assert!(n.appearance.stroke().is_none(), "{:?} gains no stroke", n.kind_label());
    }
}

#[test]
fn mixed_values_and_align_stroke_follow_the_selection() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 10.0), rect(&mut s, 60.0));
    run(&mut s, "stroke.set", json!({"ids": [b.0], "weight": 3, "join": "round"}));
    select(&mut s, &[a, b]);
    let m = s.active().unwrap().stroke_mixed();
    assert!(m.weight && m.join && !m.cap && !m.miter_limit && !m.align && !m.dash && m.can_align, "{m:?}");
    // An open path in the selection can't take an inside or outside stroke; type can (its
    // characters' stroke goes around the glyphs).
    let line = NodeId(run(&mut s, "shape.line", json!({"x1": 10, "y1": 100, "x2": 90, "y2": 100}))["id"].as_u64().unwrap());
    select(&mut s, &[a, line]);
    assert!(!s.active().unwrap().stroke_mixed().can_align);
    let text = NodeId(run(&mut s, "text.create", json!({"x": 10, "y": 200, "text": "Hi"}))["id"].as_u64().unwrap());
    select(&mut s, &[text]);
    assert!(s.active().unwrap().stroke_mixed().can_align);
    select(&mut s, &[text, line]);
    assert!(!s.active().unwrap().stroke_mixed().can_align);
    // Grouped closed paths can.
    select(&mut s, &[a, b]);
    run(&mut s, "object.group", json!({}));
    let m = s.active().unwrap().stroke_mixed();
    assert!(m.can_align && m.weight);
    assert_eq!(s.shown_stroke().map(|st| st.width), Some(1.0), "a group shows its first painted object's stroke");
}

#[test]
fn stroke_unit_follows_the_preference() {
    let mut s = session();
    assert_eq!(s.stroke_unit(), Unit::Points);
    s.prefs.units_stroke = "millimeters".into();
    assert_eq!(s.stroke_unit(), Unit::Millimeters);
    assert_eq!(s.stroke_unit().format(1.0), "0.3528 mm");
}

#[test]
fn inspect_reports_stroke_options() {
    let mut s = session();
    let line = run(&mut s, "shape.line", json!({"x1": 10, "y1": 100, "x2": 90, "y2": 100}))["id"].as_u64().unwrap();
    run(
        &mut s,
        "stroke.set",
        json!({"ids": [line], "cap": "round", "join": "bevel", "miterLimit": 4, "dash": [6, 3], "alignDashes": true, "endArrow": "Triangle", "arrowAlign": "tip", "profile": "lens"}),
    );
    let doc = run(&mut s, "document.inspect", json!({}));
    let o = &doc["layers"][0]["children"][0]["strokeOptions"];
    assert_eq!(o["cap"], "round");
    assert_eq!(o["join"], "bevel");
    assert_eq!(o["miterLimit"], 4.0);
    assert_eq!(o["align"], "center");
    assert_eq!(o["dash"], json!([6.0, 3.0]));
    assert_eq!(o["alignDashes"], true);
    assert_eq!((o["startArrow"].clone(), o["endArrow"].clone(), o["arrowAlign"].clone()), (Value::Null, json!("Triangle"), json!("tip")));
    assert_eq!(o["profile"], "lens");
    // The options read back into stroke.set unchanged.
    let mut q = o.as_object().unwrap().clone();
    q.insert("ids".into(), json!([line]));
    let before = s.doc().unwrap().doc.node(NodeId(line)).unwrap().appearance.clone();
    run(&mut s, "stroke.set", Value::Object(q));
    assert_eq!(s.doc().unwrap().doc.node(NodeId(line)).unwrap().appearance, before);
    // Type reports its characters' stroke.
    run(&mut s, "text.create", json!({"x": 10, "y": 200, "text": "Hi"}));
    run(&mut s, "stroke.set", json!({"join": "round"}));
    let doc = run(&mut s, "document.inspect", json!({}));
    let text = doc["layers"][0]["children"].as_array().unwrap().iter().find(|c| c["kind"] == "Text" || c.get("text").is_some()).unwrap().clone();
    assert_eq!(text["strokeOptions"]["join"], "round", "{text}");
}
