//! Fill and stroke given to a path of a compound path paint the compound path (#3692): the
//! compound holds the paint, so a member's own appearance is never drawn.

use serde_json::{Value, json};
use vectorcraft_color::Paint;

use crate::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 0, "width": 40, "height": 40}))["id"].as_u64().unwrap())
}

/// A compound path of two black rectangles and a loose black rectangle → (compound, members, loose).
fn compound() -> (Session, NodeId, Vec<NodeId>, NodeId) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 100}));
    let (a, b) = (rect(&mut s, 0.0), rect(&mut s, 10.0));
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    run(&mut s, "object.compoundPath.make", json!({}));
    let compound = s.doc().unwrap().selection.objects[0];
    let loose = rect(&mut s, 100.0);
    run(&mut s, "paint.setFill", json!({"color": "#000000", "ids": [compound.0, loose.0]}));
    run(&mut s, "paint.setStroke", json!({"color": "#000000", "ids": [compound.0, loose.0]}));
    let doc = &s.doc().unwrap().doc;
    let members: Vec<NodeId> = doc.node(compound).unwrap().children().into_iter().flatten().map(|m| m.id).collect();
    assert_eq!(members.len(), 2);
    (s, compound, members, loose)
}

/// The hex of `id`'s own top fill (`fill`) or stroke, `None` when it has none or it isn't solid.
fn paint(s: &Session, id: NodeId, fill: bool) -> Option<String> {
    match s.doc().unwrap().doc.node(id).unwrap().appearance.paint_at(None, fill)? {
        Paint::Solid { color, .. } => Some(color.to_hex()),
        _ => None,
    }
}

fn solid(hex: &str) -> Option<String> {
    Some(hex.to_string())
}

#[test]
fn a_fill_given_to_a_member_id_paints_its_compound() {
    let (mut s, compound, members, loose) = compound();
    let member_paint = paint(&s, members[0], true);
    run(&mut s, "paint.setFill", json!({"color": "#ff00ff", "ids": [members[0].0, members[1].0, loose.0]}));
    assert_eq!(paint(&s, compound, true), solid("#ff00ff"), "the compound path takes the fill");
    assert_eq!(paint(&s, loose, true), solid("#ff00ff"));
    assert_eq!(paint(&s, members[0], true), member_paint, "the member's own appearance is left alone");
    assert_eq!(paint(&s, compound, false), solid("#000000"), "the stroke stays");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(paint(&s, compound, true), solid("#000000"), "one undo step");
}

#[test]
fn a_stroke_given_to_a_selected_member_paints_its_compound() {
    let (mut s, compound, members, _) = compound();
    run(&mut s, "select.set", json!({"ids": [members[1].0]}));
    run(&mut s, "paint.setStroke", json!({"color": "#00ff00"}));
    assert_eq!(paint(&s, compound, false), solid("#00ff00"));
    run(&mut s, "paint.setFill", json!({"color": "#0000ff"}));
    assert_eq!(paint(&s, compound, true), solid("#0000ff"));
}

#[test]
fn the_members_of_one_compound_are_one_target() {
    let (mut s, compound, members, _) = compound();
    run(&mut s, "paint.setFill", json!({"color": "#ff0000", "ids": [compound.0]}));
    // Swapped once per id, the two members would swap the compound path back.
    run(&mut s, "paint.swap", json!({"ids": [members[0].0, members[1].0]}));
    assert_eq!(paint(&s, compound, true), solid("#000000"), "swapped once, not once per member");
    assert_eq!(paint(&s, compound, false), solid("#ff0000"));
}

#[test]
fn a_group_with_a_compound_still_paints_the_compound_once() {
    let (mut s, compound, _, loose) = compound();
    run(&mut s, "select.set", json!({"ids": [compound.0, loose.0]}));
    let group = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "paint.setFill", json!({"color": "#ff0000", "ids": [group.0]}));
    run(&mut s, "paint.swap", json!({"ids": [group.0]}));
    assert_eq!(paint(&s, compound, true), solid("#000000"));
    assert_eq!(paint(&s, compound, false), solid("#ff0000"));
    assert_eq!(paint(&s, loose, false), solid("#ff0000"));
}
