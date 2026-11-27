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

/// Whether the tool shows only the path's points and its length handles (a handle and the line to
/// it from the path's end): no hover preview, label or Smart Guide.
fn only_points(s: &mut Session, v: ViewInfo) -> bool {
    let o = s.overlays(v);
    let handles: Vec<Point> = o.iter().filter_map(|o| if let Overlay::Handle { p, .. } = o { Some(*p) } else { None }).collect();
    o.iter().all(|o| match o {
        Overlay::CurvePoint { .. } | Overlay::Handle { .. } => true,
        Overlay::Line { b, .. } => handles.contains(b),
        _ => false,
    })
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
    assert!(!only_points(&mut s, on));
    assert!(only_points(&mut s, off), "only the path's points and length handles show");
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
        // With the last path selected, a press on its anchors would edit it instead of drawing.
        s.execute("select.set", &json!({"ids": []})).unwrap();
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

/// The two arms of a cross centered on `c` among the overlays.
fn crosses_at(o: &[Overlay], c: Point) -> usize {
    o.iter().filter(|o| matches!(o, Overlay::Line { a, b, .. } if a.midpoint(*b).distance(c) < 1e-9)).count()
}

/// While a half circle from (200, 100) to (300, 100) is dragged, or is the piece the next click
/// adds, the tool shows a cross at its center, (250, 100); while a line is dragged, it shows none.
#[test]
fn an_arc_shows_a_cross_at_its_center() {
    let mut s = session();
    quick_pen(&mut s);
    let center = pt(250.0, 100.0);
    for (action, arms) in [("line", 0), ("sharpArc", 2)] {
        key(&mut s, ToolKey::Escape);
        s.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "action", "value": "line"})).unwrap();
        click(&mut s, 100.0, 100.0);
        click(&mut s, 200.0, 100.0);
        s.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "action", "value": action})).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Move, 300.0, 100.0), view()).unwrap();
        assert_eq!(crosses_at(&s.overlays(view()), center), arms, "hovering, {action}");
        s.pointer(&PointerEvent::new(PointerKind::Down, 280.0, 150.0), view()).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Drag, 300.0, 100.0), view()).unwrap();
        assert_eq!(crosses_at(&s.overlays(view()), center), arms, "dragging, {action}");
        s.pointer(&PointerEvent::new(PointerKind::Up, 300.0, 100.0), view()).unwrap();
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

/// The handle beyond the end of the path being drawn makes its last line longer, as one undo step,
/// and the next click goes on from the new end.
#[test]
fn dragging_the_end_handle_lengthens_the_last_line() {
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let undo = undo_steps(&s);
    // The handle stands 20 screen points beyond the end, along the line.
    press(&mut s, &[(220.0, 100.0), (250.0, 130.0), (280.0, 140.0)]);
    let ends: Vec<Point> = only_path(&s).anchors.iter().map(|a| a.p).collect();
    assert_eq!(ends, [pt(100.0, 100.0), pt(260.0, 100.0)]);
    assert_eq!(undo_steps(&s), undo + 1);
    click(&mut s, 260.0, 200.0);
    let ends: Vec<Point> = only_path(&s).anchors.iter().map(|a| a.p).collect();
    assert_eq!(ends, [pt(100.0, 100.0), pt(260.0, 100.0), pt(260.0, 200.0)], "the drawing went on");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(only_path(&s).anchors.last().unwrap().p, pt(200.0, 100.0));
}

/// Dragged by its end handle, the last sharp-corner arc goes on around its circle and stays the
/// last piece, so C still flips it and `[` `]` still change its size.
#[test]
fn a_dragged_last_arc_still_flips() {
    let mut s = session();
    quick_pen(&mut s);
    s.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "action", "value": "sharpArc"})).unwrap();
    click(&mut s, 100.0, 300.0);
    click(&mut s, 300.0, 300.0);
    let undo = undo_steps(&s);
    // The half circle over (200, 300) ends heading down, so its end handle is 20 points below. The
    // handle turned a quarter around the center, to (180, 400), takes the end to (200, 400).
    press(&mut s, &[(300.0, 320.0), (260.0, 400.0), (180.0, 400.0)]);
    let sp = only_path(&s);
    assert!(sp.anchors.last().unwrap().p.distance(pt(200.0, 400.0)) < 1e-6, "three quarters around: {:?}", sp.anchors.last());
    assert_eq!(sp.anchors.len(), 4);
    assert_eq!(undo_steps(&s), undo + 1);
    assert!(s.tool_claims_key(ToolKey::Letter('c'), view()));
    key(&mut s, ToolKey::Letter('c'));
    let flipped = only_path(&s);
    assert_ne!(flipped, sp);
    assert!(flipped.anchors.last().unwrap().p.distance(pt(200.0, 400.0)) < 1e-6);
    assert_eq!(undo_steps(&s), undo + 2);
    // `]` makes the dragged arc 15° larger: it keeps its ends and gains its own undo step.
    key(&mut s, ToolKey::BracketRight);
    let larger = only_path(&s);
    assert_ne!(larger, flipped);
    assert!(larger.anchors.last().unwrap().p.distance(pt(200.0, 400.0)) < 1e-6);
    assert_eq!(undo_steps(&s), undo + 3);
}

