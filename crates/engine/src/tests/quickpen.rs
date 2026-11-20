//! The Quick Pen tool driven through the session, and `path.quickPen`.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{PathData, Point, SubPath};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

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

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn paths(s: &Session) -> Vec<(NodeId, PathData)> {
    let mut v = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if let NodeKind::Path { path, .. } = &n.kind {
            v.push((n.id, path.clone()));
        }
    });
    v
}

fn only_path(s: &Session) -> SubPath {
    let all = paths(s);
    assert_eq!(all.len(), 1, "one path");
    all[0].1.subpaths[0].clone()
}

fn pt(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

fn press(s: &mut Session, pts: &[(f64, f64)]) {
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, pts[0].0, pts[0].1), v).unwrap();
    for &(x, y) in &pts[1..] {
        s.pointer(&PointerEvent::new(PointerKind::Drag, x, y), v).unwrap();
    }
    let l = pts[pts.len() - 1];
    s.pointer(&PointerEvent::new(PointerKind::Up, l.0, l.1), v).unwrap();
}

fn click(s: &mut Session, x: f64, y: f64) {
    press(s, &[(x, y)]);
}

fn key(s: &mut Session, k: ToolKey) {
    s.tool_key(k, Mods::default(), view()).unwrap();
}

fn quick_pen(s: &mut Session) {
    s.select_tool("quickPen", view()).unwrap();
}

/// Clicks draw lines; Alt tapped alone switches to the next action, so the next clicks draw a
/// continuous arc and a sharp-corner half circle. Each piece is one undo step.
#[test]
fn clicks_draw_lines_and_arcs_one_undo_step_each() {
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    let undo = undo_steps(&s);
    click(&mut s, 200.0, 100.0);
    assert_eq!(undo_steps(&s), undo + 1);
    key(&mut s, ToolKey::AltTap);
    assert_eq!(s.tool_options()["action"], "continuousArc");
    click(&mut s, 300.0, 200.0);
    key(&mut s, ToolKey::AltTap);
    assert_eq!(s.tool_options()["action"], "sharpArc");
    click(&mut s, 300.0, 300.0);
    assert_eq!(undo_steps(&s), undo + 3);
    let sp = only_path(&s);
    assert!(!sp.closed);
    let ends: Vec<Point> = sp.anchors.iter().map(|a| a.p).collect();
    assert_eq!(ends.first(), Some(&pt(100.0, 100.0)));
    assert_eq!(ends.last(), Some(&pt(300.0, 300.0)));
    // Line (2 points), quarter circle (1 more), half circle in two quarters (2 more).
    assert_eq!(ends.len(), 5);
    // The continuous arc left (200, 100) heading right, as the line arrived.
    let joint = sp.anchors.get(1).unwrap();
    assert!((joint.h_out - joint.p).y.abs() < 1e-9 && joint.h_out.x > joint.p.x);
    // The half circle on (300, 200)–(300, 300) bulges to the left of the way it was drawn
    // (downward, so to the right on the page): its middle point is at x = 350.
    assert!(sp.anchors.get(3).unwrap().p.distance(pt(350.0, 250.0)) < 1e-9);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(only_path(&s).anchors.len(), 3);
}

/// C flips the last sharp-corner arc, and [ and ] make it 15° smaller or larger, each as one
/// undo step; the next arc keeps the new size. C is not taken while the last piece is a line.
#[test]
fn c_flips_and_brackets_size_the_last_sharp_arc() {
    let mut s = session();
    quick_pen(&mut s);
    s.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "action", "value": "sharpArc"})).unwrap();
    click(&mut s, 100.0, 300.0);
    click(&mut s, 300.0, 300.0);
    assert!(only_path(&s).anchors.get(1).unwrap().p.distance(pt(200.0, 200.0)) < 1e-9, "above the chord");
    assert!(s.tool_claims_key(ToolKey::Letter('c'), view()));
    let undo = undo_steps(&s);
    key(&mut s, ToolKey::Letter('c'));
    assert!(only_path(&s).anchors.get(1).unwrap().p.distance(pt(200.0, 400.0)) < 1e-9, "below once flipped");
    assert_eq!(undo_steps(&s), undo + 1);
    key(&mut s, ToolKey::BracketLeft);
    assert_eq!(s.tool_options()["turn"], 165.0);
    let sp = only_path(&s);
    // A 165° arc is two segments; its middle point lies on the chord's perpendicular bisector,
    // still below.
    assert_eq!(sp.anchors.len(), 3);
    let mid = sp.anchors.get(1).unwrap().p;
    assert!((mid.x - 200.0).abs() < 1e-9 && mid.y > 300.0);
    assert_eq!(undo_steps(&s), undo + 2);
    s.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "action", "value": "line"})).unwrap();
    click(&mut s, 400.0, 300.0);
    assert!(!s.tool_claims_key(ToolKey::Letter('c'), view()));
}

