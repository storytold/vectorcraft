//! The Type tools give new type the font, style and size last set on type (#1655); `text.create`
//! itself keeps its defaults, so scripts and agents make the same type whatever was styled before.

use serde_json::{Value, json};
use vectorcraft_doc::{CharStyle, NodeId, NodeKind};
use vectorcraft_tools::{PointerEvent, PointerKind};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.prefs.placeholder_text = false;
    // The test families aren't installed: keep their names on the characters.
    s.prefs.missing_glyph_protection = false;
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn create(s: &mut Session, params: Value) -> NodeId {
    NodeId(s.execute("text.create", &params).unwrap()["id"].as_u64().unwrap())
}

fn first_style(s: &Session, id: NodeId) -> CharStyle {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => t.first_style(),
        _ => panic!("not type"),
    }
}

fn face(st: &CharStyle) -> (&str, &str, f64) {
    (st.font_family.as_str(), st.font_style.as_str(), st.size)
}

/// Click with the Type tool at `(x, y)`, type `text` and end the edit: the new type.
fn type_at(s: &mut Session, x: f64, y: f64, text: &str) -> NodeId {
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, x, y), v).unwrap();
    }
    s.tool_text(text, v).unwrap();
    let id = s.doc().unwrap().selection.objects[0];
    s.select_tool("selection", v).unwrap();
    id
}

#[test]
fn the_type_tool_starts_with_the_default_until_a_font_is_set() {
    let mut s = session();
    let id = type_at(&mut s, 100.0, 100.0, "One");
    assert_eq!(face(&first_style(&s, id)), face(&CharStyle::default()));
}

#[test]
fn the_type_tool_takes_the_font_style_and_size_last_set() {
    let mut s = session();
    let a = type_at(&mut s, 100.0, 100.0, "One");
    s.execute("text.setStyle", &json!({"ids": [a.0], "font": "Test Serif", "style": "Bold"})).unwrap();
    s.execute("text.setStyle", &json!({"ids": [a.0], "size": 30})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let b = type_at(&mut s, 100.0, 300.0, "Two");
    assert_eq!(face(&first_style(&s, b)), ("Test Serif", "Bold", 30.0));
    // The journal has them, so a replay makes the same type.
    let (_, p) = s.journal.since(0).iter().rev().find(|(c, _)| c == "text.create").unwrap();
    assert_eq!((&p["font"], &p["style"], &p["size"]), (&json!("Test Serif"), &json!("Bold"), &json!(30.0)));
}

#[test]
fn a_range_styled_while_typing_counts_too() {
    let mut s = session();
    let a = create(&mut s, json!({"x": 10, "y": 40, "text": "Hello"}));
    s.execute("text.setRangeStyle", &json!({"id": a.0, "start": 1, "end": 3, "font": "Test Mono", "style": "Italic", "size": 18})).unwrap();
    let b = type_at(&mut s, 100.0, 300.0, "Two");
    assert_eq!(face(&first_style(&s, b)), ("Test Mono", "Italic", 18.0));
}

#[test]
fn a_font_previewed_and_cancelled_is_not_remembered() {
    let mut s = session();
    let a = create(&mut s, json!({"x": 10, "y": 40, "text": "One"}));
    s.begin_interaction("Font").unwrap();
    s.preview("text.setStyle", &json!({"ids": [a.0], "font": "Test Serif"})).unwrap();
    s.cancel_interaction().unwrap();
    let b = type_at(&mut s, 100.0, 300.0, "Two");
    assert_eq!(first_style(&s, b).font_family, CharStyle::default().font_family);
}

#[test]
fn area_type_takes_it_and_text_create_keeps_its_defaults() {
    let mut s = session();
    let a = create(&mut s, json!({"x": 10, "y": 40, "text": "One"}));
    s.execute("text.setStyle", &json!({"ids": [a.0], "font": "Test Serif", "size": 24})).unwrap();
    // Area Type on a rectangle.
    s.execute("shape.rectangle", &json!({"x": 400, "y": 400, "width": 100, "height": 80})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("areaType", v).unwrap();
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, 400.0, 440.0), v).unwrap();
    }
    s.tool_text("In", v).unwrap();
    let area = s.doc().unwrap().selection.objects[0];
    assert_eq!((first_style(&s, area).font_family.as_str(), first_style(&s, area).size), ("Test Serif", 24.0));
    s.select_tool("selection", v).unwrap();
    // A command names what it wants: without a font it gets the default one.
    let c = create(&mut s, json!({"x": 10, "y": 200, "text": "Script"}));
    assert_eq!(face(&first_style(&s, c)), face(&CharStyle::default()));
}
