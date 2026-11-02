//! Pathfinder panel › compound shapes (`object.compoundShape.*`): Make Compound Shape in each
//! shape mode, a member's mode, Release, Expand, Undo, saving and exporting.

use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::{Node, ShapeMode};
use vectorcraft_render::effects;

use crate::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

fn area(path: &vectorcraft_geom::PathData) -> f64 {
    vectorcraft_pathops::area(path, vectorcraft_geom::FillRule::NonZero).abs()
}

/// The area a path or compound path fills.
fn node_area(n: &Node) -> f64 {
    area(&crate::cmd::pathops::node_path(n).unwrap().0)
}

fn square(s: &mut Session, x: f64, color: &str) -> NodeId {
    let id = NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 0, "width": 100, "height": 100}))["id"].as_u64().unwrap());
    run(s, "paint.setFill", json!({"color": color}));
    id
}

/// A red square (0–100) behind a blue one (50–150), both 100 high and selected → (session, back, front).
fn two_squares() -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    let back = square(&mut s, 0.0, "#ff0000");
    let front = square(&mut s, 50.0, "#0000ff");
    run(&mut s, "select.set", json!({"ids": [back.0, front.0]}));
    (s, back, front)
}

fn selected(s: &Session) -> Vec<NodeId> {
    s.doc().unwrap().selection.objects.clone()
}

/// Make Compound Shape (Alt-clicking a Shape Mode gives its mode) keeps both squares as members in
/// the mode, takes the paint of the topmost member not subtracting, draws the area the modes make,
/// and Expand turns it into that area as one path; Undo brings each step back.
#[test]
fn make_and_expand_each_shape_mode() {
    for (mode, want, fill, kind) in [
        ("add", 15_000.0, Color::rgb(0.0, 0.0, 1.0), "Path"),
        ("subtract", 5_000.0, Color::rgb(1.0, 0.0, 0.0), "Path"),
        ("intersect", 5_000.0, Color::rgb(0.0, 0.0, 1.0), "Path"),
        ("exclude", 10_000.0, Color::rgb(0.0, 0.0, 1.0), "Compound Path"),
    ] {
        let (mut s, back, front) = two_squares();
        let before = s.doc().unwrap().doc.clone();
        let id = NodeId(run(&mut s, "object.compoundShape.make", json!({"mode": mode}))["id"].as_u64().unwrap());
        assert_eq!(selected(&s), vec![id]);
        let shape = node(&s, id);
        assert!(shape.is_compound_shape(), "{mode}");
        assert_eq!(shape.kind_label(), "Compound Shape");
        let members: Vec<NodeId> = shape.children().unwrap().iter().map(|m| m.id).collect();
        assert_eq!(members, vec![back, front], "the members, in their stacking order");
        assert!(shape.children().unwrap().iter().all(|m| m.extras().shape_mode.name() == mode));
        assert_eq!(shape.appearance.fill_paint().color(), Some(fill), "{mode}");
        let drawn = effects::compound_shape_area(&shape, None).unwrap();
        assert!((area(&drawn) - want).abs() < 0.5, "{mode}: {}", area(&drawn));
        // Expand: the path, or compound path, of the area, painted as the compound shape.
        let made = run(&mut s, "object.compoundShape.expand", json!({}));
        let path = NodeId(made["ids"][0].as_u64().unwrap());
        let n = node(&s, path);
        assert_eq!(n.kind_label(), kind, "{mode}");
        assert!((node_area(&n) - want).abs() < 0.5, "{mode}");
        assert_eq!(n.appearance.fill_paint().color(), Some(fill));
        assert_eq!(selected(&s), vec![path]);
        assert!(s.doc().unwrap().doc.node(back).is_none() && s.doc().unwrap().doc.node(id).is_none());
        run(&mut s, "edit.undo", json!({}));
        assert!(node(&s, id).is_compound_shape());
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(s.doc().unwrap().doc.layers, before.layers, "{mode}: Undo leaves the squares as they were");
    }
}

/// Make Compound Shape from the panel menu gives every member Add; Release gives the members back
/// as they were, selected, in one undo step.
#[test]
fn release_returns_the_members_as_they_were() {
    let (mut s, back, front) = two_squares();
    let before = s.doc().unwrap().doc.clone();
    let id = NodeId(run(&mut s, "object.compoundShape.make", json!({}))["id"].as_u64().unwrap());
    assert!(node(&s, id).children().unwrap().iter().all(|m| m.extras().shape_mode == ShapeMode::Add));
    run(&mut s, "object.compoundShape.setMode", json!({"mode": "subtract", "ids": [front.0]}));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let undo = s.doc().unwrap().history.undo.len();
    let out = run(&mut s, "object.compoundShape.release", json!({}));
    assert_eq!(out["ids"], json!([back.0, front.0]));
    assert_eq!(selected(&s), vec![back, front]);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(s.doc().unwrap().doc.layers, before.layers, "the members' paint and place, no shape modes");
    // Nothing selected is in a compound shape: Release and Expand are off.
    assert!(s.execute("object.compoundShape.release", &json!({})).is_err());
    assert!(s.execute("object.compoundShape.expand", &json!({})).is_err());
    assert!(s.execute("object.compoundShape.setMode", &json!({"mode": "add"})).is_err());
}

