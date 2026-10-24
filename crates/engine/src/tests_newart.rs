//! The new-art template (M3.65): Stroke panel options and graphic styles chosen with nothing
//! selected, New Art Has Basic Appearance and `paint.default`.

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::{Appearance, Dash, LineCap, Node};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 20, "width": 60, "height": 40}))["id"].as_u64().unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn stroke_options_set_with_nothing_selected_carry_to_the_next_object() {
    let mut s = session();
    let before = rect(&mut s, 20.0);
    run(&mut s, "select.none", json!({}));
    let undo = undo_len(&s);
    run(&mut s, "stroke.set", json!({"cap": "round", "dash": [4, 2], "weight": 3, "endArrow": "Arrow"}));
    assert_eq!(undo_len(&s), undo, "no document change, no undo step");
    // The Stroke panel shows them with nothing selected.
    let shown = s.shown_stroke().unwrap();
    assert_eq!((shown.cap, shown.width, shown.dash.as_ref().map(|d| d.pattern.clone())), (LineCap::Round, 3.0, Some(vec![4.0, 2.0])));
    let id = rect(&mut s, 120.0);
    let st = node(&s, id).appearance.stroke().unwrap().clone();
    assert_eq!((st.cap, st.width, st.end_arrow.is_some()), (LineCap::Round, 3.0, true));
    assert_eq!(st.dash, Some(Dash { pattern: vec![4.0, 2.0], ..Default::default() }));
    assert_eq!(node(&s, id).appearance.fill_paint(), Paint::solid(vectorcraft_color::Color::WHITE), "the fill stays the proxy's");
    assert_eq!(node(&s, before).appearance.stroke().unwrap().cap, LineCap::Butt, "art drawn before keeps its stroke");
    // Editing a selected object's stroke leaves the template alone.
    run(&mut s, "stroke.set", json!({"cap": "square"}));
    run(&mut s, "select.none", json!({}));
    assert_eq!(s.shown_stroke().unwrap().cap, LineCap::Round);
    // Other new art takes it too (a line still without a fill).
    let line = NodeId(run(&mut s, "shape.line", json!({"x1": 10, "y1": 200, "x2": 200, "y2": 200}))["id"].as_u64().unwrap());
    let l = node(&s, line).appearance;
    assert_eq!((l.fill_paint(), l.stroke().unwrap().cap), (Paint::None, LineCap::Round));
    // Default Fill and Stroke resets the template.
    run(&mut s, "paint.default", json!({}));
    let plain = rect(&mut s, 200.0);
    assert_eq!(node(&s, plain).appearance, Appearance::default_art());
    assert!(s.shown_stroke().is_some_and(|st| st.dash.is_none() && st.cap == LineCap::Butt));
}

/// A rectangle with two fills, a drop shadow and 50% opacity, selected.
fn fancy(s: &mut Session) -> NodeId {
    let id = rect(s, 20.0);
    run(s, "paint.setFill", json!({"color": "#ff0000"}));
    run(s, "appearance.addFill", json!({}));
    run(s, "paint.setFill", json!({"color": "#0000ff"}));
    run(s, "effect.apply", json!({"effect": "stylize.dropShadow"}));
    run(s, "transparency.set", json!({"opacity": 50}));
    id
}

#[test]
fn with_basic_appearance_off_new_art_takes_the_last_selections_whole_appearance() {
    let mut s = session();
    assert!(s.prefs.new_art_basic, "on by default");
    let src = fancy(&mut s);
    run(&mut s, "select.none", json!({}));
    let basic = rect(&mut s, 120.0);
    let b = node(&s, basic);
    assert_eq!((b.appearance.items.len(), b.appearance.effects.len(), b.opacity), (2, 0, 1.0), "on: one fill and stroke");
    assert_eq!(run(&mut s, "appearance.setNewArtBasic", json!({})), json!({"on": false}), "no param toggles");
    run(&mut s, "select.set", json!({"ids": [src.0]}));
    run(&mut s, "select.none", json!({}));
    let id = rect(&mut s, 220.0);
    let (n, from) = (node(&s, id), node(&s, src));
    assert_eq!(n.appearance.items.iter().filter(|i| i.is_fill()).count(), 2, "both fills");
    assert_eq!(n.appearance.effects.len(), 1, "the drop shadow");
    assert!(n.appearance.approx_eq(&from.appearance));
    assert_eq!((n.opacity, n.blend), (0.5, BlendMode::Normal));
    let q = run(&mut s, "appearance.newArt", json!({}));
    assert_eq!((q["basic"].clone(), q["inherited"].clone(), q["opacity"].clone()), (json!(false), json!(true), json!(50.0)));
    // Back on, new art takes only the basic fill and stroke again.
    run(&mut s, "appearance.setNewArtBasic", json!({"on": true}));
    run(&mut s, "select.none", json!({}));
    let again = rect(&mut s, 300.0);
    let again = node(&s, again);
    assert_eq!((again.appearance.items.len(), again.appearance.effects.len(), again.opacity), (2, 0, 1.0));
}