/// Dragged back past its start, the last sharp-corner arc stops at 1°.
#[test]
fn a_dragged_last_arc_keeps_at_least_one_degree() {
    let mut s = session();
    quick_pen(&mut s);
    s.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "action", "value": "sharpArc"})).unwrap();
    click(&mut s, 100.0, 300.0);
    click(&mut s, 300.0, 300.0);
    // The half circle about (200, 300) runs over the top from (100, 300) to (300, 300); its end
    // handle is 20 points below the end. The pointer goes back around the circle from there, past
    // the start of the arc.
    let (c, r) = (pt(200.0, 300.0), (100.0f64 * 100.0 + 20.0 * 20.0).sqrt());
    let a0 = 20.0f64.atan2(100.0);
    let mut pts = vec![(300.0, 320.0)];
    for d in [45.0f64, 90.0, 135.0, 175.0, 185.0] {
        let a = a0 - d.to_radians();
        pts.push((c.x + r * a.cos(), c.y + r * a.sin()));
    }
    press(&mut s, &pts);
    let sp = only_path(&s);
    let one = 181f64.to_radians();
    let end = pt(200.0 + 100.0 * one.cos(), 300.0 + 100.0 * one.sin());
    assert!(sp.anchors.last().unwrap().p.distance(end) < 1e-6, "{:?}", sp.anchors.last());
    assert_eq!(sp.anchors.first().unwrap().p, pt(100.0, 300.0));
}

/// When the last arc goes on around the circle of the arc before it, its end handle still changes
/// the last arc alone, which C then still flips.
#[test]
fn the_end_handle_changes_the_last_arc_on_a_shared_circle() {
    let mut s = session();
    quick_pen(&mut s);
    s.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "action", "value": "sharpArc"})).unwrap();
    click(&mut s, 100.0, 300.0);
    click(&mut s, 300.0, 300.0);
    s.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "turn", "value": 90})).unwrap();
    // A quarter more around the circle about (200, 300), ending heading left.
    click(&mut s, 200.0, 400.0);
    let r = 100.0 * std::f64::consts::FRAC_1_SQRT_2;
    let to = pt(200.0 - r, 300.0 + r);
    // The handle at (180, 400) turned 45° more around the center takes the end there with it.
    let (sin, cos) = std::f64::consts::FRAC_PI_4.sin_cos();
    let (hx, hy) = (-20.0, 100.0);
    let pointer = (200.0 + hx * cos - hy * sin, 300.0 + hx * sin + hy * cos);
    press(&mut s, &[(180.0, 400.0), (170.0, 400.0), pointer]);
    let sp = only_path(&s);
    assert!(sp.anchors.last().unwrap().p.distance(to) < 1e-6, "135° from (300, 300): {:?}", sp.anchors.last());
    assert!(sp.anchors.get(2).unwrap().p.distance(pt(300.0, 300.0)) < 1e-9, "the first arc stays");
    assert!(s.tool_claims_key(ToolKey::Letter('c'), view()));
}

/// Once another path is selected while one was being drawn, the selected path shows its handles.
#[test]
fn another_selected_path_shows_its_handles() {
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let r = s.execute("shape.line", &json!({"x1": 300, "y1": 300, "x2": 400, "y2": 300})).unwrap();
    s.execute("select.set", &json!({"ids": [r["id"]]})).unwrap();
    let handle =
        |s: &mut Session| s.overlays(view()).iter().any(|o| matches!(o, vectorcraft_tools::Overlay::Handle { p, .. } if *p == pt(420.0, 300.0)));
    assert!(handle(&mut s));
    assert_eq!(s.cursor(pt(420.0, 300.0), Mods::default(), view()), vectorcraft_tools::Cursor::Move);
}

/// A selected path that is not being drawn shows its handles: the one before its start makes the
/// first arc shorter around its circle. Escape during a drag takes the drag back.
#[test]
fn the_start_handle_of_a_selected_path_shortens_its_arc() {
    let mut s = session();
    let r = s
        .execute("path.quickPen", &json!({"start": [100, 100], "pieces": [{"x": 200, "y": 100, "action": "sharpArc"}, {"x": 300, "y": 100}]}))
        .unwrap();
    s.execute("select.set", &json!({"ids": [r["id"]]})).unwrap();
    quick_pen(&mut s);
    let undo = undo_steps(&s);
    let before = only_path(&s);
    // The half circle leaves (100, 100) heading up, so its start handle is 20 points below.
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 120.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 150.0, 70.0), v).unwrap();
    assert_ne!(only_path(&s), before, "previewed");
    key(&mut s, ToolKey::Escape);
    s.pointer(&PointerEvent::new(PointerKind::Up, 150.0, 70.0), v).unwrap();
    assert_eq!(only_path(&s), before, "taken back");
    assert_eq!(undo_steps(&s), undo);
    // The handle turned a quarter around the center, to (130, 50), takes the start to the top of
    // the circle: the arc is a quarter from there to (200, 100).
    press(&mut s, &[(100.0, 120.0), (130.0, 50.0)]);
    let sp = only_path(&s);
    assert!(sp.anchors.first().unwrap().p.distance(pt(150.0, 50.0)) < 1e-6, "{:?}", sp.anchors.first());
    assert_eq!(sp.anchors.len(), 3);
    assert_eq!(undo_steps(&s), undo + 1);
}

/// A click on a length handle that does not drag is an ordinary click: while drawing it adds a
/// point there, and with no path being drawn it starts a new path.
#[test]
fn a_click_on_a_handle_adds_a_point() {
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let undo = undo_steps(&s);
    click(&mut s, 220.0, 100.0);
    let ends: Vec<Point> = only_path(&s).anchors.iter().map(|a| a.p).collect();
    assert_eq!(ends, [pt(100.0, 100.0), pt(200.0, 100.0), pt(220.0, 100.0)]);
    assert_eq!(undo_steps(&s), undo + 1);
    // After Escape the path stays selected with its handles; a click on one starts a new path.
    key(&mut s, ToolKey::Escape);
    assert_eq!(s.cursor(pt(240.0, 100.0), Mods::default(), view()), vectorcraft_tools::Cursor::Move);
    click(&mut s, 240.0, 100.0);
    assert_eq!(paths(&s).len(), 2);
}