/// The drawing end (where the Quick Pen toolbar sits beside) is the path's last point with the
/// direction it arrives in: none before the first click, no direction for one point, and none
/// once Escape ends the path.
#[test]
fn the_drawing_end_is_the_last_point_and_its_direction() {
    let mut s = session();
    quick_pen(&mut s);
    assert_eq!(s.tool_drawing_end(view()), None);
    click(&mut s, 100.0, 100.0);
    assert_eq!(s.tool_drawing_end(view()), Some((pt(100.0, 100.0), None)));
    click(&mut s, 100.0, 200.0);
    let (p, dir) = s.tool_drawing_end(view()).unwrap();
    assert_eq!(p, pt(100.0, 200.0));
    let dir = dir.unwrap();
    assert!(dir.x.abs() < 1e-9 && dir.y > 0.99, "arrives heading down: {dir:?}");
    key(&mut s, ToolKey::Escape);
    assert_eq!(s.tool_drawing_end(view()), None);
}

/// Alt tapped during a drag draws the piece with the next action: a continuous arc becomes a
/// sharp-corner arc with a corner where it starts, and the path goes on drawing after it.
#[test]
fn an_alt_tap_during_a_drag_switches_the_piece_and_drawing_goes_on() {
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    s.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "action", "value": "continuousArc"})).unwrap();
    // A quarter circle that arrives at (300, 200) heading down.
    click(&mut s, 300.0, 200.0);
    let joint = only_path(&s).anchors.len() - 1;
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 300.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 210.0, 310.0), v).unwrap();
    assert_eq!(only_path(&s).anchors.get(joint).unwrap().kind, vectorcraft_geom::AnchorKind::Smooth);
    key(&mut s, ToolKey::AltTap);
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 300.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 200.0, 300.0), v).unwrap();
    assert_eq!(s.tool_options()["action"], "sharpArc");
    assert_eq!(only_path(&s).anchors.get(joint).unwrap().kind, vectorcraft_geom::AnchorKind::Corner);
    click(&mut s, 100.0, 300.0);
    assert_eq!(only_path(&s).anchors.last().unwrap().p, pt(100.0, 300.0), "the same path");
}

/// A click on the first point closes the path; the next click starts a new one.
#[test]
fn a_click_on_the_first_point_closes_the_path() {
    let mut s = session();
    quick_pen(&mut s);
    for (x, y) in [(100.0, 100.0), (200.0, 100.0), (200.0, 200.0), (101.0, 101.0)] {
        click(&mut s, x, y);
    }
    let sp = only_path(&s);
    assert!(sp.closed);
    assert_eq!(sp.anchors.len(), 3);
    click(&mut s, 400.0, 400.0);
    assert_eq!(paths(&s).len(), 2);
}

/// Escape ends the path: the next click starts a new path. A press dragged places the piece
/// where it is let go.
#[test]
fn escape_ends_the_path_and_a_drag_places_the_piece() {
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    press(&mut s, &[(150.0, 100.0), (180.0, 120.0), (250.0, 150.0)]);
    assert_eq!(only_path(&s).anchors.last().unwrap().p, pt(250.0, 150.0));
    assert!(s.tool_ends_on_escape(view()));
    key(&mut s, ToolKey::Escape);
    assert!(!s.tool_ends_on_escape(view()));
    click(&mut s, 500.0, 500.0);
    assert_eq!(paths(&s).len(), 2);
}

