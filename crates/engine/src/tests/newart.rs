//! The new-art template (M3.65): Stroke panel options and graphic styles chosen with nothing
//! selected, New Art Has Basic Appearance and `paint.default`.

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::{Appearance, Dash, LineCap, Node};

use crate::*;

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
fn invalid_new_art_basic_parameters_keep_the_preference_unchanged() {
    let mut s = session();
    let undo = undo_len(&s);
    for params in [json!({"on": "false"}), json!({"on": 1})] {
        assert!(s.execute("appearance.setNewArtBasic", &params).is_err(), "accepted {params}");
        assert!(s.prefs.new_art_basic, "changed the preference for {params}");
        assert_eq!(undo_len(&s), undo, "recorded undo for {params}");
    }
    // Left out, or null as for every command, it toggles.
    assert_eq!(s.execute("appearance.setNewArtBasic", &json!(null)).unwrap(), json!({"on": false}));
    assert_eq!(s.execute("appearance.setNewArtBasic", &json!({"on": null})).unwrap(), json!({"on": true}));
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

/// A rectangle with this fill, stroke and weight, selected.
fn painted(s: &mut Session, x: f64, fill: &str, stroke: &str, weight: f64) -> NodeId {
    let id = rect(s, x);
    run(s, "paint.setFill", json!({"color": fill}));
    run(s, "paint.setStroke", json!({"color": stroke}));
    run(s, "stroke.set", json!({"weight": weight}));
    id
}

fn select(s: &mut Session, ids: &[NodeId]) {
    run(s, "select.set", json!({"ids": ids.iter().map(|id| id.0).collect::<Vec<_>>()}));
}

#[test]
fn selecting_an_object_gives_new_art_its_fill_stroke_and_weight_with_basic_appearance_on() {
    let mut s = session();
    assert!(s.prefs.new_art_basic, "on by default");
    let green = painted(&mut s, 20.0, "#ff0000", "#00ff00", 5.0);
    painted(&mut s, 120.0, "#0000ff", "#000000", 2.0);
    select(&mut s, &[green]);
    run(&mut s, "select.none", json!({}));
    let id = rect(&mut s, 220.0);
    let (n, from) = (node(&s, id).appearance, node(&s, green).appearance);
    assert_eq!((n.fill_paint(), n.stroke_paint(), n.stroke_width()), (from.fill_paint(), from.stroke_paint(), 5.0));
}

#[test]
fn with_basic_appearance_on_only_the_basic_fill_and_stroke_follow_the_selection() {
    let mut s = session();
    let src = fancy(&mut s);
    painted(&mut s, 120.0, "#00ff00", "#000000", 2.0);
    select(&mut s, &[src]);
    run(&mut s, "select.none", json!({}));
    let id = rect(&mut s, 220.0);
    let n = node(&s, id);
    assert_eq!(n.appearance.fill_paint(), node(&s, src).appearance.fill_paint(), "the top fill");
    assert_eq!((n.appearance.items.len(), n.appearance.effects.len(), n.opacity), (2, 0, 1.0), "one fill and stroke, nothing else");
}

#[test]
fn a_paint_the_selected_objects_do_not_share_stays_as_it_was() {
    let mut s = session();
    let a = painted(&mut s, 20.0, "#ff0000", "#00ff00", 5.0);
    let b = painted(&mut s, 120.0, "#0000ff", "#00ff00", 5.0);
    painted(&mut s, 220.0, "#ffff00", "#000000", 1.0);
    let before = s.paint.fill.clone();
    select(&mut s, &[a, b]);
    assert_eq!(s.paint.fill, before, "the fills differ");
    assert_eq!((s.paint.stroke.clone(), s.paint.stroke_width), (node(&s, a).appearance.stroke_paint(), 5.0), "the strokes are shared");
}

#[test]
fn art_without_paint_leaves_the_proxies_as_they_were() {
    let mut s = session();
    painted(&mut s, 20.0, "#ff0000", "#00ff00", 5.0);
    let (fill, stroke) = (s.paint.fill.clone(), s.paint.stroke.clone());
    let dir = super::links::Folder::new("new-art-image");
    let path = dir.file("pic.png");
    super::links::write(&path, &super::links::png(4, 4, super::links::RED));
    let image = super::links::place(&mut s, &path);
    run(&mut s, "select.none", json!({}));
    select(&mut s, &[image]);
    assert_eq!((s.paint.fill.clone(), s.paint.stroke.clone(), s.paint.stroke_width), (fill, stroke, 5.0), "new art stays visible");
}

#[test]
fn an_object_without_a_stroke_clears_the_stroke_and_keeps_the_weight() {
    let mut s = session();
    let bare = rect(&mut s, 20.0);
    run(&mut s, "paint.setStroke", json!({"none": true}));
    painted(&mut s, 120.0, "#0000ff", "#000000", 7.0);
    select(&mut s, &[bare]);
    assert_eq!((s.paint.stroke.clone(), s.paint.stroke_width), (Paint::None, 7.0));
}

#[test]
fn a_selected_gradient_loads_without_its_place_on_the_object() {
    let mut s = session();
    let src = rect(&mut s, 20.0);
    let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]);
    run(&mut s, "paint.setFill", json!({"gradient": {"stops": stops, "start": [20, 40], "end": [50, 40]}}));
    painted(&mut s, 120.0, "#0000ff", "#000000", 2.0);
    select(&mut s, &[src]);
    assert!(matches!(s.paint.fill, Paint::Gradient(_)), "{:?}", s.paint.fill);
    run(&mut s, "select.none", json!({}));
    let id = rect(&mut s, 220.0);
    assert!(matches!(node(&s, id).appearance.fill_paint(), Paint::Gradient(_)));
}