/// During a length drag the tool shows the handle and no preview of a next piece to where the
/// pointer was before the press.
#[test]
fn a_length_drag_shows_no_next_piece() {
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Move, 220.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 220.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 280.0, 100.0), v).unwrap();
    let o = s.overlays(v);
    assert!(o.iter().any(|o| matches!(o, vectorcraft_tools::Overlay::Handle { .. })));
    assert!(!o.iter().any(|o| matches!(o, vectorcraft_tools::Overlay::Path { .. } | vectorcraft_tools::Overlay::Measure { .. })), "{o:?}");
    s.pointer(&PointerEvent::new(PointerKind::Up, 280.0, 100.0), v).unwrap();
    assert_eq!(only_path(&s).anchors.last().unwrap().p, pt(260.0, 100.0));
}

/// A press on a length handle clears the Smart Guides of the hover before it, so none show during
/// the drag.
#[test]
fn a_length_drag_shows_no_smart_guide() {
    let mut s = session();
    s.execute("shape.line", &json!({"x1": 220, "y1": 100, "x2": 220, "y2": 300})).unwrap();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let on = ViewInfo { smart_guides: true, ..Default::default() };
    // The end handle stands on the other line's anchor, so the hover there shows its guide.
    s.pointer(&PointerEvent::new(PointerKind::Move, 220.0, 100.0), on).unwrap();
    assert!(!only_points(&mut s, on), "the anchor's guide shows");
    s.pointer(&PointerEvent::new(PointerKind::Down, 220.0, 100.0), on).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 280.0, 140.0), on).unwrap();
    assert!(only_points(&mut s, on), "no guide during the drag: {:?}", s.overlays(on));
    s.pointer(&PointerEvent::new(PointerKind::Up, 280.0, 140.0), on).unwrap();
    let drawn = paths(&s).into_iter().find(|(_, p)| p.subpaths[0].anchors.first().is_some_and(|a| a.p == pt(100.0, 100.0))).unwrap();
    assert_eq!(drawn.1.subpaths[0].anchors.last().unwrap().p, pt(260.0, 100.0));
}

/// A length drag that would take the path past the canvas leaves it where the drag last put it,
/// and the drawing goes on from there.
#[test]
fn a_length_drag_stays_on_the_canvas() {
    let mut s = session();
    quick_pen(&mut s);
    let x = vectorcraft_doc::MAX_COORD - 1000.0;
    click(&mut s, x, 100.0);
    click(&mut s, x + 500.0, 100.0);
    let undo = undo_steps(&s);
    let before = only_path(&s);
    // The end handle is at x + 520; a drag to x + 2000 would put the end at x + 1980, past the canvas.
    press(&mut s, &[(x + 520.0, 100.0), (x + 2000.0, 100.0)]);
    assert_eq!(only_path(&s), before);
    assert_eq!(undo_steps(&s), undo);
    press(&mut s, &[(x + 520.0, 100.0), (x + 820.0, 100.0)]);
    assert_eq!(only_path(&s).anchors.last().unwrap().p, pt(x + 800.0, 100.0));
    assert_eq!(undo_steps(&s), undo + 1);
    click(&mut s, x + 800.0, 200.0);
    assert_eq!(only_path(&s).anchors.len(), 3, "the drawing went on");
}

/// A length drag across a line, or out along it and back, leaves its length as it was: it changes
/// nothing and records no undo step, on the path being drawn and on a selected path.
#[test]
fn a_length_drag_that_keeps_the_length_records_no_undo_step() {
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let undo = undo_steps(&s);
    let before = only_path(&s);
    // The end handle is at (220, 100); the first drag goes straight down from it, the second out
    // along the line and back.
    for drag in [[(220.0, 100.0), (220.0, 160.0), (220.0, 160.0)], [(220.0, 100.0), (260.0, 100.0), (220.0, 100.0)]] {
        press(&mut s, &drag);
        assert_eq!(only_path(&s), before, "{drag:?}");
        assert_eq!(undo_steps(&s), undo, "{drag:?}");
    }
    key(&mut s, ToolKey::Escape);
    let undo = undo_steps(&s);
    for drag in [[(220.0, 100.0), (220.0, 160.0), (220.0, 160.0)], [(220.0, 100.0), (260.0, 100.0), (220.0, 100.0)]] {
        press(&mut s, &drag);
        assert_eq!(only_path(&s), before, "{drag:?}");
        assert_eq!(undo_steps(&s), undo, "{drag:?}");
    }
}

/// The paths of a selected compound path show length handles, also after a drag selected the
/// compound again.
#[test]
fn a_compound_path_shows_the_length_handles_of_its_paths() {
    let mut s = session();
    let a = s.execute("path.quickPen", &json!({"start": [100, 100], "pieces": [{"x": 200, "y": 100}]})).unwrap();
    let b = s.execute("path.quickPen", &json!({"start": [100, 300], "pieces": [{"x": 200, "y": 300}]})).unwrap();
    s.execute("select.set", &json!({"ids": [a["id"], b["id"]]})).unwrap();
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    quick_pen(&mut s);
    press(&mut s, &[(220.0, 100.0), (250.0, 100.0), (280.0, 100.0)]);
    press(&mut s, &[(220.0, 300.0), (250.0, 300.0), (270.0, 300.0)]);
    let mut ends: Vec<Point> = paths(&s).iter().filter_map(|(_, pd)| pd.subpaths[0].anchors.last().map(|a| a.p)).collect();
    ends.sort_by(|p, q| p.y.total_cmp(&q.y));
    assert_eq!(ends, [pt(260.0, 100.0), pt(250.0, 300.0)]);
}

