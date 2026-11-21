//! The Quick Pen tool driven through the session, and `path.quickPen`.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{PathData, Point, SubPath};
use vectorcraft_tools::{Mods, Overlay, PointerEvent, PointerKind, ToolKey};

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
    press_in(s, pts, view());
}

fn press_in(s: &mut Session, pts: &[(f64, f64)], v: ViewInfo) {
    press_with(s, pts, v, Mods::default());
}

fn press_with(s: &mut Session, pts: &[(f64, f64)], v: ViewInfo, m: Mods) {
    s.pointer(&PointerEvent::new(PointerKind::Down, pts[0].0, pts[0].1).with_mods(m), v).unwrap();
    for &(x, y) in &pts[1..] {
        s.pointer(&PointerEvent::new(PointerKind::Drag, x, y).with_mods(m), v).unwrap();
    }
    let l = pts[pts.len() - 1];
    s.pointer(&PointerEvent::new(PointerKind::Up, l.0, l.1).with_mods(m), v).unwrap();
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

/// With Smart Guides on, a click near another path's anchor puts the point on that anchor; with
/// them off, the point stays where the click was.
#[test]
fn a_point_snaps_to_a_smart_guide() {
    for (guides, want) in [(true, pt(300.0, 300.0)), (false, pt(302.0, 301.0))] {
        let mut s = session();
        s.execute("shape.line", &json!({"x1": 300, "y1": 300, "x2": 500, "y2": 450})).unwrap();
        s.execute("select.set", &json!({"ids": []})).unwrap();
        quick_pen(&mut s);
        let v = ViewInfo { smart_guides: guides, ..Default::default() };
        press_in(&mut s, &[(100.0, 100.0)], v);
        press_in(&mut s, &[(302.0, 301.0)], v);
        let drawn = paths(&s).into_iter().find(|(_, p)| p.subpaths[0].anchors.first().is_some_and(|a| a.p == pt(100.0, 100.0))).unwrap();
        assert!(drawn.1.subpaths[0].anchors.last().unwrap().p.distance(want) < 1e-9, "{guides}");
    }
}

/// A point 100 pt from (200, 100) and 30.5° below the +x direction a path from (100, 100) ends in:
/// a continuous arc to it sweeps 61°.
fn arc_end_61() -> Point {
    let half = 30.5f64.to_radians();
    pt(200.0 + 100.0 * half.cos(), 100.0 + 100.0 * half.sin())
}

/// The protractor's 60° tick for the arc to [`arc_end_61`]: on the same circle, a 60° sweep.
fn tick_60() -> Point {
    let r = 50.0 / 30.5f64.to_radians().sin();
    pt(200.0 + r * 30f64.to_radians().cos(), 100.0 + r * 30f64.to_radians().sin())
}

/// A path from (100, 100) to (200, 100) drawn with Quick Pen, the action then a continuous arc.
fn continuous_arc_after_a_line(s: &mut Session, v: ViewInfo) {
    quick_pen(s);
    press_in(s, &[(100.0, 100.0)], v);
    press_in(s, &[(200.0, 100.0)], v);
    key(s, ToolKey::AltTap);
}

/// The last point of the path the tool drew (any other path is a two-point line).
fn drawn_end(s: &Session) -> Point {
    let drawn = paths(s).into_iter().find(|(_, p)| p.subpaths[0].anchors.len() >= 3).unwrap();
    drawn.1.subpaths[0].anchors.last().unwrap().p
}

/// A continuous arc dragged to end 61° round goes onto the protractor's 60° tick with Smart Guides
/// on, also with Shift held, which does not constrain Quick Pen points; with them off it ends at
/// the pointer. A Smart Guide in reach (another path's anchor) wins over the tick, also with its
/// label hidden, and so do Snap to Grid and Snap to Pixel.
#[test]
fn a_continuous_arc_end_snaps_to_the_protractor() {
    let at = arc_end_61();
    let anchor = pt(at.x + 0.2, at.y + 0.15);
    let fresh = session();
    let g = &fresh.doc().unwrap().doc.grid;
    let step = g.spacing / g.subdivisions.max(1) as f64;
    let on_grid = pt((at.x / step).round() * step, (at.y / step).round() * step);
    let on_pixel = pt(at.x.round(), at.y.round());
    let shift = Mods { shift: true, ..Default::default() };
    // (Smart Guides, the other path, Snap to Grid, Snap to Pixel, Anchor/Path Labels, mods, where
    // the end goes, the pointer)
    let cases = [
        (true, false, false, false, true, Mods::default(), tick_60(), at),
        (true, false, false, false, true, shift, tick_60(), at),
        (false, false, false, false, true, Mods::default(), at, at),
        (true, true, false, false, true, Mods::default(), anchor, at),
        (true, false, true, false, true, Mods::default(), on_grid, on_grid),
        (true, false, false, true, true, Mods::default(), on_pixel, on_pixel),
        // Exactly on the anchor, which shows no guide with the labels off.
        (true, true, false, false, false, Mods::default(), anchor, anchor),
    ];
    for (guides, other, grid, pixel, labels, m, want, pointer) in cases {
        let mut s = session();
        s.prefs.anchor_path_labels = labels;
        if other {
            s.execute("shape.line", &json!({"x1": anchor.x, "y1": anchor.y, "x2": 600, "y2": anchor.y})).unwrap();
            s.execute("select.set", &json!({"ids": []})).unwrap();
        }
        let v = ViewInfo { smart_guides: guides, snap_to_grid: grid, snap_to_pixel: pixel, ..Default::default() };
        continuous_arc_after_a_line(&mut s, v);
        press_with(&mut s, &[(280.0, 140.0), (pointer.x, pointer.y)], v, m);
        let end = drawn_end(&s);
        assert!(end.distance(want) < 1e-6, "{guides} {other} {grid} {pixel} {labels} {m:?}: {end:?}");
    }
}

/// With Smart Guides on, a click without a drag puts a continuous arc's end on the protractor's
/// tick; after Alt switches to a sharp-corner arc, a click at the same place puts the end at the
/// pointer.
#[test]
fn a_click_puts_a_continuous_arc_end_on_a_tick() {
    let at = arc_end_61();
    let v = ViewInfo::default();
    for (taps, want) in [(0, tick_60()), (1, at)] {
        let mut s = session();
        continuous_arc_after_a_line(&mut s, v);
        for _ in 0..taps {
            key(&mut s, ToolKey::AltTap);
        }
        press_in(&mut s, &[(at.x, at.y)], v);
        let end = drawn_end(&s);
        assert!(end.distance(want) < 1e-6, "{taps}: {end:?}");
    }
}

/// Alt tapped during a press moves the end of the piece for the new action at once, with no
/// further drag: a continuous arc's end on the 60° tick goes to the pointer once the piece is a
/// sharp-corner arc, after a drag and after a press that has not dragged, and a line's end at the
/// pointer goes onto the tick once the piece is a continuous arc.
#[test]
fn an_alt_tap_during_a_press_moves_the_end_for_the_new_action() {
    let at = arc_end_61();
    let v = ViewInfo::default();
    // (drag first, Alt taps before the press, the end before Alt, the end after it)
    for (drag, taps, before, after) in [(true, 0, tick_60(), at), (false, 0, tick_60(), at), (true, 2, at, tick_60())] {
        let mut s = session();
        continuous_arc_after_a_line(&mut s, v);
        for _ in 0..taps {
            key(&mut s, ToolKey::AltTap);
        }
        let first = if drag { (280.0, 140.0) } else { (at.x, at.y) };
        s.pointer(&PointerEvent::new(PointerKind::Down, first.0, first.1), v).unwrap();
        if drag {
            s.pointer(&PointerEvent::new(PointerKind::Drag, at.x, at.y), v).unwrap();
        }
        assert!(drawn_end(&s).distance(before) < 1e-6, "{drag} {taps}: {:?}", drawn_end(&s));
        s.tool_key(ToolKey::AltTap, Mods::default(), v).unwrap();
        assert!(drawn_end(&s).distance(after) < 1e-6, "{drag} {taps} after Alt: {:?}", drawn_end(&s));
        s.pointer(&PointerEvent::new(PointerKind::Up, at.x, at.y), v).unwrap();
        assert!(drawn_end(&s).distance(after) < 1e-6, "{drag} {taps} placed: {:?}", drawn_end(&s));
    }
}

/// After a click puts a continuous arc's end on a tick away from the pointer, no preview runs from
/// the placed end back to where the pointer was; the next move shows the preview again.
#[test]
fn a_click_on_a_tick_leaves_no_preview_back_to_the_pointer() {
    let at = arc_end_61();
    let v = ViewInfo::default();
    let mut s = session();
    continuous_arc_after_a_line(&mut s, v);
    s.pointer(&PointerEvent::new(PointerKind::Move, at.x, at.y), v).unwrap();
    press_in(&mut s, &[(at.x, at.y)], v);
    assert!(drawn_end(&s).distance(tick_60()) < 1e-6);
    let label = |s: &mut Session| s.overlays(v).iter().any(|o| matches!(o, Overlay::Measure { .. }));
    assert!(!label(&mut s), "no distance label back to the pointer");
    s.pointer(&PointerEvent::new(PointerKind::Move, 400.0, 300.0), v).unwrap();
    assert!(label(&mut s));
}

/// Hovering near a tick as a continuous arc puts the distance label beside the tick; after Alt
/// switches the action, the same hover puts it beside the pointer.
#[test]
fn a_hover_snaps_to_a_tick_only_for_a_continuous_arc() {
    let at = arc_end_61();
    let tick = tick_60();
    let v = ViewInfo::default();
    let mut s = session();
    continuous_arc_after_a_line(&mut s, v);
    s.pointer(&PointerEvent::new(PointerKind::Move, at.x, at.y), v).unwrap();
    let label = |s: &mut Session| {
        s.overlays(v).iter().find_map(|o| match o {
            Overlay::Measure { p, .. } => Some(*p),
            _ => None,
        })
    };
    let off = label(&mut s).unwrap() - tick;
    assert!(off.x > 0.0 && (off.x - off.y).abs() < 1e-9, "the label sits diagonally off the tick: {off:?}");
    for _ in 0..2 {
        key(&mut s, ToolKey::AltTap);
        assert!((label(&mut s).unwrap() - off).distance(at) < 1e-9, "{:?}", s.tool_options()["action"]);
    }
}

/// A hover made with Smart Guides off shows nothing once they are turned on under the still
/// pointer, so it shows no tick that a click would not use (another path's anchor under the
/// pointer takes the click). The next move shows the hover again, its label beside the anchor;
/// once Smart Guides are turned off again, its guides are hidden with it, also after a click.
#[test]
fn a_hover_shows_nothing_after_smart_guides_change() {
    let at = arc_end_61();
    let off = ViewInfo { smart_guides: false, ..Default::default() };
    let on = ViewInfo::default();
    let mut s = session();
    s.execute("shape.line", &json!({"x1": at.x, "y1": at.y, "x2": 600, "y2": at.y})).unwrap();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    continuous_arc_after_a_line(&mut s, off);
    s.pointer(&PointerEvent::new(PointerKind::Move, at.x, at.y), off).unwrap();
    let label = |s: &mut Session, v: ViewInfo| {
        s.overlays(v).iter().find_map(|o| match o {
            Overlay::Measure { p, .. } => Some(*p),
            _ => None,
        })
    };
    assert!(label(&mut s, off).is_some());
    assert_eq!(label(&mut s, on), None, "the hover from before the change shows nothing");
    s.pointer(&PointerEvent::new(PointerKind::Move, at.x, at.y), on).unwrap();
    let off_anchor = label(&mut s, on).unwrap() - at;
    assert!(off_anchor.x > 0.0 && (off_anchor.x - off_anchor.y).abs() < 1e-9, "the label sits diagonally off the anchor: {off_anchor:?}");
    let only_points = |s: &mut Session, v: ViewInfo| s.overlays(v).iter().all(|o| matches!(o, Overlay::CurvePoint { .. }));
    assert!(!only_points(&mut s, on));
    assert!(only_points(&mut s, off), "only the path's points show");
    // A click before the pointer moves does not bring the stale hover back.
    press_in(&mut s, &[(400.0, 300.0)], off);
    assert_eq!(label(&mut s, off), None, "no label for the stale hover after the click");
}

/// While the pointer hovers, the tool asks the canvas to hover it again when the view, the document
/// or the selection changes under a still pointer, so the snapped point follows the art; a press
/// stops it until the next move.
#[test]
fn a_hover_is_hovered_again_when_the_art_changes() {
    let mut s = session();
    continuous_arc_after_a_line(&mut s, view());
    assert!(!s.tool_wants_rehover());
    s.pointer(&PointerEvent::new(PointerKind::Move, 280.0, 180.0), view()).unwrap();
    assert!(s.tool_wants_rehover());
    s.pointer(&PointerEvent::new(PointerKind::Down, 280.0, 180.0), view()).unwrap();
    assert!(!s.tool_wants_rehover());
    s.pointer(&PointerEvent::new(PointerKind::Up, 280.0, 180.0), view()).unwrap();
    assert!(!s.tool_wants_rehover());
}

/// A hover on the path's last point, which another path's anchor shares, shows that anchor's Smart
/// Guide; after Smart Guides are turned off, a click there adds nothing and the guide stays hidden
/// while the button is held and after it is let go.
#[test]
fn a_click_that_adds_nothing_keeps_a_stale_hover_hidden() {
    let off = ViewInfo { smart_guides: false, ..Default::default() };
    let on = ViewInfo::default();
    let mut s = session();
    s.execute("shape.line", &json!({"x1": 200, "y1": 100, "x2": 200, "y2": 400})).unwrap();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    continuous_arc_after_a_line(&mut s, on);
    s.pointer(&PointerEvent::new(PointerKind::Move, 200.0, 100.0), on).unwrap();
    let only_points = |s: &mut Session, v: ViewInfo| s.overlays(v).iter().all(|o| matches!(o, Overlay::CurvePoint { .. }));
    assert!(!only_points(&mut s, on), "the anchor's guide shows");
    assert!(only_points(&mut s, off));
    let steps = undo_steps(&s);
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 100.0), off).unwrap();
    assert!(only_points(&mut s, off), "the guide stays hidden while the button is held");
    s.pointer(&PointerEvent::new(PointerKind::Up, 200.0, 100.0), off).unwrap();
    assert_eq!(undo_steps(&s), steps, "the click adds nothing");
    assert!(only_points(&mut s, off), "the guide stays hidden");
}

