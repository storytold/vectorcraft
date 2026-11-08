//! Graphic Styles panel depth: Merge Graphic Styles, moving styles and Override Character Color.

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{GraphicStyle, NodeId, NodeKind};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn style(s: &Session, name: &str) -> GraphicStyle {
    s.doc().unwrap().doc.graphic_style(name).unwrap_or_else(|| panic!("no style {name}")).clone()
}

fn names(s: &Session) -> Vec<String> {
    s.doc().unwrap().doc.graphic_styles.iter().map(|g| g.name.clone()).collect()
}

/// A style named `name` made from a rectangle filled `fill`, stroked `stroke` `width` pt wide,
/// with `opacity` and, if given, one effect.
fn make(s: &mut Session, name: &str, fill: &str, stroke: &str, width: f64, opacity: f64, effect: Option<&str>) {
    let id = id_of(&run(s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})));
    run(s, "paint.setFill", json!({"color": fill, "ids": [id.0]}));
    run(s, "paint.setStroke", json!({"color": stroke, "ids": [id.0]}));
    run(s, "stroke.set", json!({"weight": width}));
    run(s, "transparency.set", json!({"opacity": opacity, "blend": if opacity < 100.0 { "multiply" } else { "normal" }}));
    if let Some(e) = effect {
        run(s, "effect.apply", json!({ "effect": e }));
    }
    run(s, "graphicStyle.new", json!({ "id": id.0, "name": name }));
}

#[test]
fn merge_combines_the_stacks_in_order() {
    let mut s = session();
    make(&mut s, "A", "#ff0000", "#000000", 2.0, 50.0, Some("distort.roughen"));
    make(&mut s, "B", "#00ff00", "#0000ff", 6.0, 100.0, Some("distort.twist"));
    let (a, b) = (style(&s, "A"), style(&s, "B"));
    let undo = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "graphicStyle.merge", json!({"names": ["A", "B"], "name": "AB"}));
    assert_eq!(r["name"], "AB");
    let m = style(&s, "AB");
    // Each style's fills and strokes on top of the ones before it, effects likewise.
    let items: Vec<_> = a.appearance.items.iter().chain(&b.appearance.items).cloned().collect();
    assert_eq!(m.appearance.items, items);
    let fx: Vec<&str> = m.appearance.effects.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(fx, ["distort.roughen", "distort.twist"]);
    // The first style's transparency, a fresh id, added last, one undo step.
    assert_eq!((m.opacity, m.blend), (0.5, BlendMode::Multiply));
    assert!(m.id != 0 && m.id != a.id && m.id != b.id);
    assert_eq!(names(&s).last().unwrap(), "AB");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    // The merged style paints both: the top fill is B's, and objects take the whole stack.
    let t = id_of(&run(&mut s, "shape.rectangle", json!({"x": 200, "y": 10, "width": 40, "height": 40})));
    run(&mut s, "graphicStyle.apply", json!({"name": "AB", "ids": [t.0]}));
    let n = s.doc().unwrap().doc.node(t).unwrap().clone();
    assert_eq!(n.appearance.fill_paint(), Paint::solid(Color::from_hex("#00ff00").unwrap()));
    assert_eq!(n.appearance.items.len(), 4);
    // Names stay unique, the default name is the next free one, and one style is not a merge.
    assert_eq!(run(&mut s, "graphicStyle.merge", json!({"names": ["B", "A"], "name": "AB"}))["name"], "AB 2");
    assert!(run(&mut s, "graphicStyle.merge", json!({"names": ["A", "B"]}))["name"].as_str().unwrap().starts_with("Graphic Style"));
    assert!(s.execute("graphicStyle.merge", &json!({"names": ["A"]})).is_err());
    assert!(s.execute("graphicStyle.merge", &json!({"names": ["A", "nope"]})).is_err());
    while s.doc().unwrap().history.undo.len() > undo {
        run(&mut s, "edit.undo", json!({}));
    }
    assert!(!names(&s).iter().any(|n| n.starts_with("AB")));
}