/// An edit made outside the tool during a length press ends the press: a later drag does not
/// preview from the path as the press found it, and so does not undo the edit.
#[test]
fn an_outside_edit_during_a_length_press_is_kept() {
    // The edit comes before the first drag event, or after the drag began.
    for began in [false, true] {
        let mut s = session();
        quick_pen(&mut s);
        click(&mut s, 100.0, 100.0);
        click(&mut s, 200.0, 100.0);
        let v = view();
        let id = paths(&s)[0].0.0;
        s.pointer(&PointerEvent::new(PointerKind::Down, 220.0, 100.0), v).unwrap();
        if began {
            s.pointer(&PointerEvent::new(PointerKind::Drag, 250.0, 100.0), v).unwrap();
        }
        s.execute("path.quickPenLength", &json!({"id": id, "length": 40})).unwrap();
        let edited = only_path(&s);
        s.pointer(&PointerEvent::new(PointerKind::Drag, 280.0, 100.0), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, 280.0, 100.0), v).unwrap();
        assert_eq!(only_path(&s), edited, "began: {began}");
    }
    // Escape after the edit takes the drag back without taking the edit back.
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let v = view();
    let id = paths(&s)[0].0.0;
    s.pointer(&PointerEvent::new(PointerKind::Down, 220.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 250.0, 100.0), v).unwrap();
    s.execute("path.quickPenLength", &json!({"id": id, "length": 40})).unwrap();
    let edited = only_path(&s);
    key(&mut s, ToolKey::Escape);
    s.pointer(&PointerEvent::new(PointerKind::Up, 250.0, 100.0), v).unwrap();
    assert_eq!(only_path(&s), edited, "escape");
}

/// C and `[` `]` wait for a length drag to end: they neither add an undo step of their own nor
/// change the arc the drag is changing.
#[test]
fn piece_keys_do_nothing_during_a_length_drag() {
    let mut s = session();
    quick_pen(&mut s);
    s.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "action", "value": "sharpArc"})).unwrap();
    click(&mut s, 100.0, 300.0);
    click(&mut s, 300.0, 300.0);
    let undo = undo_steps(&s);
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 300.0, 320.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 260.0, 400.0), v).unwrap();
    let during = only_path(&s);
    for k in [ToolKey::Letter('c'), ToolKey::BracketRight, ToolKey::BracketLeft] {
        assert!(s.tool_claims_key(k, view()), "{k:?} stays with the tool");
        key(&mut s, k);
        assert_eq!(only_path(&s), during, "{k:?}");
    }
    s.pointer(&PointerEvent::new(PointerKind::Drag, 180.0, 400.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 180.0, 400.0), v).unwrap();
    assert_eq!(undo_steps(&s), undo + 1);
    assert!(only_path(&s).anchors.last().unwrap().p.distance(pt(200.0, 400.0)) < 1e-6);
}

/// Choosing Quick Pen again during a length drag leaves the drawing alone: the drag goes on, ends
/// where the pointer is let go, and the next click goes on drawing the same path.
#[test]
fn choosing_the_tool_again_during_a_length_drag_keeps_the_drawing() {
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 220.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 250.0, 130.0), v).unwrap();
    quick_pen(&mut s);
    s.pointer(&PointerEvent::new(PointerKind::Drag, 280.0, 140.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 280.0, 140.0), v).unwrap();
    let ends: Vec<Point> = only_path(&s).anchors.iter().map(|a| a.p).collect();
    assert_eq!(ends, [pt(100.0, 100.0), pt(260.0, 100.0)]);
    click(&mut s, 260.0, 200.0);
    assert_eq!(paths(&s).len(), 1, "the drawing went on");
    assert_eq!(only_path(&s).anchors.len(), 3);
}

/// A length drag ends where the pointer is let go, also when no drag event came after the press.
#[test]
fn a_length_drag_ends_at_the_release_point() {
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let undo = undo_steps(&s);
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 220.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 250.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 280.0, 100.0), v).unwrap();
    assert_eq!(only_path(&s).anchors.last().unwrap().p, pt(260.0, 100.0));
    assert_eq!(undo_steps(&s), undo + 1);
    // A press let go away from the handle without a drag event drags too.
    s.pointer(&PointerEvent::new(PointerKind::Down, 280.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 320.0, 100.0), v).unwrap();
    assert_eq!(only_path(&s).anchors.last().unwrap().p, pt(300.0, 100.0));
    assert_eq!(only_path(&s).anchors.len(), 2);
    assert_eq!(undo_steps(&s), undo + 2);
}

/// `path.quickPenLength` refuses a result past the canvas, not only one past the input limit.
#[test]
fn path_quick_pen_length_stays_on_the_canvas() {
    let mut s = session();
    let x = vectorcraft_doc::MAX_COORD - 500_000.0;
    let r = s.execute("path.quickPen", &json!({"start": [x, 0], "pieces": [{"x": x + 10.0, "y": 0}]})).unwrap();
    let id = r["id"].as_u64().unwrap();
    let before = only_path(&s);
    let undo = undo_steps(&s);
    let e = s.execute("path.quickPenLength", &json!({"id": id, "length": 1_000_000})).unwrap_err().to_string();
    assert!(e.contains("would exceed the canvas"), "{e}");
    assert_eq!(only_path(&s), before);
    assert_eq!(undo_steps(&s), undo);
    s.execute("path.quickPenLength", &json!({"id": id, "length": 400_000})).unwrap();
    assert_eq!(only_path(&s).anchors.last().unwrap().p, pt(x + 400_000.0, 0.0));
}

