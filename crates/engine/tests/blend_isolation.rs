//! #6618: double-click entry to live blend isolation and editing its existing keys.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_engine::{Session, ViewInfo};
use vectorcraft_geom::Point;
use vectorcraft_tools::{PointerEvent, PointerKind};

fn fixture(curved: bool) -> (Session, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let rectangle = |s: &mut Session, x, y| {
        NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap())
    };
    let (a, b) = (rectangle(&mut s, 100, 100), rectangle(&mut s, 300, 100));
    rectangle(&mut s, 550, 350);
    let blend = NodeId(
        s.execute("object.blend.make", &json!({"ids": [a.0, b.0], "steps": 3, "orientation": if curved { "path" } else { "page" }})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    if curved {
        let path = s.execute("path.create", &json!({"d": "M120 120 C120 220 320 220 320 120"})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [blend.0, path]})).unwrap();
        s.execute("object.blend.replaceSpine", &json!({})).unwrap();
    }
    s.execute("prefs.set", &json!({"key": "doubleClickToIsolate", "value": true})).unwrap();
    s.select_tool("selection", ViewInfo::default()).unwrap();
    (s, blend, a, b)
}

fn pointer(s: &mut Session, kind: PointerKind, x: f64, y: f64) {
    s.pointer(&PointerEvent::new(kind, x, y), ViewInfo { smart_guides: false, ..ViewInfo::default() }).unwrap();
}

fn spec(s: &Session, blend: NodeId) -> serde_json::Value {
    let NodeKind::Blend { spec, .. } = &s.doc().unwrap().doc.node(blend).unwrap().kind else { panic!("still a live blend") };
    serde_json::to_value(spec).unwrap()
}

fn drawn_centers(s: &Session, blend: NodeId) -> Vec<Point> {
    let NodeKind::Blend { children, spec } = &s.doc().unwrap().doc.node(blend).unwrap().kind else { panic!("still a live blend") };
    vectorcraft_doc::live::blend_expand(children, spec).iter().map(|n| n.geometric_bounds().unwrap().center()).collect()
}

#[test]
fn double_click_blend_isolation_limits_selection_and_keeps_its_spine() {
    for curved in [false, true] {
        let (mut s, blend, a, b) = fixture(curved);
        let before = spec(&s, blend);
        pointer(&mut s, PointerKind::DoubleClick, 120.0, 120.0);
        assert_eq!(s.doc().unwrap().isolation, Some(blend), "curved: {curved}");
        assert!(s.doc().unwrap().selection.objects.is_empty());
        s.execute("select.all", &json!({})).unwrap();
        let selected = &s.doc().unwrap().selection.objects;
        assert!(selected.len() == 2 && selected.contains(&a) && selected.contains(&b));
        pointer(&mut s, PointerKind::Down, 570.0, 370.0);
        pointer(&mut s, PointerKind::Up, 570.0, 370.0);
        assert!(s.doc().unwrap().selection.objects.is_empty(), "outside art is not selected");
        pointer(&mut s, PointerKind::Down, 120.0, 120.0);
        pointer(&mut s, PointerKind::Up, 120.0, 120.0);
        assert_eq!(s.doc().unwrap().selection.objects, vec![a], "the key is selected on its own");
        assert_eq!(spec(&s, blend), before, "entry and selection keep spacing, orientation and spine");
        pointer(&mut s, PointerKind::DoubleClick, 750.0, 550.0);
        assert_eq!(s.doc().unwrap().isolation, None);
        assert_eq!(s.doc().unwrap().selection.objects, vec![blend]);
        assert_eq!(spec(&s, blend), before);
    }
}

#[test]
fn dragging_a_key_after_double_click_isolation_undoes_without_expanding_the_blend() {
    let (mut s, blend, a, b) = fixture(false);
    let before = drawn_centers(&s, blend);
    pointer(&mut s, PointerKind::DoubleClick, 120.0, 120.0);
    assert_eq!(s.doc().unwrap().isolation, Some(blend));
    pointer(&mut s, PointerKind::Down, 120.0, 120.0);
    pointer(&mut s, PointerKind::Up, 120.0, 120.0);
    pointer(&mut s, PointerKind::Down, 120.0, 120.0);
    pointer(&mut s, PointerKind::Drag, 130.0, 130.0);
    pointer(&mut s, PointerKind::Up, 130.0, 130.0);
    let x = |s: &Session| s.doc().unwrap().doc.node(a).unwrap().geometric_bounds().unwrap().x0;
    assert!((x(&s) - 110.0).abs() < 1e-6);
    assert_eq!(s.doc().unwrap().doc.node(b).unwrap().geometric_bounds().unwrap().x0, 300.0, "the other key stays in place");
    let moved = drawn_centers(&s, blend);
    assert_eq!(moved.len(), 5, "the two keys and three intermediate steps are drawn");
    assert!(moved[0].distance(Point::new(130.0, 130.0)) < 1e-6, "the drawn key follows its selection outline");
    assert!(moved[2].distance(Point::new(225.0, 125.0)) < 1e-6, "the middle step updates with the moved key");
    assert_eq!(moved.last(), before.last(), "the other drawn key stays in place");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!((x(&s) - 100.0).abs() < 1e-6);
    assert_eq!(drawn_centers(&s, blend), before, "Undo restores the generated art too");
    s.execute("edit.redo", &json!({})).unwrap();
    assert!((x(&s) - 110.0).abs() < 1e-6);
    assert_eq!(drawn_centers(&s, blend), moved, "Redo restores the generated art too");
    assert_eq!(s.doc().unwrap().doc.parent_of(a), Some(blend));
    assert_eq!(s.doc().unwrap().isolation, Some(blend));
    assert_eq!(spec(&s, blend)["spacing"], json!({"steps": 3}));
    s.execute("object.exitIsolation", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, vec![blend]);
}