/// The piece a press draws ends where the press is let go, past the last drag too, and a press let
/// go a drag's distance away with no drag in between places it there as well.
#[test]
fn the_piece_ends_where_the_press_is_let_go() {
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 150.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 180.0, 120.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 250.0, 150.0), v).unwrap();
    assert_eq!(only_path(&s).anchors.last().unwrap().p, pt(250.0, 150.0));
    let undo = undo_steps(&s);
    s.pointer(&PointerEvent::new(PointerKind::Down, 300.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 340.0, 100.0), v).unwrap();
    assert_eq!(only_path(&s).anchors.last().unwrap().p, pt(340.0, 100.0));
    assert_eq!(undo_steps(&s), undo + 1);
}

/// C and `[` `]` during a drag change the sharp-corner arc being dragged, within the press's one
/// undo step.
#[test]
fn c_and_brackets_during_a_drag_change_the_arc_being_dragged() {
    let mut s = session();
    quick_pen(&mut s);
    s.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "action", "value": "sharpArc"})).unwrap();
    click(&mut s, 100.0, 300.0);
    let undo = undo_steps(&s);
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 280.0, 300.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 300.0, 300.0), v).unwrap();
    assert!(s.tool_claims_key(ToolKey::Letter('c'), view()));
    key(&mut s, ToolKey::Letter('c'));
    assert!(only_path(&s).anchors.get(1).unwrap().p.distance(pt(200.0, 400.0)) < 1e-9, "flipped while dragging");
    key(&mut s, ToolKey::BracketLeft);
    assert_eq!(s.tool_options()["turn"], 165.0);
    // A 165° arc is two segments, and its middle point lies the arc's sagitta below the chord
    // (a 180° arc's lies 100 below).
    let half = 82.5f64.to_radians();
    let mid_165 = pt(200.0, 300.0 + 100.0 / half.sin() * (1.0 - half.cos()));
    assert_eq!(only_path(&s).anchors.len(), 3);
    assert!(only_path(&s).anchors.get(1).unwrap().p.distance(mid_165) < 1e-6, "the dragged arc is 165°");
    s.pointer(&PointerEvent::new(PointerKind::Up, 300.0, 300.0), v).unwrap();
    let sp = only_path(&s);
    assert_eq!(sp.anchors.len(), 3);
    assert!(sp.anchors.get(1).unwrap().p.distance(mid_165) < 1e-6, "the flip and the size stay after the release");
    assert_eq!(undo_steps(&s), undo + 1);
}

/// An edit made by another command during a drag ends the path: the next drag does not draw over
/// the edit, the piece stays as last shown, and the next click starts a new path.
#[test]
fn an_edit_during_a_drag_ends_the_path() {
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let id = paths(&s)[0].0;
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 180.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 200.0), v).unwrap();
    s.execute("path.moveAnchor", &json!({"id": id.0, "anchor": 0, "x": 90, "y": 90})).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 250.0, 250.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 260.0, 260.0), v).unwrap();
    let ends: Vec<Point> = only_path(&s).anchors.iter().map(|a| a.p).collect();
    assert_eq!(ends, [pt(90.0, 90.0), pt(200.0, 100.0), pt(200.0, 200.0)], "the edit and the piece as last shown");
    click(&mut s, 400.0, 400.0);
    assert_eq!(paths(&s).len(), 2, "a new path");
}

/// With Real-time Drawing off, a sharp-corner arc let go past the last drag is placed where it was
/// let go, and C then flips that arc.
#[test]
fn a_deferred_arc_let_go_past_the_last_drag_flips() {
    let mut s = session();
    s.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": false})).unwrap();
    quick_pen(&mut s);
    s.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "action", "value": "sharpArc"})).unwrap();
    click(&mut s, 100.0, 300.0);
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 250.0, 300.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 270.0, 300.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 300.0, 300.0), v).unwrap();
    let sp = only_path(&s);
    assert_eq!(sp.anchors.len(), 3);
    assert_eq!(sp.anchors.last().unwrap().p, pt(300.0, 300.0));
    let mid = sp.anchors.get(1).unwrap().p;
    key(&mut s, ToolKey::Letter('c'));
    let flipped = only_path(&s);
    assert_eq!(flipped.anchors.len(), 3);
    assert!(flipped.anchors.get(1).unwrap().p.distance(pt(200.0, 600.0 - mid.y)) < 1e-9, "flipped across the chord");
}