/// Dragged by the start handle while the path is being drawn, the first line changes and the last
/// sharp-corner arc stays the last piece, so C still flips it.
#[test]
fn the_start_handle_keeps_the_last_arc_a_piece() {
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 300.0);
    click(&mut s, 200.0, 300.0);
    s.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "action", "value": "sharpArc"})).unwrap();
    click(&mut s, 400.0, 300.0);
    assert!(s.tool_claims_key(ToolKey::Letter('c'), view()));
    let undo = undo_steps(&s);
    // The line leaves (100, 300) heading left, seen from the start, so its handle is 20 points left.
    press(&mut s, &[(80.0, 300.0), (60.0, 300.0), (50.0, 300.0)]);
    let sp = only_path(&s);
    assert_eq!(sp.anchors.first().unwrap().p, pt(70.0, 300.0));
    assert_eq!(undo_steps(&s), undo + 1);
    assert!(s.tool_claims_key(ToolKey::Letter('c'), view()), "the last arc is still a piece");
    key(&mut s, ToolKey::Letter('c'));
    let flipped = only_path(&s);
    assert_ne!(flipped, sp);
    assert_eq!(flipped.anchors.first().unwrap().p, pt(70.0, 300.0));
    assert!(flipped.anchors.last().unwrap().p.distance(pt(400.0, 300.0)) < 1e-9);
}

/// `path.quickPenLength` sets the length of a line or an arc at either end; the same length
/// changes nothing and records no undo step; bad params are refused.
#[test]
fn path_quick_pen_length_sets_an_end() {
    let mut s = session();
    let r = s.execute("path.quickPen", &json!({"start": [0, 0], "pieces": [{"x": 100, "y": 0}, {"x": 200, "y": 0, "action": "sharpArc"}]})).unwrap();
    let id = r["id"].as_u64().unwrap();
    let r = s.execute("path.quickPenLength", &json!({"id": id, "end": "start", "length": 40})).unwrap();
    assert_eq!((r["kind"].as_str(), r["length"].as_f64()), (Some("line"), Some(40.0)));
    assert_eq!(only_path(&s).anchors.first().unwrap().p, pt(60.0, 0.0));
    let undo = undo_steps(&s);
    let r = s.execute("path.quickPenLength", &json!({"id": id, "length": std::f64::consts::PI * 50.0})).unwrap();
    assert_eq!(r["kind"], "arc");
    assert_eq!(undo_steps(&s), undo, "unchanged");
    s.execute("path.quickPenLength", &json!({"id": id, "length": std::f64::consts::PI * 25.0})).unwrap();
    assert!(only_path(&s).anchors.last().unwrap().p.distance(pt(150.0, -50.0)) < 1e-9);
    assert_eq!(undo_steps(&s), undo + 1);
    let before = paths(&s);
    let bad: [Value; 7] = [
        json!({"length": 10}),
        json!({"id": id}),
        json!({"id": id, "length": "10"}),
        json!({"id": id, "length": 0}),
        json!({"id": id, "length": 1e7, "end": "start"}),
        json!({"id": id, "length": 10, "end": "middle"}),
        json!({"id": id, "length": 10, "subpath": 1}),
    ];
    for p in bad {
        assert!(s.execute("path.quickPenLength", &p).is_err(), "{p}");
    }
    let curve = s.execute("shape.line", &json!({"x1": 0, "y1": 300, "x2": 100, "y2": 300})).unwrap();
    s.execute("path.convertAnchor", &json!({"id": curve["id"], "anchor": 1, "to": "smooth", "x": 150, "y": 360})).unwrap();
    assert!(s.execute("path.quickPenLength", &json!({"id": curve["id"], "length": 10})).is_err(), "not a line or an arc");
    s.execute("path.quickPen", &json!({"id": id, "closed": true})).unwrap();
    assert!(s.execute("path.quickPenLength", &json!({"id": id, "length": 10})).is_err(), "closed");
    assert_eq!(paths(&s).len(), before.len() + 1);
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

/// A selected path drawn with `path.quickPen` from `start` through `pieces`, with the Quick Pen
/// chosen and no path being drawn.
fn selected_quick_pen_path(s: &mut Session, start: (f64, f64), pieces: Value) -> NodeId {
    let r = s.execute("path.quickPen", &json!({"start": [start.0, start.1], "pieces": pieces})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    quick_pen(s);
    id
}

fn points(s: &Session) -> Vec<Point> {
    only_path(s).anchors.iter().map(|a| a.p).collect()
}

fn shift_click(s: &mut Session, x: f64, y: f64) {
    let v = view();
    let mods = Mods { shift: true, ..Mods::default() };
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent { mods, ..PointerEvent::new(kind, x, y) }, v).unwrap();
    }
}

/// Is segment `i` of `sp` a circular arc: do its points a quarter and three quarters along lie on
/// the circle through its ends and its middle?
fn segment_is_circular(sp: &SubPath, i: usize) -> bool {
    use vectorcraft_geom::ParamCurve;
    let seg = sp.segment(i);
    let (a, b, c) = (seg.eval(0.0), seg.eval(0.5), seg.eval(1.0));
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    let sq = |p: Point| p.x * p.x + p.y * p.y;
    let center = pt(
        (sq(a) * (b.y - c.y) + sq(b) * (c.y - a.y) + sq(c) * (a.y - b.y)) / d,
        (sq(a) * (c.x - b.x) + sq(b) * (a.x - c.x) + sq(c) * (b.x - a.x)) / d,
    );
    let r = center.distance(a);
    [0.25, 0.75].iter().all(|&t| (seg.eval(t).distance(center) - r).abs() < r * 1e-3)
}

