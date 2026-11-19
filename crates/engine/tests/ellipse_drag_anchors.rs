//! Ellipse arrow keys edit the anchor count of the shape actually being dragged.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_doc::{LiveShape, Node, NodeId};
use vectorcraft_engine::{Session, ViewInfo};
use vectorcraft_geom::{AnchorKind, Rect};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.execute(command, &params).unwrap()
}

fn view() -> ViewInfo {
    ViewInfo { smart_guides: false, snap_to_point: false, snap_to_grid: false, snap_to_pixel: false, ..ViewInfo::default() }
}

fn fixture() -> (Session, NodeId) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 700, "height": 500}));
    let other = NodeId(run(&mut s, "shape.rectangle", json!({"x": 400, "y": 300, "width": 30, "height": 20}))["id"].as_u64().unwrap());
    run(&mut s, "select.none", json!({}));
    s.select_tool("ellipse", view()).unwrap();
    (s, other)
}

fn pointer(s: &mut Session, view: ViewInfo, kind: PointerKind, x: f64, y: f64, mods: Mods) {
    s.pointer(&PointerEvent::new(kind, x, y).with_mods(mods), view).unwrap();
}

fn begin(s: &mut Session, view: ViewInfo, mods: Mods) {
    run(s, "select.none", json!({}));
    pointer(s, view, PointerKind::Down, 100.0, 100.0, mods);
    pointer(s, view, PointerKind::Drag, 220.0, 180.0, mods);
}

fn key(s: &mut Session, view: ViewInfo, key: ToolKey) {
    s.tool_key(key, Mods::default(), view).unwrap();
}

fn drawn(s: &Session) -> Node {
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects.len(), 1, "one drawn ellipse is selected");
    st.doc.node(st.selection.objects[0]).unwrap().clone()
}

fn count(s: &Session) -> usize {
    drawn(s).path_data().unwrap().anchor_count()
}

fn bounds_are(node: &Node, expected: Rect) {
    let got = node.geometric_bounds().unwrap();
    for (a, b) in [(got.x0, expected.x0), (got.y0, expected.y0), (got.x1, expected.x1), (got.y1, expected.y1)] {
        assert!((a - b).abs() < 1e-6, "bounds {got:?}, expected {expected:?}");
    }
}

fn document(s: &Session) -> Value {
    json!(s.doc().unwrap().doc)
}

