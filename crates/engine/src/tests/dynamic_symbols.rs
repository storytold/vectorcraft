//! Dynamic symbols (#4938): an instance changes the appearance of objects in its symbol's art and
//! stays linked; editing the symbol keeps those changes, Reset and Break Link handle them.

use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::Rect;

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s
}

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A blue square at `x` (40 pt, no stroke).
fn square(s: &mut Session, x: f64) -> NodeId {
    let r = id(&s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 40, "height": 40})).unwrap());
    s.execute("paint.setFill", &json!({"ids": [r.0], "color": "#0000ff"})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [r.0], "none": true})).unwrap();
    r
}

/// Symbol "Pair" of two blue squares (x 10..50 and 60..100) of `kind`, made in place: its instance.
fn pair(s: &mut Session, kind: &str) -> NodeId {
    let (a, b) = (square(s, 10.0), square(s, 60.0));
    id(&s.execute("symbol.new", &json!({"name": "Pair", "ids": [a.0, b.0], "type": kind})).unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn overrides(s: &Session, id: NodeId) -> usize {
    node(s, id).extras().symbol_overrides.len()
}

fn pixel(s: &Session, x: u32, y: u32) -> [u8; 4] {
    vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, Rect::new(0.0, 0.0, 200.0, 100.0), 1.0, true).pixel(x, y)
}

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

#[test]
fn a_dynamic_instance_paints_one_object_its_own_way_and_stays_linked() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    assert_eq!(s.execute("symbol.list", &json!({})).unwrap()["symbols"][0]["type"], "dynamic");
    let other = id(&s.execute("symbol.place", &json!({"name": "Pair", "x": 55, "y": 80})).unwrap());
    let r = s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [1], "fill": "#ff0000"})).unwrap();
    assert_eq!(r["overrides"], 1);
    assert!(matches!(node(&s, inst).kind, NodeKind::SymbolInstance { .. }), "still an instance");
    assert_eq!(pixel(&s, 80, 30), RED, "the changed object");
    assert_eq!(pixel(&s, 30, 30), BLUE, "its sibling keeps the symbol's look");
    assert_eq!(overrides(&s, other), 0, "other instances keep the symbol's look");
    // A later change to the same object merges with the first; null takes one back.
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [1], "opacity": 50})).unwrap();
    assert_eq!(node(&s, inst).extras().symbol_overrides[0].opacity, Some(0.5));
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [1], "opacity": null})).unwrap();
    let o = node(&s, inst).extras().symbol_overrides[0].clone();
    assert!(o.opacity.is_none() && o.fill.is_some());
    // SVG export draws it too.
    let svg = s.execute("document.export", &json!({"format": "svg"})).unwrap();
    let svg = String::from_utf8(vectorcraft_format::base64_decode(svg["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    assert!(svg.to_ascii_lowercase().contains("#ff0000") || svg.contains("rgb(255,0,0)"), "{svg}");
}

#[test]
fn static_symbols_and_bad_targets_are_refused() {
    let mut s = session();
    let inst = pair(&mut s, "static");
    let set = |s: &mut Session, p: Value| s.execute("symbol.instance.setAppearance", &p);
    assert!(set(&mut s, json!({"id": inst.0, "path": [0], "fill": "#ff0000"})).is_err(), "a static symbol");
    s.execute("symbol.options", &json!({"name": "Pair", "type": "dynamic"})).unwrap();
    for bad in [
        json!({"id": inst.0, "path": [2], "fill": "#ff0000"}),
        json!({"id": inst.0, "path": [0, 0], "fill": "#ff0000"}),
        json!({"id": inst.0, "path": [-1], "fill": "#ff0000"}),
        json!({"id": inst.0, "path": "0", "fill": "#ff0000"}),
        json!({"id": inst.0, "fill": "#ff0000"}),
        json!({"id": inst.0, "path": [0]}),
        json!({"id": inst.0, "path": [0], "blend": "sparkle"}),
        json!({"id": inst.0, "path": [0], "opacity": "half"}),
        json!({"id": inst.0, "path": [0], "fill": {}}),
    ] {
        assert!(set(&mut s, bad.clone()).is_err(), "{bad}");
    }
    let plain = square(&mut s, 120.0);
    assert!(set(&mut s, json!({"id": plain.0, "path": [], "fill": "#ff0000"})).is_err(), "not an instance");
    assert!(s.execute("symbol.options", &json!({"name": "Pair", "type": "sometimes"})).is_err());
    assert_eq!(overrides(&s, inst), 0);
}

#[test]
fn redefining_the_symbol_keeps_the_changes_whose_objects_remain() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [1], "fill": "#ff0000"})).unwrap();
    // New geometry with the same structure: two squares, now 10..40 and 50..80 tall.
    let (a, b) = (square(&mut s, 10.0), square(&mut s, 120.0));
    s.execute("symbol.update", &json!({"name": "Pair", "ids": [a.0, b.0]})).unwrap();
    assert_eq!(overrides(&s, inst), 1, "the change follows the edited symbol");
    // Art of one object (no children) leaves path [1] behind.
    let c = square(&mut s, 10.0);
    s.execute("symbol.update", &json!({"name": "Pair", "ids": [c.0]})).unwrap();
    assert_eq!(overrides(&s, inst), 0);
}