/// With no path being drawn, a drag on an anchor of the selected path moves that anchor alone, in
/// one undo step: at the corner between a line and an arc, the line stays a line and the arc a
/// circular arc.
#[test]
fn a_drag_on_an_anchor_moves_it_and_the_arc_stays_an_arc() {
    let mut s = session();
    selected_quick_pen_path(&mut s, (100.0, 100.0), json!([{"x": 200, "y": 100}, {"x": 300, "y": 100, "action": "sharpArc", "turn": 90}]));
    assert_eq!(points(&s), [pt(100.0, 100.0), pt(200.0, 100.0), pt(300.0, 100.0)]);
    let undo = undo_steps(&s);
    press(&mut s, &[(200.0, 100.0), (200.0, 80.0), (200.0, 50.0)]);
    assert_eq!(undo_steps(&s), undo + 1);
    let sp = only_path(&s);
    assert_eq!(sp.anchors.iter().map(|a| a.p).collect::<Vec<_>>(), [pt(100.0, 100.0), pt(200.0, 50.0), pt(300.0, 100.0)]);
    assert!(sp.segment_is_line(0), "{:?}", sp.anchors);
    assert!(!sp.segment_is_line(1) && segment_is_circular(&sp, 1), "{:?}", sp.anchors);
    assert_eq!(paths(&s).len(), 1, "no new path");
}

/// A drag on a segment of the selected path moves that segment's two ends.
#[test]
fn a_drag_on_a_segment_moves_its_ends() {
    let mut s = session();
    selected_quick_pen_path(&mut s, (100.0, 300.0), json!([{"x": 300, "y": 300}, {"x": 300, "y": 400}]));
    press(&mut s, &[(200.0, 300.0), (200.0, 280.0), (200.0, 250.0)]);
    assert_eq!(points(&s), [pt(100.0, 250.0), pt(300.0, 250.0), pt(300.0, 400.0)]);
    assert_eq!(paths(&s).len(), 1);
}

/// A drag that starts on an end of the selected open path moves that end; a click there still
/// goes on drawing from it.
#[test]
fn a_drag_on_an_open_end_moves_it() {
    let mut s = session();
    let id = selected_quick_pen_path(&mut s, (100.0, 300.0), json!([{"x": 300, "y": 300}]));
    press(&mut s, &[(300.0, 300.0), (320.0, 320.0), (350.0, 350.0)]);
    assert_eq!(points(&s), [pt(100.0, 300.0), pt(350.0, 350.0)]);
    assert!(s.doc().unwrap().selection.partial(id).is_some(), "the dragged end is selected");
    click(&mut s, 350.0, 350.0);
    assert!(s.doc().unwrap().selection.partial(id).is_none(), "drawing goes on with the path selected whole");
    click(&mut s, 350.0, 450.0);
    assert_eq!(points(&s), [pt(100.0, 300.0), pt(350.0, 350.0), pt(350.0, 450.0)]);
}

/// A press let go past the drag distance moves what it holds even when no drag event went that
/// far: an inner anchor moves, and an open end moves instead of going on drawing.
#[test]
fn a_release_past_the_drag_distance_moves_the_anchor() {
    let mut s = session();
    selected_quick_pen_path(&mut s, (100.0, 100.0), json!([{"x": 200, "y": 100}, {"x": 300, "y": 100}]));
    let undo = undo_steps(&s);
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 102.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 200.0, 150.0), v).unwrap();
    assert_eq!(points(&s), [pt(100.0, 100.0), pt(200.0, 150.0), pt(300.0, 100.0)]);
    assert_eq!(undo_steps(&s), undo + 1);
    s.pointer(&PointerEvent::new(PointerKind::Down, 300.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 300.0, 200.0), v).unwrap();
    assert_eq!(points(&s), [pt(100.0, 100.0), pt(200.0, 150.0), pt(300.0, 200.0)]);
    assert_eq!(undo_steps(&s), undo + 2);
    assert_eq!(paths(&s).len(), 1, "no new path");
}

/// A drag on an anchor or a segment that comes back to where it started moves nothing and records
/// no undo step.
#[test]
fn a_drag_back_to_its_start_records_no_undo_step() {
    let mut s = session();
    selected_quick_pen_path(&mut s, (100.0, 100.0), json!([{"x": 200, "y": 100}, {"x": 300, "y": 100}]));
    let undo = undo_steps(&s);
    for at in [(200.0, 100.0), (250.0, 100.0)] {
        press(&mut s, &[at, (at.0, at.1 + 50.0), at]);
        assert_eq!(points(&s), [pt(100.0, 100.0), pt(200.0, 100.0), pt(300.0, 100.0)], "press at {at:?}");
        assert_eq!(undo_steps(&s), undo, "press at {at:?}");
    }
    // Choosing another tool during such a drag records none either.
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 150.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 100.0), v).unwrap();
    s.select_tool("selection", v).unwrap();
    assert_eq!(points(&s), [pt(100.0, 100.0), pt(200.0, 100.0), pt(300.0, 100.0)]);
    assert_eq!(undo_steps(&s), undo);
}

/// When another command changes the selection while the press is down, neither a click nor a drag
/// selects or moves the anchor the press found.
#[test]
fn a_selection_change_during_an_edit_press_stops_it() {
    let mut s = session();
    let id = selected_quick_pen_path(&mut s, (100.0, 100.0), json!([{"x": 200, "y": 100}, {"x": 300, "y": 100}]));
    let v = view();
    for drag in [false, true] {
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        let undo = undo_steps(&s);
        s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 100.0), v).unwrap();
        s.execute("select.none", &json!({})).unwrap();
        let y = if drag { 150.0 } else { 100.0 };
        if drag {
            s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, y), v).unwrap();
        }
        s.pointer(&PointerEvent::new(PointerKind::Up, 200.0, y), v).unwrap();
        assert!(s.doc().unwrap().selection.objects.is_empty(), "drag: {drag}");
        assert_eq!(points(&s), [pt(100.0, 100.0), pt(200.0, 100.0), pt(300.0, 100.0)], "drag: {drag}");
        assert_eq!(undo_steps(&s), undo, "drag: {drag}");
    }
}

