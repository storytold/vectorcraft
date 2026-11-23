//! Live shape widgets through MCP: a shape tool's double-click on a selected live shape's corner
//! widget, whose gesture's `requests` carry the Transform panel, and a drag on a live polygon's
//! side widget.

use serde_json::{Value, json};
use vectorcraft_tools::corners::CornerWidgets;

use crate::{Backend, Headless, call_tool};

fn ok(h: &mut Headless, name: &str, args: Value) -> Value {
    let r = call_tool(h, name, &args);
    assert!(!r.is_error, "{name} {args}: {r:?}");
    serde_json::from_str(r.content[0]["text"].as_str().unwrap()).unwrap_or(Value::Null)
}

#[test]
fn a_corner_widget_double_click_with_a_shape_tool_requests_the_transform_panel() {
    let mut h = Headless::new();
    h.call("engine.execute", json!({"command": "file.new", "params": {"width": 300, "height": 300}})).unwrap();
    ok(&mut h, "draw_shape", json!({"shape": "rectangle", "x": 100, "y": 100, "width": 100, "height": 100}));
    let st = h.session.doc().unwrap();
    let w = CornerWidgets::showing(&st.doc, &st.selection, h.view.zoom, false, true, 180.0).expect("corner widgets show");
    let at = w.visible().next().expect("a widget");
    let steps = st.history.undo.len();
    let dbl = ok(&mut h, "pointer_gesture", json!({"tool": "rectangle", "events": [{"kind": "doubleclick", "x": at.x, "y": at.y}]}));
    assert_eq!(dbl["requests"], json!([{"panel": "transform"}]), "{dbl}");
    assert_eq!(h.session.doc().unwrap().history.undo.len(), steps, "no undo step");
    // Off the widgets, the double-click asks for nothing.
    let off = ok(&mut h, "pointer_gesture", json!({"events": [{"kind": "doubleclick", "x": 280, "y": 280}]}));
    assert_eq!(off["requests"], json!([]), "{off}");
}

/// The selected live polygon's sides, radius and transform.
fn polygon(h: &mut Headless) -> (u32, f64, vectorcraft_geom::Affine) {
    let st = h.session.doc().unwrap();
    let id = st.selection.objects[0];
    let vectorcraft_doc::NodeKind::Path { live, .. } = &st.doc.node(id).unwrap().kind else { panic!("not a path") };
    let Some(vectorcraft_doc::LiveShape::Polygon { sides, radius, xf, .. }) = live.as_deref() else { panic!("not a live polygon: {live:?}") };
    (*sides, *radius, *xf)
}

/// A drag down on a live polygon's side widget with the Selection tool gives it more sides in one
/// undo step, keeping its radius and transform; undo puts the count back.
#[test]
fn a_side_widget_drag_changes_a_polygons_sides_in_one_undo_step() {
    let mut h = Headless::new();
    h.call("engine.execute", json!({"command": "file.new", "params": {"width": 300, "height": 300}})).unwrap();
    h.call("engine.execute", json!({"command": "shape.polygon", "params": {"cx": 150, "cy": 150, "radius": 60, "sides": 5}})).unwrap();
    let st = h.session.doc().unwrap();
    let id = st.selection.objects[0];
    let b = st.doc.node(id).unwrap().geometric_bounds_with(&st.doc.symbol_boxes()).expect("bounds");
    let bx = vectorcraft_doc::OrientedBox::aligned(b);
    let w = vectorcraft_tools::sidewidget::SideWidget::of(&st.doc, &st.selection, &bx, h.view.zoom).expect("a side widget");
    let steps = st.history.undo.len();
    let before = polygon(&mut h);
    assert_eq!(before.0, 5);
    let zoom = h.view.zoom;
    let ev = |kind: &str, dy: f64| json!({"kind": kind, "x": w.at.x, "y": w.at.y + dy / zoom});
    ok(&mut h, "pointer_gesture", json!({"tool": "selection", "events": [ev("down", 0.0), ev("drag", 13.0), ev("drag", 25.0), ev("up", 25.0)]}));
    let after = polygon(&mut h);
    assert_eq!((after.0, after.1, after.2), (7, before.1, before.2));
    assert_eq!(h.session.doc().unwrap().history.undo.len(), steps + 1, "one undo step");
    h.call("engine.execute", json!({"command": "edit.undo", "params": {}})).unwrap();
    assert_eq!(polygon(&mut h), before);
    // A drag that comes back to the start leaves no undo step.
    ok(&mut h, "pointer_gesture", json!({"tool": "selection", "events": [ev("down", 0.0), ev("drag", 25.0), ev("drag", 0.0), ev("up", 0.0)]}));
    assert_eq!(polygon(&mut h), before);
    assert_eq!(h.session.doc().unwrap().history.undo.len(), steps);
}

/// A batch run during a side widget drag settles the drag first. A batch step that switches
/// documents then ends the drag without undoing the batch's own steps in the document it leaves.
#[test]
fn a_batch_that_switches_documents_during_a_side_widget_drag_keeps_its_steps() {
    let mut h = Headless::new();
    for _ in 0..2 {
        h.call("engine.execute", json!({"command": "file.new", "params": {"width": 300, "height": 300}})).unwrap();
    }
    h.call("engine.execute", json!({"command": "shape.polygon", "params": {"cx": 150, "cy": 150, "radius": 60, "sides": 5}})).unwrap();
    let st = h.session.doc().unwrap();
    let (uid, id) = (st.uid, st.selection.objects[0]);
    let b = st.doc.node(id).unwrap().geometric_bounds_with(&st.doc.symbol_boxes()).expect("bounds");
    let bx = vectorcraft_doc::OrientedBox::aligned(b);
    let w = vectorcraft_tools::sidewidget::SideWidget::of(&st.doc, &st.selection, &bx, h.view.zoom).expect("a side widget");
    let steps = st.history.undo.len();
    let zoom = h.view.zoom;
    let ev = |kind: &str, dy: f64| json!({"kind": kind, "x": w.at.x, "y": w.at.y + dy / zoom});
    // The drag stays held: no `up`.
    ok(&mut h, "pointer_gesture", json!({"tool": "selection", "events": [ev("down", 0.0), ev("drag", 13.0), ev("drag", 25.0)]}));
    let batch = json!({"commands": [
        {"command": "shape.rectangle", "params": {"x": 10, "y": 10, "width": 20, "height": 20}},
        {"command": "document.activate", "params": {"index": 0}}
    ]});
    h.call("engine.execute", json!({"command": "command.batch", "params": batch})).unwrap();
    assert_eq!(h.session.active_index(), Some(0), "the batch activates document 0");
    assert_ne!(h.session.doc().unwrap().uid, uid, "the batch leaves the polygon's document");
    let st = h.session.documents().iter().find(|d| d.uid == uid).expect("the polygon's document");
    let vectorcraft_doc::NodeKind::Path { live, .. } = &st.doc.node(id).unwrap().kind else { panic!("not a path") };
    assert!(matches!(live.as_deref(), Some(vectorcraft_doc::LiveShape::Polygon { sides: 7, .. })), "the settled drag: {live:?}");
    assert_eq!(st.history.undo.len(), steps + 2, "the drag and the batch, each one undo step");
    let rect = st.selection.objects[0];
    assert_ne!(rect, id, "the batch's rectangle is selected");
    assert!(st.doc.node(rect).is_some(), "the batch's rectangle stays");
}