/// While a press drags an arc, the tool shows its protractor; while it drags a line, it shows none.
#[test]
fn an_arc_drag_shows_the_protractor() {
    let mut s = session();
    quick_pen(&mut s);
    let protractors = |s: &mut Session| s.overlays(view()).iter().filter(|o| matches!(o, Overlay::Path { .. })).count();
    for (action, shown) in [("line", 0), ("continuousArc", 1), ("sharpArc", 1)] {
        key(&mut s, ToolKey::Escape);
        s.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "action", "value": "line"})).unwrap();
        click(&mut s, 100.0, 100.0);
        click(&mut s, 200.0, 100.0);
        s.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "action", "value": action})).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Down, 250.0, 150.0), view()).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Drag, 280.0, 180.0), view()).unwrap();
        assert_eq!(protractors(&mut s), shown, "{action}");
        s.pointer(&PointerEvent::new(PointerKind::Up, 280.0, 180.0), view()).unwrap();
    }
}

/// While hovering, the piece the next click adds shows its protractor beside the rubber band, and
/// neither shows with the Pen tool's rubber band preference off; a line has no protractor. (Smart
/// Guides are off, so no guide paths show.)
#[test]
fn a_hovered_arc_shows_the_protractor_with_the_rubber_band() {
    for (rubber_band, action, paths) in [(true, "continuousArc", 2), (true, "line", 1), (false, "continuousArc", 0)] {
        let mut s = session();
        s.prefs.pen_rubber_band = rubber_band;
        quick_pen(&mut s);
        click(&mut s, 100.0, 100.0);
        click(&mut s, 200.0, 100.0);
        s.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "action", "value": action})).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Move, 280.0, 180.0), view()).unwrap();
        let shown = s.overlays(view()).iter().filter(|o| matches!(o, Overlay::Path { .. })).count();
        assert_eq!(shown, paths, "{rubber_band} {action}");
    }
}

/// With Measurement Labels on, hovering after the path's last point shows the distance label
/// beside the pointer; with them off, no label shows.
#[test]
fn hovering_shows_the_distance_label() {
    for labels in [true, false] {
        let mut s = session();
        s.prefs.measurement_labels = labels;
        quick_pen(&mut s);
        click(&mut s, 100.0, 100.0);
        click(&mut s, 200.0, 100.0);
        s.pointer(&PointerEvent::new(PointerKind::Move, 300.0, 200.0), view()).unwrap();
        let o = s.overlays(view());
        let label = o.iter().find_map(|o| match o {
            Overlay::Measure { p, .. } => Some(*p),
            _ => None,
        });
        assert_eq!(label.is_some(), labels);
        if let Some(p) = label {
            assert!(p.x > 300.0 && p.y > 200.0, "beside the pointer: {p:?}");
        }
    }
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