#[test]
fn move_reorders_and_undoes_in_one_step() {
    let mut s = session();
    let before = names(&s);
    let last = before.last().unwrap().clone();
    run(&mut s, "graphicStyle.move", json!({"name": last, "to": 0}));
    assert_eq!(names(&s)[0], last);
    assert_eq!(names(&s)[1..], before[..before.len() - 1]);
    // `to` past the end is clamped to the last place.
    let first = before[0].clone();
    run(&mut s, "graphicStyle.move", json!({"name": first, "to": 99}));
    assert_eq!(names(&s).last().unwrap(), &first);
    // Moving to where it is records nothing.
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, "graphicStyle.move", json!({"name": first, "to": before.len() - 1}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(names(&s), before);
    assert!(s.execute("graphicStyle.move", &json!({"name": "nope", "to": 0})).is_err());
    assert!(s.execute("graphicStyle.move", &json!({"name": first})).is_err());
}

/// A graphic style moved to place 2^32 goes last. A 32-bit build read the place as 0 and moved it
/// to the front.
#[test]
fn move_past_u32_goes_last() {
    let mut s = session();
    let second = names(&s)[1].clone();
    run(&mut s, "graphicStyle.move", json!({"name": second, "to": 1u64 << 32}));
    assert_eq!(names(&s).last().unwrap(), &second);
}

#[test]
fn override_character_color_clears_the_run_fill() {
    let mut s = session();
    make(&mut s, "Blue", "#0000ff", "#000000", 1.0, 100.0, None);
    let text = |s: &mut Session, y: f64| id_of(&run(s, "text.create", json!({"x": 10, "y": y, "text": "Hi", "size": 40})));
    let runs = |s: &Session, id: NodeId| match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => t.runs.iter().map(|r| (r.style.fill.clone(), r.style.stroke.clone())).collect::<Vec<_>>(),
        _ => panic!("type"),
    };
    // On by default: the characters' own colour gives way to the style's fill and stroke.
    assert_eq!(run(&mut s, "graphicStyle.setOptions", json!({}))["overrideCharColor"], true);
    let a = text(&mut s, 100.0);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000", "ids": [a.0]}));
    assert!(runs(&s, a).iter().all(|(f, _)| *f != Paint::None));
    run(&mut s, "graphicStyle.apply", json!({"name": "Blue", "ids": [a.0]}));
    assert!(runs(&s, a).iter().all(|(f, st)| *f == Paint::None && *st == Paint::None));
    assert_eq!(s.doc().unwrap().doc.node(a).unwrap().appearance.fill_paint(), Paint::solid(Color::from_hex("#0000ff").unwrap()));
    // Off: they keep it (the style's paint draws over them).
    assert_eq!(run(&mut s, "graphicStyle.setOptions", json!({"overrideCharColor": false}))["overrideCharColor"], false);
    assert_eq!(run(&mut s, "prefs.get", json!({"key": "overrideCharColor"})), false);
    assert_eq!(run(&mut s, "graphicStyle.list", json!({}))["overrideCharColor"], false);
    let b = text(&mut s, 200.0);
    let kept = runs(&s, b);
    run(&mut s, "graphicStyle.apply", json!({"name": "Blue", "ids": [b.0]}));
    assert_eq!(runs(&s, b), kept);
    // The preference drives it too.
    run(&mut s, "prefs.set", json!({"key": "overrideCharColor", "value": true}));
    assert!(s.prefs.override_char_color);
}

#[test]
fn override_character_color_round_trips_and_defaults_on() {
    let p = Prefs { override_char_color: false, ..Prefs::default() };
    let back: Prefs = serde_json::from_value(p.to_json()).unwrap();
    assert!(!back.override_char_color);
    // Preferences saved before the option had it on.
    let mut old = Prefs::default().to_json();
    old.as_object_mut().unwrap().remove("overrideCharColor");
    assert!(serde_json::from_value::<Prefs>(old).unwrap().override_char_color);
}