#[test]
fn a_graphic_style_clicked_with_nothing_selected_styles_the_next_object() {
    let mut s = session();
    fancy(&mut s);
    // A placed gradient on the top fill: the style keeps it relative to the object's bounds.
    let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]);
    run(&mut s, "paint.setFill", json!({"gradient": {"stops": stops, "start": [20, 40], "end": [50, 40]}}));
    let name = run(&mut s, "graphicStyle.new", json!({"name": "Fancy"}))["name"].as_str().unwrap().to_string();
    run(&mut s, "select.none", json!({}));
    let undo = undo_len(&s);
    assert_eq!(run(&mut s, "graphicStyle.apply", json!({"name": name})), json!({"newArt": true}));
    assert_eq!(undo_len(&s), undo);
    assert_eq!(run(&mut s, "appearance.newArt", json!({}))["graphicStyle"], json!("Fancy"));
    let id = NodeId(run(&mut s, "shape.ellipse", json!({"x": 200, "y": 100, "width": 100, "height": 50}))["id"].as_u64().unwrap());
    let n = node(&s, id);
    assert_eq!((n.appearance.items.len(), n.appearance.effects.len(), n.opacity), (3, 1, 0.5));
    // Linked to the style (its look, gradient placed on the ellipse's own bounds).
    let (g, linked) = s.selection_graphic_style().unwrap();
    assert_eq!((g.name.as_str(), linked), ("Fancy", true));
    // Alt-click (add) with nothing selected stacks a style on the template, unlinked.
    run(&mut s, "select.none", json!({}));
    run(&mut s, "graphicStyle.apply", json!({"name": name, "add": true}));
    let q = run(&mut s, "appearance.newArt", json!({}));
    assert_eq!((q["appearance"]["items"].as_array().unwrap().len(), q["graphicStyle"].clone()), (6, Value::Null));
    assert!(s.execute("graphicStyle.apply", &json!({"name": name, "ids": []})).is_err(), "explicit ids that match nothing");
}

#[test]
fn the_option_round_trips_with_the_preferences() {
    let mut s = session();
    run(&mut s, "prefs.set", json!({"key": "newArtBasic", "value": false}));
    assert!(!s.prefs.new_art_basic);
    assert_eq!(run(&mut s, "prefs.get", json!({"key": "newArtBasic"})), json!(false));
    let json = s.prefs.to_json();
    assert_eq!(json["newArtBasic"], json!(false));
    let back: Prefs = serde_json::from_value(json).unwrap();
    assert_eq!(back, s.prefs);
    // Older preference files load with it on.
    let old: Prefs = serde_json::from_value(json!({"keyboardIncrement": 2})).unwrap();
    assert!(old.new_art_basic);
}

/// Line Segment Tool Options' Fill Line: `shape.line {fill: true}` gives the line the current
/// fill; without it a new line has none.
#[test]
fn fill_line_gives_a_new_line_the_current_fill() {
    let mut s = session();
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    let plain = NodeId(run(&mut s, "shape.line", json!({"x1": 10, "y1": 20, "x2": 200, "y2": 80}))["id"].as_u64().unwrap());
    let filled = NodeId(run(&mut s, "shape.line", json!({"x1": 10, "y1": 20, "x2": 200, "y2": 80, "fill": true}))["id"].as_u64().unwrap());
    assert_eq!(node(&s, plain).appearance.fill_paint(), Paint::None);
    assert_eq!(node(&s, filled).appearance.fill_paint(), s.paint.fill);
    assert_ne!(s.paint.fill, Paint::None);
    assert!(s.execute("shape.line", &json!({"x1": 0, "y1": 0, "x2": 1, "y2": 1, "fill": "yes"})).is_err());
}