#[test]
fn selecting_leaves_the_stroke_panels_options_for_new_art_alone() {
    let mut s = session();
    let plain = painted(&mut s, 20.0, "#ff0000", "#00ff00", 5.0);
    run(&mut s, "select.none", json!({}));
    run(&mut s, "stroke.set", json!({"cap": "round", "dash": [4, 2]}));
    select(&mut s, &[plain]);
    run(&mut s, "select.none", json!({}));
    let id = rect(&mut s, 120.0);
    let st = node(&s, id).appearance.stroke().unwrap().clone();
    assert_eq!((st.paint.clone(), st.width), (node(&s, plain).appearance.stroke_paint(), 5.0));
    assert_eq!((st.cap, st.dash.map(|d| d.pattern)), (LineCap::Round, Some(vec![4.0, 2.0])));
}

#[test]
fn a_sampled_paint_stays_until_the_selection_changes() {
    let mut s = session();
    let a = painted(&mut s, 20.0, "#ff0000", "#00ff00", 5.0);
    let sampled = Paint::solid(vectorcraft_color::Color::WHITE);
    s.paint.fill = sampled.clone();
    run(&mut s, "transparency.set", json!({"opacity": 50}));
    assert_eq!(s.paint.fill, sampled, "not a selection command");
    select(&mut s, &[a]);
    assert_eq!(s.paint.fill, sampled, "the same selection again");
    run(&mut s, "select.none", json!({}));
    assert_eq!(s.paint.fill, sampled, "nothing selected shows nothing to load");
    select(&mut s, &[a]);
    assert_eq!(s.paint.fill, node(&s, a).appearance.fill_paint(), "selected anew");
}

#[test]
fn targeting_a_layer_loads_the_paints_of_its_contents() {
    let mut s = session();
    let a = painted(&mut s, 20.0, "#ff0000", "#00ff00", 5.0);
    let layer = s.doc().unwrap().doc.layers[0].id;
    run(&mut s, "select.none", json!({}));
    run(&mut s, "paint.setFill", json!({"color": "#0000ff"}));
    run(&mut s, "layer.target", json!({"id": layer.0}));
    assert_eq!(s.paint.fill, node(&s, a).appearance.fill_paint());
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