/// An edit made by another command to the path's look during a drag ends the path too.
#[test]
fn a_look_edit_during_a_drag_ends_the_path() {
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let id = paths(&s)[0].0;
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 180.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 200.0), v).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [id.0], "color": "#00ff00"})).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 260.0, 260.0), v).unwrap();
    let ends: Vec<Point> = only_path(&s).anchors.iter().map(|a| a.p).collect();
    assert_eq!(ends, [pt(100.0, 100.0), pt(200.0, 100.0), pt(200.0, 200.0)], "the piece as last shown");
    click(&mut s, 400.0, 400.0);
    assert_eq!(paths(&s).len(), 2, "a new path");
}

/// An edit made by another command to another object during a drag stays and ends the path: the
/// press previews no more, its piece stays as last shown, and the next click starts a new path.
#[test]
fn an_edit_to_another_object_during_a_drag_stays() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 500, "y": 400, "width": 50, "height": 50})).unwrap();
    let rect = s.doc().unwrap().selection.objects[0];
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 180.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 200.0), v).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [rect.0], "color": "#00ff00"})).unwrap();
    let edited = s.doc().unwrap().doc.node(rect).unwrap().clone();
    s.pointer(&PointerEvent::new(PointerKind::Up, 260.0, 260.0), v).unwrap();
    assert_eq!(*s.doc().unwrap().doc.node(rect).unwrap(), edited, "the other object's edit stays");
    let pen = paths(&s).into_iter().find(|(id, _)| *id != rect).unwrap().1;
    let ends: Vec<Point> = pen.subpaths[0].anchors.iter().map(|a| a.p).collect();
    assert_eq!(ends, [pt(100.0, 100.0), pt(200.0, 100.0), pt(200.0, 200.0)], "the piece as last shown");
    click(&mut s, 400.0, 400.0);
    assert_eq!(paths(&s).len(), 3, "a new path");
}

/// With Real-time Drawing off, a drag during which another command edits the path adds no piece,
/// and the edit stays.
#[test]
fn an_edit_during_a_deferred_drag_stays() {
    let mut s = session();
    s.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": false})).unwrap();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let id = paths(&s)[0].0;
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 180.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 200.0), v).unwrap();
    s.execute("path.moveAnchor", &json!({"id": id.0, "anchor": 0, "x": 90, "y": 90})).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 250.0, 250.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 260.0, 260.0), v).unwrap();
    let ends: Vec<Point> = only_path(&s).anchors.iter().map(|a| a.p).collect();
    assert_eq!(ends, [pt(90.0, 90.0), pt(200.0, 100.0)], "the edit, and no piece");
}

/// An edit made by another command to another object during a click stays, and the path goes on:
/// a click previews its piece only when the press begins.
#[test]
fn an_edit_to_another_object_during_a_click_stays() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 500, "y": 400, "width": 50, "height": 50})).unwrap();
    let rect = s.doc().unwrap().selection.objects[0];
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 200.0), v).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [rect.0], "color": "#00ff00"})).unwrap();
    let edited = s.doc().unwrap().doc.node(rect).unwrap().clone();
    s.pointer(&PointerEvent::new(PointerKind::Up, 200.0, 200.0), v).unwrap();
    assert_eq!(*s.doc().unwrap().doc.node(rect).unwrap(), edited, "the other object's edit stays");
    click(&mut s, 300.0, 200.0);
    let all = paths(&s);
    assert_eq!(all.len(), 2, "no new path");
    let pen = all.into_iter().find(|(id, _)| *id != rect).unwrap().1;
    let ends: Vec<Point> = pen.subpaths[0].anchors.iter().map(|a| a.p).collect();
    assert_eq!(ends, [pt(100.0, 100.0), pt(200.0, 100.0), pt(200.0, 200.0), pt(300.0, 200.0)]);
}

/// An edit made by another command during a click (a press let go where it went down) ends the
/// path too: the next click starts a new path instead of drawing over the edit.
#[test]
fn an_edit_during_a_click_ends_the_path() {
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let id = paths(&s)[0].0;
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 180.0), v).unwrap();
    s.execute("path.moveAnchor", &json!({"id": id.0, "anchor": 0, "x": 90, "y": 90})).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 200.0, 180.0), v).unwrap();
    click(&mut s, 400.0, 400.0);
    assert_eq!(paths(&s).len(), 2, "a new path");
    let ends: Vec<Point> = paths(&s)[0].1.subpaths[0].anchors.iter().map(|a| a.p).collect();
    assert_eq!(ends, [pt(90.0, 90.0), pt(200.0, 100.0), pt(200.0, 180.0)], "the edit stays");
}