fn depth(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn ellipse_drag_anchors_baseline_default_four_is_the_live_ellipse() {
    let (mut s, _) = fixture();
    let before = depth(&s);
    begin(&mut s, view(), Mods::default());
    let preview = drawn(&s);
    assert_eq!(preview.path_data().unwrap().anchor_count(), 4);
    assert!(matches!(preview.live_shape(), Some(LiveShape::Ellipse { .. })));
    bounds_are(&preview, Rect::new(100.0, 100.0, 220.0, 180.0));
    pointer(&mut s, view(), PointerKind::Up, 220.0, 180.0, Mods::default());
    assert_eq!(depth(&s), before + 1);
    assert!(matches!(drawn(&s).live_shape(), Some(LiveShape::Ellipse { .. })));
}

#[test]
fn ellipse_drag_anchors_baseline_up_changes_preview_and_commits_once() {
    let (mut s, other) = fixture();
    let before = document(&s);
    let other_before = json!(s.doc().unwrap().doc.node(other));
    let history = depth(&s);
    begin(&mut s, view(), Mods::default());
    assert_eq!(count(&s), 4, "normal drag before the key");
    key(&mut s, view(), ToolKey::Up);
    assert_eq!(count(&s), 5, "Up changes the shape currently previewed");
    let preview = drawn(&s);
    assert!(preview.live_shape().is_none(), "a non-four count is the existing plain path");
    assert!(preview.path_data().unwrap().subpaths.iter().all(|sp| sp.closed));
    assert!(preview.path_data().unwrap().subpaths.iter().flat_map(|sp| &sp.anchors).all(|a| a.kind == AnchorKind::Smooth));
    bounds_are(&preview, Rect::new(100.0, 100.0, 220.0, 180.0));
    pointer(&mut s, view(), PointerKind::Up, 220.0, 180.0, Mods::default());
    assert_eq!(count(&s), 5);
    bounds_are(&drawn(&s), Rect::new(100.0, 100.0, 220.0, 180.0));
    assert_eq!(drawn(&s).appearance, preview.appearance);
    assert_eq!(json!(s.doc().unwrap().doc.node(other)), other_before);
    assert_eq!(depth(&s), history + 1, "keys update one preview, not separate edits");
    let after = document(&s);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(document(&s), before);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(document(&s), after);
    let idle = document(&s);
    key(&mut s, view(), ToolKey::Up);
    assert_eq!(document(&s), idle, "an idle tool key does not change existing art");
    assert_eq!(s.tool_options()["anchors"], 5);
}

#[test]
fn ellipse_drag_anchors_limits_remembered_count_and_cancel_baseline() {
    for (initial, pressed, expected) in [(2, ToolKey::Down, 2), (999, ToolKey::Up, 1000)] {
        let (mut s, _) = fixture();
        s.set_tool_option("anchors", &json!(initial));
        begin(&mut s, view(), Mods::default());
        assert_eq!(count(&s), initial, "the remembered click count starts the drag");
        key(&mut s, view(), pressed);
        key(&mut s, view(), pressed);
        assert_eq!(count(&s), expected, "one step per key, clamped to the existing domain");
        bounds_are(&drawn(&s), Rect::new(100.0, 100.0, 220.0, 180.0));
        pointer(&mut s, view(), PointerKind::Up, 220.0, 180.0, Mods::default());
        s.select_tool("selection", view()).unwrap();
        s.select_tool("ellipse", view()).unwrap();
        assert_eq!(s.tool_options()["anchors"], expected);
    }
    let (mut s, _) = fixture();
    s.set_tool_option("anchors", &json!(6));
    let before = document(&s);
    let history = depth(&s);
    begin(&mut s, view(), Mods::default());
    key(&mut s, view(), ToolKey::Up);
    assert_eq!(count(&s), 7);
    key(&mut s, view(), ToolKey::Escape);
    assert_eq!(document(&s), before);
    assert_eq!(depth(&s), history);
    assert_eq!(s.tool_options()["anchors"], 6, "Cancel restores the count before the drag");
    begin(&mut s, view(), Mods::default());
    key(&mut s, view(), ToolKey::Up);
    s.set_tool_option("anchors", &json!(9));
    key(&mut s, view(), ToolKey::Up);
    assert_eq!(count(&s), 10);
    key(&mut s, view(), ToolKey::Escape);
    assert_eq!(document(&s), before);
    assert_eq!(depth(&s), history);
    assert_eq!(s.tool_options()["anchors"], 9, "an explicit setting updates the Cancel baseline");
    s.select_tool("selection", view()).unwrap();
    s.select_tool("ellipse", view()).unwrap();
    assert_eq!(s.tool_options()["anchors"], 9, "the restored option survives switching tools");
}

#[test]
fn ellipse_drag_anchors_keys_keep_shift_alt_space_and_turned_axes() {
    for angle in [0.0, 90.0] {
        let (mut s, _) = fixture();
        s.prefs.constrain_angle = angle;
        let mods = Mods { shift: true, alt: true, ..Mods::default() };
        pointer(&mut s, view(), PointerKind::Down, 100.0, 100.0, mods);
        pointer(&mut s, view(), PointerKind::Drag, 180.0, 160.0, mods);
        bounds_are(&drawn(&s), Rect::new(20.0, 20.0, 180.0, 180.0));
        key(&mut s, view(), ToolKey::Up);
        assert_eq!(count(&s), 5);
        bounds_are(&drawn(&s), Rect::new(20.0, 20.0, 180.0, 180.0));
        let moving = Mods { space: true, ..mods };
        pointer(&mut s, view(), PointerKind::Drag, 200.0, 180.0, moving);
        bounds_are(&drawn(&s), Rect::new(40.0, 40.0, 200.0, 200.0));
        key(&mut s, view(), ToolKey::Up);
        assert_eq!(count(&s), 6);
        bounds_are(&drawn(&s), Rect::new(40.0, 40.0, 200.0, 200.0));
        pointer(&mut s, view(), PointerKind::Up, 200.0, 180.0, moving);
        assert_eq!(count(&s), 6);
        bounds_are(&drawn(&s), Rect::new(40.0, 40.0, 200.0, 200.0));
    }
}

#[test]
fn ellipse_drag_anchors_key_keeps_the_tangent_snapped_box() {
    let (mut s, _) = fixture();
    let line = NodeId(run(&mut s, "shape.line", json!({"x1": 40, "y1": 200, "x2": 260, "y2": 200}))["id"].as_u64().unwrap());
    let line_before = json!(s.doc().unwrap().doc.node(line));
    run(&mut s, "select.none", json!({}));
    s.prefs.snap_tangent = true;
    let snap = ViewInfo { smart_guides: true, snap_to_point: false, ..view() };
    let mods = Mods { shift: true, alt: true, ..Mods::default() };
    pointer(&mut s, snap, PointerKind::Down, 100.0, 100.0, mods);
    pointer(&mut s, snap, PointerKind::Drag, 198.0, 130.0, mods);
    // The pointer alone gives radius98; the horizontal line pulls the shown radius to100.
    bounds_are(&drawn(&s), Rect::new(0.0, 0.0, 200.0, 200.0));
    key(&mut s, snap, ToolKey::Up);
    assert_eq!(count(&s), 5);
    bounds_are(&drawn(&s), Rect::new(0.0, 0.0, 200.0, 200.0));
    pointer(&mut s, snap, PointerKind::Up, 198.0, 130.0, mods);
    assert_eq!(count(&s), 5);
    bounds_are(&drawn(&s), Rect::new(0.0, 0.0, 200.0, 200.0));
    assert_eq!(json!(s.doc().unwrap().doc.node(line)), line_before);
}