/// When another command edits the path while the press is down, neither a drag nor a release past
/// the drag distance moves the anchor the press found.
#[test]
fn an_edit_during_an_edit_press_stops_the_move() {
    let mut s = session();
    let id = selected_quick_pen_path(&mut s, (100.0, 100.0), json!([{"x": 200, "y": 100}, {"x": 300, "y": 100}]));
    let v = view();
    for (drag, x) in [(true, 90.0), (false, 80.0)] {
        s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 100.0), v).unwrap();
        s.execute("path.moveAnchor", &json!({"id": id.0, "anchor": 0, "x": x, "y": 90})).unwrap();
        let undo = undo_steps(&s);
        if drag {
            s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 150.0), v).unwrap();
        }
        s.pointer(&PointerEvent::new(PointerKind::Up, 200.0, 150.0), v).unwrap();
        assert_eq!(points(&s), [pt(x, 90.0), pt(200.0, 100.0), pt(300.0, 100.0)], "drag: {drag}");
        assert_eq!(undo_steps(&s), undo, "drag: {drag}");
    }
}

/// An edit made by another command during an anchor drag stays: the rest of the drag and the
/// release move nothing more, and one undo step holds the move as last shown with the edit.
#[test]
fn an_edit_during_an_anchor_drag_stays() {
    let mut s = session();
    let id = selected_quick_pen_path(&mut s, (100.0, 100.0), json!([{"x": 200, "y": 100}, {"x": 300, "y": 100}]));
    let undo = undo_steps(&s);
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 150.0), v).unwrap();
    s.execute("path.moveAnchor", &json!({"id": id.0, "anchor": 0, "x": 90, "y": 90})).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 200.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 200.0, 210.0), v).unwrap();
    assert_eq!(points(&s), [pt(90.0, 90.0), pt(200.0, 150.0), pt(300.0, 100.0)], "the edit and the move as last shown");
    assert_eq!(undo_steps(&s), undo + 1);
    assert!(!s.in_interaction(), "the step is committed");
    // Escape after such an edit keeps it too.
    s.pointer(&PointerEvent::new(PointerKind::Down, 300.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 300.0, 150.0), v).unwrap();
    s.execute("path.moveAnchor", &json!({"id": id.0, "anchor": 0, "x": 80, "y": 80})).unwrap();
    key(&mut s, ToolKey::Escape);
    s.pointer(&PointerEvent::new(PointerKind::Up, 300.0, 200.0), v).unwrap();
    assert_eq!(points(&s), [pt(80.0, 80.0), pt(200.0, 150.0), pt(300.0, 150.0)]);
    assert!(!s.in_interaction(), "the step is committed");
}

/// When another command edits the path while a click is down, the click neither selects the
/// anchor nor goes on drawing from the open end it found.
#[test]
fn an_edit_during_an_edit_click_does_nothing() {
    let mut s = session();
    let id = selected_quick_pen_path(&mut s, (100.0, 100.0), json!([{"x": 200, "y": 100}, {"x": 300, "y": 100}]));
    let v = view();
    for (x, moved_y) in [(200.0, 90.0), (300.0, 80.0)] {
        s.pointer(&PointerEvent::new(PointerKind::Down, x, 100.0), v).unwrap();
        s.execute("path.moveAnchor", &json!({"id": id.0, "anchor": 0, "x": 100, "y": moved_y})).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, x, 100.0), v).unwrap();
        assert!(s.doc().unwrap().selection.partial(id).is_none(), "no anchor selected after a click at x = {x}");
    }
    // The open end was not taken up, so the next click starts a new path.
    click(&mut s, 400.0, 200.0);
    assert_eq!(paths(&s).len(), 2);
}

/// A click on an anchor selects it alone, Shift-click adds another, and Delete (`edit.clear`, the
/// context menu's Delete) then removes both.
#[test]
fn clicks_select_anchors_for_delete() {
    let mut s = session();
    let id = selected_quick_pen_path(&mut s, (100.0, 100.0), json!([{"x": 200, "y": 100}, {"x": 300, "y": 100}, {"x": 400, "y": 100}]));
    let undo = undo_steps(&s);
    click(&mut s, 200.0, 100.0);
    let picked = |s: &Session| s.doc().unwrap().selection.partial(id).cloned().unwrap_or_default().into_iter().collect::<Vec<_>>();
    assert_eq!(picked(&s), [(0, 1)]);
    shift_click(&mut s, 300.0, 100.0);
    assert_eq!(picked(&s), [(0, 1), (0, 2)]);
    shift_click(&mut s, 300.0, 100.0);
    assert_eq!(picked(&s), [(0, 1)], "Shift-click takes it out again");
    shift_click(&mut s, 300.0, 100.0);
    assert_eq!(undo_steps(&s), undo, "selecting is no edit");
    s.execute("edit.clear", &json!({})).unwrap();
    let left: Vec<Point> = paths(&s).iter().flat_map(|(_, pd)| pd.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p))).collect();
    assert!(!left.contains(&pt(200.0, 100.0)) && !left.contains(&pt(300.0, 100.0)), "{left:?}");
}