/// With Align New Objects to Pixel Grid on, the path the tool draws is pixel-aligned: its points
/// move onto the grid, and the next click and drag still go on drawing the same path.
#[test]
fn a_pixel_aligned_path_goes_on_drawing() {
    let mut s = session();
    s.execute("document.setup", &json!({"alignNewObjectsToPixelGrid": true})).unwrap();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let ends = |s: &Session| only_path(s).anchors.iter().map(|a| a.p).collect::<Vec<Point>>();
    let two = ends(&s);
    assert_eq!(two.len(), 2);
    assert_eq!(two[0].y, two[1].y, "a level line");
    press(&mut s, &[(300.0, 150.0), (310.0, 170.0), (320.0, 200.0)]);
    let three = ends(&s);
    assert_eq!(three.len(), 3, "one path of three points");
    assert_eq!(three[..2], two[..], "the points before the piece stay as they were");
    assert!(three[2].distance(pt(320.0, 200.0)) <= 1.0, "the drag's piece ends near where it was let go: {:?}", three[2]);
    click(&mut s, 400.0, 100.0);
    let four = ends(&s);
    assert_eq!(four.len(), 4);
    assert_eq!(four[..3], three[..]);
}

/// A dropped press takes back only its own piece, and with Real-time Drawing off a drag places its
/// piece on the release: in both cases the next click goes on drawing the same path.
#[test]
fn a_dropped_press_and_a_deferred_drag_keep_the_path() {
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 300.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 320.0, 150.0), v).unwrap();
    s.drop_tool_press(v).unwrap();
    assert_eq!(only_path(&s).anchors.len(), 2, "the dropped piece is gone");
    click(&mut s, 300.0, 200.0);
    assert_eq!(only_path(&s).anchors.len(), 3, "the path goes on");
    s.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": false})).unwrap();
    press(&mut s, &[(400.0, 200.0), (420.0, 250.0), (450.0, 300.0)]);
    assert_eq!(only_path(&s).anchors.last().unwrap().p, pt(450.0, 300.0));
    click(&mut s, 500.0, 100.0);
    assert_eq!(only_path(&s).anchors.len(), 5, "the path goes on after the deferred piece");
}

/// An anchor moved by another command ends the path being drawn: C does not flip the old arc back
/// over the edit, and the next click starts a new path instead of rebuilding the old one.
#[test]
fn an_edit_from_outside_ends_the_path() {
    let mut s = session();
    quick_pen(&mut s);
    s.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "action", "value": "sharpArc"})).unwrap();
    click(&mut s, 100.0, 300.0);
    click(&mut s, 300.0, 300.0);
    let (id, _) = paths(&s)[0].clone();
    s.execute("path.moveAnchor", &json!({"id": id.0, "anchor": 2, "x": 320, "y": 280})).unwrap();
    assert!(!s.tool_claims_key(ToolKey::Letter('c'), view()));
    click(&mut s, 500.0, 500.0);
    let all = paths(&s);
    assert_eq!(all.len(), 2, "a new path");
    assert_eq!(all[0].1.subpaths[0].anchors.last().unwrap().p, pt(320.0, 280.0), "the edit stays");
    // The same for a point of a path the tool went on from, and for the first point of a new path.
    for new in [true, false] {
        let mut s = session();
        let id = if new {
            quick_pen(&mut s);
            click(&mut s, 100.0, 100.0);
            click(&mut s, 200.0, 100.0);
            paths(&s)[0].0
        } else {
            let r = s.execute("shape.line", &json!({"x1": 100, "y1": 100, "x2": 200, "y2": 100})).unwrap();
            let id = NodeId(r["id"].as_u64().unwrap());
            s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
            quick_pen(&mut s);
            click(&mut s, 200.0, 100.0);
            click(&mut s, 200.0, 200.0);
            id
        };
        s.execute("path.moveAnchor", &json!({"id": id.0, "anchor": 0, "x": 90, "y": 90})).unwrap();
        click(&mut s, 400.0, 400.0);
        assert_eq!(paths(&s).len(), 2, "a new path (new: {new})");
        assert_eq!(s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0].anchors.first().unwrap().p, pt(90.0, 90.0));
    }
}