#[test]
fn reset_and_static_drop_the_changes_and_undo_brings_them_back() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [0], "fill": "#ff0000", "blend": "multiply"})).unwrap();
    let r = s.execute("symbol.instance.reset", &json!({"ids": [inst.0]})).unwrap();
    assert_eq!(r["ids"], json!([inst.0]));
    assert_eq!(overrides(&s, inst), 0);
    assert_eq!(pixel(&s, 30, 30), BLUE);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(overrides(&s, inst), 1);
    // Nothing to reset: no undo step.
    let other = id(&s.execute("symbol.place", &json!({"name": "Pair"})).unwrap());
    let undo = s.doc().unwrap().history.undo.len();
    assert_eq!(s.execute("symbol.instance.reset", &json!({"ids": [other.0]})).unwrap()["ids"], json!([]));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    // Static again: the instances show the symbol's appearance.
    s.execute("symbol.options", &json!({"name": "Pair", "type": "static"})).unwrap();
    assert_eq!(overrides(&s, inst), 0);
}

#[test]
fn break_link_bakes_the_changes_and_edit_symbol_shows_the_symbols_own_art() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [1], "fill": "#ff0000"})).unwrap();
    let other = id(&s.execute("symbol.place", &json!({"name": "Pair", "x": 55, "y": 80})).unwrap());
    s.execute("symbol.instance.setAppearance", &json!({"id": other.0, "path": [1], "fill": "#ff0000"})).unwrap();
    let fills = |s: &Session, art: NodeId| -> Vec<String> {
        node(s, art).children().unwrap().iter().map(|c| c.appearance.fill_paint().color().unwrap().to_hex()).collect()
    };
    let art = NodeId(s.execute("symbol.breakLink", &json!({"ids": [inst.0]})).unwrap()["ids"][0].as_u64().unwrap());
    assert_eq!(fills(&s, art), ["#0000ff", "#ff0000"]);
    let art = NodeId(s.execute("symbol.edit", &json!({"ids": [other.0]})).unwrap()["ids"][0].as_u64().unwrap());
    assert_eq!(fills(&s, art), ["#0000ff", "#0000ff"], "the symbol's art, without this instance's changes");
}

#[test]
fn dynamic_symbols_and_their_changes_round_trip_and_old_documents_save_as_before() {
    let mut s = session();
    pair(&mut s, "static");
    let json = serde_json::to_string(&s.doc().unwrap().doc).unwrap();
    assert!(!json.contains("\"dynamic\"") && !json.contains("symbol_overrides"), "nothing new is written for static symbols");
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [1], "fill": "#ff0000", "opacity": 25})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(doc, false)).unwrap();
    assert!(back.symbols[0].dynamic);
    assert_eq!(back.node(inst).unwrap().extras().symbol_overrides, doc.node(inst).unwrap().extras().symbol_overrides);
    assert_eq!(back.symbols, doc.symbols);
}
