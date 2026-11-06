//! Shape tools: the selection they keep on show moves by its centre point (Attributes › Show
//! Center) and turns from the rotate zone just outside a corner of its box, as the Selection
//! tool's do, through the whole engine (#5477).

use serde_json::json;
use vectorcraft_doc::{LiveShape, NodeKind};
use vectorcraft_geom::{Point, Rect, Vec2};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::*;

fn new_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn view() -> ViewInfo {
    ViewInfo { smart_guides: false, ..ViewInfo::default() }
}

fn gesture(s: &mut Session, mods: Mods, events: &[(PointerKind, f64, f64)]) {
    for (k, x, y) in events {
        s.pointer(&PointerEvent::new(*k, *x, *y).with_mods(mods), view()).unwrap();
    }
}

const NONE: Mods = Mods { shift: false, alt: false, cmd: false, ctrl: false, space: false };

/// A 100 × 80 pt rectangle drawn from (100, 100) to (200, 180) with the Rectangle tool, still
/// selected: its centre point is at (150, 140).
fn drawn_rect(s: &mut Session) -> NodeId {
    s.select_tool("rectangle", view()).unwrap();
    gesture(s, NONE, &[(PointerKind::Down, 100.0, 100.0), (PointerKind::Drag, 200.0, 180.0), (PointerKind::Up, 200.0, 180.0)]);
    *s.doc().unwrap().selection.objects.first().expect("the drawn rectangle is selected")
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.bounds_of(&[id], false).expect("bounds")
}

fn angle(s: &Session, id: NodeId) -> f64 {
    s.doc().unwrap().doc.oriented_bounds(&[id], false).map_or(0.0, |b| b.angle)
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// With the Rectangle tool still active, a drag from the shape's centre point moves it, in one
/// undo step; Alt-dragging it copies it; Escape turns a drag back.
#[test]
fn a_shape_tool_moves_the_shape_from_its_centre_point() {
    let mut s = new_doc();
    let id = drawn_rect(&mut s);
    let before = bounds(&s, id);
    let steps = undo_steps(&s);
    gesture(
        &mut s,
        NONE,
        &[(PointerKind::Down, 150.0, 140.0), (PointerKind::Drag, 170.0, 160.0), (PointerKind::Drag, 180.0, 170.0), (PointerKind::Up, 180.0, 170.0)],
    );
    assert_eq!(bounds(&s, id), before + Vec2::new(30.0, 30.0), "moved by the drag");
    assert_eq!(undo_steps(&s), steps + 1, "one undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(bounds(&s, id), before);
    // Alt-dragging from the centre leaves a copy where the original was.
    gesture(
        &mut s,
        Mods { alt: true, ..NONE },
        &[(PointerKind::Down, 150.0, 140.0), (PointerKind::Drag, 200.0, 140.0), (PointerKind::Up, 200.0, 140.0)],
    );
    let objects = s.doc().unwrap().doc.layers[0].children().map(|c| c.len()).unwrap_or_default();
    assert_eq!(objects, 2, "a copy");
    // Escape turns the drag back, and the release leaves the art alone.
    let steps = undo_steps(&s);
    let moved = s.doc().unwrap().selection.objects.clone();
    let copy = *moved.first().expect("the copy is selected");
    let at = bounds(&s, copy);
    gesture(&mut s, NONE, &[(PointerKind::Down, 200.0, 140.0), (PointerKind::Drag, 250.0, 190.0)]);
    s.tool_key(ToolKey::Escape, NONE, view()).unwrap();
    gesture(&mut s, NONE, &[(PointerKind::Up, 250.0, 190.0)]);
    assert_eq!(undo_steps(&s), steps, "no step");
    assert_eq!(s.doc().unwrap().selection.objects, moved);
    assert_eq!(bounds(&s, copy), at, "back where it was");
}

/// A press in the rotate zone just outside a corner of the shape's box turns it about that box's
/// centre, as the Selection tool's does; a press on the corner itself still rounds it.
#[test]
fn a_shape_tool_turns_the_shape_from_just_outside_a_corner() {
    let mut s = new_doc();
    let id = drawn_rect(&mut s);
    // 8.49 pt outside the top-left corner (100, 100), past the handle's 5 pt square.
    gesture(
        &mut s,
        NONE,
        &[(PointerKind::Down, 94.0, 94.0), (PointerKind::Drag, 150.0, 70.0), (PointerKind::Drag, 150.0, 70.0), (PointerKind::Up, 150.0, 70.0)],
    );
    let turned = angle(&s, id);
    assert!(turned.abs() > 1.0, "turned: {turned}");
    let centre = bounds(&s, id).center();
    assert!((centre - Point::new(150.0, 140.0)).hypot() < 1e-6, "about its centre: {centre:?}");
    // A press on a Live Corners widget rounds it, as before (#5477): the widget keeps the corner,
    // and sits 10 pt inside it.
    let mut s = new_doc();
    let id = drawn_rect(&mut s);
    gesture(&mut s, NONE, &[(PointerKind::Down, 110.0, 110.0), (PointerKind::Drag, 125.0, 125.0), (PointerKind::Up, 125.0, 125.0)]);
    let n = s.doc().unwrap().doc.node(id).expect("the rectangle");
    let NodeKind::Path { live: Some(live), .. } = &n.kind else { panic!("a live shape: {:?}", n.kind) };
    let LiveShape::Rectangle { radii, .. } = &**live else { panic!("a rectangle") };
    assert!(radii[0] > 0.0, "the corner rounded: {radii:?}");
    assert_eq!(angle(&s, id), 0.0, "rounding does not turn it");
}

/// The Selection tool takes the centre point of an unfilled shape too: a drag there moves it
/// instead of starting a marquee.
#[test]
fn the_selection_tool_moves_an_unfilled_shape_from_its_centre_point() {
    let mut s = new_doc();
    let id = NodeId(s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 80})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.select_tool("selection", view()).unwrap();
    let before = bounds(&s, id);
    gesture(&mut s, NONE, &[(PointerKind::Down, 150.0, 140.0), (PointerKind::Drag, 170.0, 160.0), (PointerKind::Up, 170.0, 160.0)]);
    assert_eq!(bounds(&s, id), before + Vec2::new(20.0, 20.0));
    // Away from it, a drag is still a marquee.
    let steps = undo_steps(&s);
    gesture(&mut s, NONE, &[(PointerKind::Down, 300.0, 300.0), (PointerKind::Drag, 350.0, 350.0), (PointerKind::Up, 350.0, 350.0)]);
    assert_eq!(undo_steps(&s), steps, "a marquee changes no art");
    assert!(s.doc().unwrap().selection.objects.is_empty(), "the marquee took nothing");
}