/// A click on an end of an open path goes on drawing from it, in the same path; from its start,
/// the path is reversed first.
#[test]
fn a_click_on_an_open_end_continues_the_path() {
    let mut s = session();
    let r = s.execute("shape.line", &json!({"x1": 100, "y1": 100, "x2": 200, "y2": 100})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 100.0, 200.0);
    let all = paths(&s);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0, id);
    let ends: Vec<Point> = all[0].1.subpaths[0].anchors.iter().map(|a| a.p).collect();
    assert_eq!(ends, [pt(200.0, 100.0), pt(100.0, 100.0), pt(100.0, 200.0)]);
}

/// `path.quickPen` makes a new path with the current paint, or a black stroke without any.
#[test]
fn path_quick_pen_makes_a_stroked_path() {
    let mut s = session();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    let r = s
        .execute(
            "path.quickPen",
            &json!({"start": [0, 0], "pieces": [{"x": 100, "y": 0}, {"x": 100, "y": 100, "action": "sharpArc", "turn": 90, "flip": true}]}),
        )
        .unwrap();
    let id = r["id"].as_u64().unwrap();
    let n = s.doc().unwrap().doc.node(NodeId(id)).unwrap().clone();
    assert!(!n.appearance.stroke_paint().is_none(), "a black stroke");
    assert_eq!(n.path_data().unwrap().subpaths[0].anchors.len(), 3);
}

/// Bad params are refused and change nothing.
#[test]
fn path_quick_pen_refuses_bad_params() {
    let mut s = session();
    let r = s.execute("path.quickPen", &json!({"start": [0, 0], "pieces": [{"x": 100, "y": 0}]})).unwrap();
    let id = r["id"].as_u64().unwrap();
    let before = paths(&s);
    let bad: [Value; 11] = [
        json!({}),
        json!({"pieces": []}),
        json!({"start": [0, 0], "pieces": [{"x": 1, "y": 1, "action": "arc"}]}),
        json!({"start": [0, 0], "pieces": [{"x": 1, "y": 1, "action": "sharpArc", "turn": 0}]}),
        json!({"start": [0, 0], "pieces": [{"x": 1, "y": 1, "turn": 360}]}),
        json!({"start": [0, 0], "pieces": [{"x": "1", "y": 1}]}),
        json!({"start": [0, 0], "pieces": [{"x": 1, "y": 1, "flip": "yes"}]}),
        json!({"start": [0, 0], "keep": 1}),
        json!({"id": id, "keep": 0}),
        json!({"id": id, "keep": 3}),
        json!({"id": id, "start": [0, 0]}),
    ];
    for p in bad {
        assert!(s.execute("path.quickPen", &p).is_err(), "{p}");
    }
    assert!(s.execute("path.quickPen", &json!({"start": [0, 0], "pieces": vec![json!({"x": 1, "y": 1}); 10_001]})).is_err());
    s.execute("path.quickPen", &json!({"id": id, "closed": true})).unwrap();
    assert!(s.execute("path.quickPen", &json!({"id": id, "pieces": [{"x": 5, "y": 5}]})).is_err(), "closed");
    assert_eq!(paths(&s).len(), before.len());
}

/// With `id` and `keep`, the points after the first `keep` are replaced by the pieces.
#[test]
fn path_quick_pen_replaces_the_points_after_keep() {
    let mut s = session();
    let r = s.execute("path.quickPen", &json!({"start": [0, 0], "pieces": [{"x": 100, "y": 0}, {"x": 100, "y": 100}]})).unwrap();
    let id = r["id"].as_u64().unwrap();
    s.execute("path.quickPen", &json!({"id": id, "keep": 2, "pieces": [{"x": 200, "y": 0}]})).unwrap();
    let ends: Vec<Point> = only_path(&s).anchors.iter().map(|a| a.p).collect();
    assert_eq!(ends, [pt(0.0, 0.0), pt(100.0, 0.0), pt(200.0, 0.0)]);
}
