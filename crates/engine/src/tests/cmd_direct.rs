//! Cmd/Ctrl held with the Selection tool lends Direct Selection (#6583).

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_geom::{PathData, Point};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use crate::tooling::ViewInfo;
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn view() -> ViewInfo {
    ViewInfo { smart_guides: false, ..Default::default() }
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn path(s: &Session, id: NodeId) -> PathData {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().clone()
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn ev(kind: PointerKind, x: f64, y: f64, m: Mods) -> PointerEvent {
    PointerEvent::new(kind, x, y).with_mods(m)
}

const NONE: Mods = Mods { cmd: false, ctrl: false, shift: false, alt: false, space: false };
const CMD: Mods = Mods { cmd: true, ..NONE };

#[test]
fn cmd_over_a_selected_anchor_shows_direct_selections_cursor() {
    let v = view();
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.select_tool("selection", v).unwrap();
    let corner = Point::new(100.0, 100.0);
    let (held, released) = (s.cursor(corner, CMD, v), s.cursor(corner, NONE, v));
    assert_eq!(s.tool_id(), "selection", "the tool itself never changes");
    // What Direct Selection itself shows there.
    s.select_tool("directSelection", v).unwrap();
    let direct = s.cursor(corner, NONE, v);
    assert_eq!(held, direct, "Direct Selection's");
    assert_ne!(held, released, "released: the Selection tool's");
}

#[test]
fn cmd_drag_is_a_direct_selection_drag_and_gives_the_selection_tool_back() {
    let v = view();
    let gesture = |s: &mut Session, m: Mods| {
        s.pointer(&ev(PointerKind::Down, 100.0, 100.0, m), v).unwrap();
        s.pointer(&ev(PointerKind::Drag, 90.0, 80.0, m), v).unwrap();
        s.pointer(&ev(PointerKind::Drag, 80.0, 70.0, m), v).unwrap();
        s.pointer(&ev(PointerKind::Up, 80.0, 70.0, m), v).unwrap();
    };
    let anchors = |s: &Session, id| path(s, id).subpaths[0].anchors.iter().map(|a| a.p).collect::<Vec<_>>();

    // Direct Selection itself, as the reference: dragging a corner of a wholly selected rectangle
    // moves all its anchors, as all are selected.
    let mut direct = session();
    let id = rect(&mut direct, 100.0, 100.0, 100.0, 100.0);
    direct.select_tool("directSelection", v).unwrap();
    gesture(&mut direct, NONE);
    let expected = anchors(&direct, id);
    assert_eq!(expected[0], Point::new(80.0, 70.0), "the dragged anchor followed the pointer");

    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.select_tool("selection", v).unwrap();
    let undo = undo_steps(&s);
    s.pointer(&ev(PointerKind::Down, 100.0, 100.0, CMD), v).unwrap();
    assert_eq!(s.tool_id(), "directSelection", "lent for the gesture");
    s.pointer(&ev(PointerKind::Drag, 90.0, 80.0, CMD), v).unwrap();
    s.pointer(&ev(PointerKind::Drag, 80.0, 70.0, CMD), v).unwrap();
    s.pointer(&ev(PointerKind::Up, 80.0, 70.0, CMD), v).unwrap();
    assert_eq!(s.tool_id(), "selection", "released: the Selection tool is back");
    assert_eq!(s.doc().unwrap().selection.objects, vec![id], "the selection is kept");
    assert_eq!(anchors(&s, id), expected, "the same drag as Direct Selection's");
    assert_eq!(undo_steps(&s) - undo, 1, "one undo step");
}

#[test]
fn cmd_pressed_mid_drag_does_not_switch_tools() {
    let v = view();
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.select_tool("selection", v).unwrap();
    // Press on the body with no key: a move of the whole object.
    s.pointer(&ev(PointerKind::Down, 150.0, 150.0, NONE), v).unwrap();
    s.pointer(&ev(PointerKind::Drag, 160.0, 150.0, NONE), v).unwrap();
    // Cmd goes down over the anchor under way: the pointer and the tool stay the Selection tool's.
    let corner = Point::new(100.0, 100.0);
    assert_eq!(s.cursor(corner, CMD, v), s.cursor(corner, NONE, v));
    s.pointer(&ev(PointerKind::Drag, 170.0, 150.0, CMD), v).unwrap();
    assert_eq!(s.tool_id(), "selection");
    s.pointer(&ev(PointerKind::Up, 170.0, 150.0, CMD), v).unwrap();
    let p = path(&s, id);
    assert_eq!(p.subpaths[0].anchors[0].p, Point::new(120.0, 100.0), "the whole object moved");
    assert_eq!(p.subpaths[0].anchors[2].p, Point::new(220.0, 200.0));
}

#[test]
fn cmd_click_still_selects_behind_away_from_a_selected_objects_parts() {
    let v = view();
    let mut s = session();
    let back = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let front = rect(&mut s, 150.0, 100.0, 100.0, 100.0);
    s.select_tool("selection", v).unwrap();
    // Inside both squares, off every anchor and edge of the selected front one.
    let at = (175.0, 150.0);
    assert_eq!(
        s.cursor(Point::new(at.0, at.1), CMD, v),
        s.cursor(Point::new(at.0, at.1), NONE, v),
        "Select Behind keeps the Selection tool's pointer"
    );
    s.pointer(&ev(PointerKind::Down, at.0, at.1, CMD), v).unwrap();
    assert_eq!(s.tool_id(), "selection");
    s.pointer(&ev(PointerKind::Up, at.0, at.1, CMD), v).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, vec![back], "Cmd-click selects behind");
    assert_ne!(back, front);
}

#[test]
fn without_select_behind_cmd_always_lends_direct_selection() {
    let v = view();
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.prefs.ctrl_click_selects_behind = false;
    s.select_tool("selection", v).unwrap();
    let inside = Point::new(150.0, 150.0);
    assert_ne!(s.cursor(inside, CMD, v), s.cursor(inside, NONE, v), "Direct Selection's pointer");
    s.pointer(&ev(PointerKind::Down, inside.x, inside.y, CMD), v).unwrap();
    assert_eq!(s.tool_id(), "directSelection");
    s.pointer(&ev(PointerKind::Up, inside.x, inside.y, CMD), v).unwrap();
    assert_eq!(s.tool_id(), "selection");
}

#[test]
fn cmd_shift_keeps_the_selection_tool() {
    let v = view();
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.select_tool("selection", v).unwrap();
    let both = Mods { shift: true, ..CMD };
    assert_eq!(s.cursor(Point::new(100.0, 100.0), both, v), s.cursor(Point::new(100.0, 100.0), NONE, v));
    s.pointer(&ev(PointerKind::Down, 100.0, 100.0, both), v).unwrap();
    assert_eq!(s.tool_id(), "selection");
    s.pointer(&ev(PointerKind::Up, 100.0, 100.0, both), v).unwrap();
}