/// A member selected (Direct Selection) takes the mode of the Shape Mode clicked; the area follows,
/// and Undo puts the mode back. Release and Expand act on the compound shape it is in.
#[test]
fn a_selected_member_takes_a_shape_mode() {
    let (mut s, _, front) = two_squares();
    let id = NodeId(run(&mut s, "object.compoundShape.make", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [front.0]}));
    assert_eq!(cmd::selected_member_modes(&s), vec![ShapeMode::Add]);
    run(&mut s, "object.compoundShape.setMode", json!({"mode": "intersect"}));
    assert_eq!(cmd::selected_member_modes(&s), vec![ShapeMode::Intersect]);
    assert!((area(&effects::compound_shape_area(&node(&s, id), None).unwrap()) - 5_000.0).abs() < 0.5);
    // The same mode again is no edit.
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, "object.compoundShape.setMode", json!({"mode": "intersect"}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    assert!(s.execute("object.compoundShape.setMode", &json!({"mode": "divide"})).is_err());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, front).extras().shape_mode, ShapeMode::Add);
    run(&mut s, "select.set", json!({"ids": [front.0]}));
    let made = run(&mut s, "object.compoundShape.expand", json!({}));
    assert!((node_area(&node(&s, NodeId(made["ids"][0].as_u64().unwrap()))) - 15_000.0).abs() < 0.5);
}

/// A compound shape holds its paint: a fill given to it (or to a member) paints the compound shape,
/// whose members keep theirs.
#[test]
fn paint_goes_to_the_compound_shape() {
    let (mut s, back, front) = two_squares();
    let id = NodeId(run(&mut s, "object.compoundShape.make", json!({}))["id"].as_u64().unwrap());
    let out = run(&mut s, "paint.setFill", json!({"color": "#00ff00"}));
    assert_eq!(out["objects"], 1);
    assert_eq!(node(&s, id).appearance.fill_paint().color(), Some(Color::rgb(0.0, 1.0, 0.0)));
    run(&mut s, "paint.setFill", json!({"color": "#ffff00", "ids": [front.0]}));
    assert_eq!(node(&s, id).appearance.fill_paint().color(), Some(Color::rgb(1.0, 1.0, 0.0)));
    assert_eq!(node(&s, back).appearance.fill_paint().color(), Some(Color::rgb(1.0, 0.0, 0.0)));
    assert_eq!(node(&s, front).appearance.fill_paint().color(), Some(Color::rgb(0.0, 0.0, 1.0)));
}

/// Make Compound Shape needs two or more objects it can take.
#[test]
fn make_needs_two_objects_it_can_take() {
    let (mut s, back, _) = two_squares();
    run(&mut s, "select.set", json!({"ids": [back.0]}));
    assert!(s.execute("object.compoundShape.make", &json!({})).is_err());
    let (mut s, back, front) = two_squares();
    assert!(s.execute("object.compoundShape.make", &json!({"mode": "divide"})).is_err());
    assert_eq!(selected(&s), vec![back, front], "nothing changed");
}

/// Saved and opened again, a compound shape keeps its members and their modes; a mode a file names
/// that isn't one reads as Add.
#[test]
fn compound_shapes_survive_a_save() {
    let (mut s, _, front) = two_squares();
    let id = NodeId(run(&mut s, "object.compoundShape.make", json!({"mode": "subtract"}))["id"].as_u64().unwrap());
    let doc = s.doc().unwrap().doc.clone();
    let bytes = vectorcraft_format::save(&doc, false);
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert_eq!(back.node(id), doc.node(id));
    assert!(back.node(id).unwrap().is_compound_shape());
    assert_eq!(back.node(front).unwrap().extras().shape_mode, ShapeMode::Subtract);
    let text = String::from_utf8(bytes).unwrap().replace("\"shape_mode\":\"subtract\"", "\"shape_mode\":\"divide\"");
    let odd = vectorcraft_format::load(text.as_bytes()).unwrap();
    assert_eq!(odd.node(front).unwrap().extras().shape_mode, ShapeMode::Add);
}

/// SVG export draws the area, painted as the compound shape, not the members.
#[test]
fn svg_export_draws_the_area() {
    let (mut s, _, _) = two_squares();
    run(&mut s, "object.compoundShape.make", json!({"mode": "subtract"}));
    let svg = vectorcraft_svg::export(&s.doc().unwrap().doc, &Default::default());
    assert!(svg.contains("#ff0000"), "the back square's red paints the area");
    assert!(!svg.contains("#0000ff"), "the front square isn't drawn: {svg}");
}

/// The Shape Modes and Pathfinders take a compound shape as the area it draws.
#[test]
fn pathfinder_operations_take_the_area() {
    let (mut s, _, _) = two_squares();
    let id = NodeId(run(&mut s, "object.compoundShape.make", json!({"mode": "subtract"}))["id"].as_u64().unwrap());
    let r = NodeId(run(&mut s, "shape.rectangle", json!({"x": 200, "y": 0, "width": 50, "height": 100}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [id.0, r.0]}));
    let out = run(&mut s, "object.pathfinder.unite", json!({}));
    let n = node(&s, NodeId(out["ids"][0].as_u64().unwrap()));
    assert!((node_area(&n) - 10_000.0).abs() < 0.5, "the 50 × 100 area and the 50 × 100 rectangle: {}", node_area(&n));
}