/// The first Shift-click on an anchor of a path selected whole selects that anchor alone.
#[test]
fn a_first_shift_click_selects_the_anchor_alone() {
    let mut s = session();
    let id = selected_quick_pen_path(&mut s, (100.0, 100.0), json!([{"x": 200, "y": 100}, {"x": 300, "y": 100}]));
    assert!(s.doc().unwrap().selection.partial(id).is_none(), "the path is selected whole");
    shift_click(&mut s, 200.0, 100.0);
    let picked = s.doc().unwrap().selection.partial(id).cloned().unwrap_or_default().into_iter().collect::<Vec<_>>();
    assert_eq!(picked, [(0, 1)]);
}

/// A drag on a segment moves its two ends alone, also when they are selected with another anchor.
#[test]
fn a_segment_drag_moves_only_its_ends() {
    let mut s = session();
    let pieces = json!([{"x": 300, "y": 300}, {"x": 300, "y": 400}, {"x": 100, "y": 400}, {"x": 100, "y": 500}]);
    let id = selected_quick_pen_path(&mut s, (100.0, 300.0), pieces);
    click(&mut s, 300.0, 300.0);
    shift_click(&mut s, 300.0, 400.0);
    shift_click(&mut s, 100.0, 400.0);
    let picked = s.doc().unwrap().selection.partial(id).cloned().unwrap_or_default().into_iter().collect::<Vec<_>>();
    assert_eq!(picked, [(0, 1), (0, 2), (0, 3)]);
    press(&mut s, &[(300.0, 350.0), (320.0, 350.0), (350.0, 350.0)]);
    assert_eq!(points(&s), [pt(100.0, 300.0), pt(350.0, 300.0), pt(350.0, 400.0), pt(100.0, 400.0), pt(100.0, 500.0)]);
    assert_eq!(paths(&s).len(), 1);
}

/// A drag on a segment whose two ends are already selected as anchors selects the segment, so
/// Delete then cuts that connection and keeps both anchors.
#[test]
fn a_segment_drag_from_its_selected_ends_selects_the_segment() {
    let mut s = session();
    let pieces = json!([{"x": 200, "y": 100}, {"x": 300, "y": 100}, {"x": 400, "y": 100}]);
    let id = selected_quick_pen_path(&mut s, (100.0, 100.0), pieces);
    click(&mut s, 200.0, 100.0);
    shift_click(&mut s, 300.0, 100.0);
    let picked = s.doc().unwrap().selection.partial(id).cloned().unwrap_or_default().into_iter().collect::<Vec<_>>();
    assert_eq!(picked, [(0, 1), (0, 2)]);
    press(&mut s, &[(250.0, 100.0), (250.0, 120.0), (250.0, 150.0)]);
    assert_eq!(points(&s), [pt(100.0, 100.0), pt(200.0, 150.0), pt(300.0, 150.0), pt(400.0, 100.0)]);
    let segments = s.doc().unwrap().selection.segments.get(&id).cloned().unwrap_or_default().into_iter().collect::<Vec<_>>();
    assert_eq!(segments, [(0, 1)], "the segment is selected");
    s.execute("edit.clear", &json!({})).unwrap();
    let left: Vec<Point> = paths(&s).iter().flat_map(|(_, pd)| pd.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p))).collect();
    for p in [pt(100.0, 100.0), pt(200.0, 150.0), pt(300.0, 150.0), pt(400.0, 100.0)] {
        assert!(left.contains(&p), "{p:?} stays: {left:?}");
    }
}

/// A click on an open end of another path, in front of the selected path, that lies on a segment
/// of the selected path goes on drawing from that end.
#[test]
fn an_open_end_on_a_selected_segment_continues_its_path() {
    let mut s = session();
    let id = selected_quick_pen_path(&mut s, (100.0, 300.0), json!([{"x": 300, "y": 300}]));
    let r = s.execute("shape.line", &json!({"x1": 200, "y1": 200, "x2": 200, "y2": 300})).unwrap();
    let other = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    click(&mut s, 200.0, 300.0);
    click(&mut s, 250.0, 250.0);
    let all = paths(&s);
    assert_eq!(all.len(), 2, "no new path");
    let ends: Vec<Point> = all.iter().find(|(id, _)| *id == other).unwrap().1.subpaths[0].anchors.iter().map(|a| a.p).collect();
    assert_eq!(ends, [pt(200.0, 200.0), pt(200.0, 300.0), pt(250.0, 250.0)]);
}

/// Escape during a drag puts the anchor back, and the release then changes nothing.
#[test]
fn escape_takes_back_an_anchor_drag() {
    let mut s = session();
    selected_quick_pen_path(&mut s, (100.0, 100.0), json!([{"x": 200, "y": 100}, {"x": 300, "y": 100}]));
    let (before, undo) = (points(&s), undo_steps(&s));
    let v = view();
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 150.0), v).unwrap();
    assert_ne!(points(&s), before, "the drag previews");
    key(&mut s, ToolKey::Escape);
    s.pointer(&PointerEvent::new(PointerKind::Up, 200.0, 150.0), v).unwrap();
    assert_eq!(points(&s), before);
    assert_eq!(undo_steps(&s), undo);
    assert_eq!(paths(&s).len(), 1);
}

/// While a path is being drawn, a press on one of its points adds a piece as before; the editing
/// starts only after Escape ends the path.
#[test]
fn a_press_while_drawing_adds_a_piece() {
    let mut s = session();
    quick_pen(&mut s);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    click(&mut s, 200.0, 200.0);
    press(&mut s, &[(200.0, 100.0), (220.0, 90.0), (250.0, 80.0)]);
    assert_eq!(points(&s).len(), 4, "a piece added");
    key(&mut s, ToolKey::Escape);
    press(&mut s, &[(200.0, 100.0), (200.0, 70.0), (200.0, 50.0)]);
    let got = points(&s);
    assert_eq!(got.len(), 4);
    assert!(got[1].distance(pt(200.0, 50.0)) < 1e-9, "{got:?}");
}
