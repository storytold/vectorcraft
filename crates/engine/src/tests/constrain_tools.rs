//! General › Constrain Angle turns the axes the arrow keys, Shift-constrained drags and the
//! Rectangle, Rounded Rectangle and Ellipse tools work along, and General › Corner Radius is the
//! Rounded Rectangle tool's radius (G12, #394, first batch).

use serde_json::{Value, json};
use vectorcraft_doc::{LiveShape, NodeKind};
use vectorcraft_geom::{Point, Vec2};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::*;

fn new_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn set_pref(s: &mut Session, key: &str, value: Value) {
    s.execute("prefs.set", &json!({"key": key, "value": value})).unwrap();
}

fn rect(s: &mut Session, x: f64, y: f64) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap())
}

fn centre(s: &Session, id: NodeId) -> Point {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().center()
}

fn gesture(s: &mut Session, mods: Mods, events: &[(PointerKind, f64, f64)]) {
    for (k, x, y) in events {
        s.pointer(&PointerEvent::new(*k, *x, *y).with_mods(mods), ViewInfo { smart_guides: false, ..ViewInfo::default() }).unwrap();
    }
}

/// The live rectangle or ellipse `id`: (width, height, rotation in degrees counter-clockwise on
/// the page, corner radius).
fn live_box(s: &Session, id: NodeId) -> (f64, f64, f64, f64) {
    let n = s.doc().unwrap().doc.node(id).unwrap();
    let NodeKind::Path { live: Some(live), .. } = &n.kind else { panic!("a live shape: {:?}", n.kind) };
    let (w, h, xf, r) = match &**live {
        LiveShape::Rectangle { w, h, xf, radii, .. } => (*w, *h, *xf, radii[0]),
        LiveShape::Ellipse { w, h, xf, .. } => (*w, *h, *xf, 0.0),
        other => panic!("a rectangle or ellipse: {other:?}"),
    };
    let [a, b, ..] = xf.as_coeffs();
    (w, h, -b.atan2(a).to_degrees(), r)
}

fn last_id(s: &Session) -> NodeId {
    *s.doc().unwrap().selection.objects.first().unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// General › Constrain Angle turns the arrow keys' axes: at 30°, → moves the art 30° up from the
/// horizontal and ↓ 30° round from straight down; ruler guides keep to the page's axes.
#[test]
fn constrain_angle_turns_the_arrow_keys() {
    let mut s = new_doc();
    let id = rect(&mut s, 100.0, 100.0);
    set_pref(&mut s, "keyboardIncrement", json!(10));
    let at = centre(&s, id);
    s.execute("object.nudge", &json!({"dx": 1, "dy": 0})).unwrap();
    assert!(close(centre(&s, id).x - at.x, 10.0) && close(centre(&s, id).y, at.y), "0°: straight right");
    set_pref(&mut s, "constrainAngle", json!(30));
    let at = centre(&s, id);
    s.execute("object.nudge", &json!({"dx": 1, "dy": 0})).unwrap();
    let d = centre(&s, id) - at;
    assert!(close(d.x, 10.0 * 30f64.to_radians().cos()) && close(d.y, -5.0), "→ at 30°: {d:?}");
    let at = centre(&s, id);
    s.execute("object.nudge", &json!({"dx": 0, "dy": 1})).unwrap();
    let d = centre(&s, id) - at;
    assert!(close(d.x, 5.0) && close(d.y, 10.0 * 30f64.to_radians().cos()), "↓ at 30°: {d:?}");
    // Ruler guides: along the page.
    let g = s.execute("guide.add", &json!({"vertical": true, "pos": 50})).unwrap()["index"].as_u64().unwrap();
    s.execute("guide.select", &json!({"indexes": [g]})).unwrap();
    s.execute("object.nudge", &json!({"dx": 1, "dy": 0})).unwrap();
    let pos = s.doc().unwrap().doc.guides[g as usize].pos;
    assert!(close(pos, 60.0), "a vertical guide moves 10 right: {pos}");
}

/// General › Constrain Angle holds Shift-drags to the turned axes: a Selection-tool move dragged
/// 2° off the 30° axis lands on it.
#[test]
fn constrain_angle_holds_shift_drags_to_the_turned_axes() {
    use PointerKind::{Down, Drag, Up};
    let mut s = new_doc();
    let id = rect(&mut s, 100.0, 200.0);
    s.select_tool("selection", ViewInfo::default()).unwrap();
    let shift = Mods { shift: true, ..Mods::default() };
    let drag = |s: &mut Session| {
        let at = centre(s, id);
        let a = 32f64.to_radians();
        let to = at + Vec2::new(a.cos(), -a.sin()) * 100.0;
        gesture(s, shift, &[(Down, at.x, at.y), (Drag, at.x + 20.0, at.y - 5.0), (Drag, to.x, to.y), (Up, to.x, to.y)]);
        let d = centre(s, id) - at;
        s.execute("edit.undo", &json!({})).unwrap();
        -d.y.atan2(d.x).to_degrees()
    };
    let a = drag(&mut s);
    assert!(close(a, 45.0), "0°: 45° steps from the page's axes, {a}");
    set_pref(&mut s, "constrainAngle", json!(30));
    let a = drag(&mut s);
    assert!(close(a, 30.0), "30°: the drag keeps to the turned x axis, {a}");
}

/// General › Constrain Angle: rectangles, rounded rectangles and ellipses are drawn along the
/// turned axes (the drag's width and height measured along them).
#[test]
fn constrain_angle_draws_boxes_along_the_turned_axes() {
    use PointerKind::{Down, Drag, Up};
    let mut s = new_doc();
    set_pref(&mut s, "constrainAngle", json!(90));
    for tool in ["rectangle", "ellipse"] {
        s.select_tool(tool, ViewInfo::default()).unwrap();
        // 60 right and 20 down on the page: along axes turned 90°, 20 wide and 60 high.
        gesture(&mut s, Mods::default(), &[(Down, 100.0, 100.0), (Drag, 130.0, 110.0), (Drag, 160.0, 120.0), (Up, 160.0, 120.0)]);
        let id = last_id(&s);
        let (w, h, rot, _) = live_box(&s, id);
        assert!(close(w, 20.0) && close(h, 60.0) && close(rot.rem_euclid(360.0), 90.0), "{tool}: {w} × {h} at {rot}°");
        let b = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
        assert!(close(b.x0, 100.0) && close(b.y0, 100.0) && close(b.x1, 160.0) && close(b.y1, 120.0), "{tool}: covers the drag, {b:?}");
    }
    set_pref(&mut s, "constrainAngle", json!(0));
    s.select_tool("rectangle", ViewInfo::default()).unwrap();
    gesture(&mut s, Mods::default(), &[(Down, 100.0, 200.0), (Drag, 130.0, 210.0), (Drag, 160.0, 220.0), (Up, 160.0, 220.0)]);
    let (w, h, rot, _) = live_box(&s, last_id(&s));
    assert!(close(w, 60.0) && close(h, 20.0) && close(rot, 0.0), "0°: along the page, {w} × {h} at {rot}°");
}

/// General › Constrain Angle of a whole turn (±360°) is no turn: the arrow keys move along
/// the page exactly as at 0°, with no floating-point noise.
#[test]
fn constrain_angle_whole_turns_nudge_as_zero() {
    let mut s = new_doc();
    let id = rect(&mut s, 100.0, 100.0);
    set_pref(&mut s, "keyboardIncrement", json!(10));
    let moved = |s: &mut Session, dx: i32, dy: i32| {
        let at = centre(s, id);
        s.execute("object.nudge", &json!({"dx": dx, "dy": dy})).unwrap();
        centre(s, id) - at
    };
    let flat = [moved(&mut s, 1, 0), moved(&mut s, 0, 1)];
    // The preference takes −360…360.
    for turn in [360, -360] {
        set_pref(&mut s, "constrainAngle", json!(turn));
        assert_eq!([moved(&mut s, 1, 0), moved(&mut s, 0, 1)], flat, "{turn}°");
    }
    // A quarter turn is exact too: → moves straight up.
    set_pref(&mut s, "constrainAngle", json!(90));
    assert_eq!(moved(&mut s, 1, 0), Vec2::new(0.0, -10.0));
}

/// General › Constrain Angle turns only the art's arrow-key axes: ruler guides selected with the
/// art keep to the page's axes. At 90°, → moves the art straight up, a vertical guide 10 right and
/// a horizontal guide not at all; ↓ moves the horizontal guide 10 down. Copies alike.
#[test]
fn constrain_angle_keeps_guides_selected_with_art_on_the_page_axes() {
    let mut s = new_doc();
    let id = rect(&mut s, 100.0, 100.0);
    set_pref(&mut s, "keyboardIncrement", json!(10));
    set_pref(&mut s, "constrainAngle", json!(90));
    let v = s.execute("guide.add", &json!({"vertical": true, "pos": 50})).unwrap()["index"].as_u64().unwrap() as usize;
    let h = s.execute("guide.add", &json!({"vertical": false, "pos": 50})).unwrap()["index"].as_u64().unwrap() as usize;
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("guide.select", &json!({"indexes": [v, h], "toggle": true})).unwrap();
    let pos = |s: &Session| s.doc().unwrap().doc.guides.iter().map(|g| g.pos).collect::<Vec<_>>();
    let at = centre(&s, id);
    s.execute("object.nudge", &json!({"dx": 1, "dy": 0})).unwrap();
    assert_eq!(centre(&s, id) - at, Vec2::new(0.0, -10.0), "the art moves along the turned axis");
    assert_eq!(pos(&s), [60.0, 50.0], "→: the vertical guide moves right, the horizontal one stays");
    s.execute("object.nudge", &json!({"dx": 0, "dy": 1})).unwrap();
    assert_eq!(pos(&s), [60.0, 60.0], "↓: the horizontal guide moves down");
    // With a copy, the guides' copies keep to the page too.
    s.execute("object.nudge", &json!({"dx": 1, "dy": 0, "copy": true})).unwrap();
    assert_eq!(pos(&s), [60.0, 60.0, 70.0, 60.0]);
}

/// General › Corner Radius is the Rounded Rectangle tool's radius, a change while the tool is
/// active reaches it, and the radius ↑ sets while drawing one becomes the preference.
#[test]
fn corner_radius_is_the_rounded_rectangle_tools_radius() {
    use PointerKind::{Down, Drag, Up};
    let mut s = new_doc();
    let v = ViewInfo::default();
    set_pref(&mut s, "cornerRadius", json!(7));
    s.select_tool("roundedRectangle", v).unwrap();
    let draw = |s: &mut Session, y: f64, up: bool| {
        gesture(s, Mods::default(), &[(Down, 100.0, y), (Drag, 150.0, y + 30.0), (Drag, 200.0, y + 60.0)]);
        if up {
            s.tool_key(ToolKey::Up, Mods::default(), v).unwrap();
        }
        gesture(s, Mods::default(), &[(Up, 200.0, y + 60.0)]);
        live_box(s, last_id(s)).3
    };
    assert!(close(draw(&mut s, 10.0, false), 7.0), "the preference's radius");
    // Set while the tool is active: the tool takes it.
    set_pref(&mut s, "cornerRadius", json!(4));
    assert!(close(draw(&mut s, 100.0, false), 4.0), "the new preference's radius");
    assert!(close(draw(&mut s, 200.0, true), 5.0), "↑ adds a point");
    assert_eq!(s.execute("prefs.get", &json!({"key": "cornerRadius"})).unwrap(), json!(5.0), "and it is the preference now");
    s.select_tool("selection", v).unwrap();
    s.select_tool("roundedRectangle", v).unwrap();
    assert_eq!(s.tool_options()["cornerRadius"], json!(5.0), "chosen again, the tool keeps it");
    // A tool option set from the Tool Options dialog is the preference too, within its range.
    s.set_tool_option("cornerRadius", &json!(5000));
    assert_eq!(s.execute("prefs.get", &json!({"key": "cornerRadius"})).unwrap(), json!(1296.0));
    assert_eq!(s.tool_options()["cornerRadius"], json!(1296.0), "and the tool itself stops at the range");
    draw(&mut s, 300.0, true);
    assert_eq!(s.tool_options()["cornerRadius"], json!(1296.0), "↑ at the top stays there");
}

/// General › Corner Radius set for the Rounded Rectangle tool by name (`tool.setOption {tool}`)
/// while it has lent a selection tool for a Cmd-drag reaches the tool kept aside: back from the
/// drag, it draws the radius that is the preference now, not the one from before.
#[test]
fn corner_radius_set_by_name_reaches_the_tool_that_lent_a_selection_tool() {
    use PointerKind::{Down, Drag, Up};
    let mut s = new_doc();
    let v = ViewInfo::default();
    set_pref(&mut s, "cornerRadius", json!(7));
    s.select_tool("roundedRectangle", v).unwrap();
    let cmd = Mods { cmd: true, ..Mods::default() };
    gesture(&mut s, cmd, &[(Down, 300.0, 300.0), (Drag, 320.0, 320.0)]);
    assert_eq!(s.tool_id(), "selection", "Cmd lends the Selection tool");
    s.set_tool_option_cmd(&json!({"tool": "roundedRectangle", "key": "cornerRadius", "value": 9})).unwrap();
    assert_eq!(s.execute("prefs.get", &json!({"key": "cornerRadius"})).unwrap(), json!(9.0), "the preference");
    gesture(&mut s, cmd, &[(Up, 320.0, 320.0)]);
    assert_eq!(s.tool_id(), "roundedRectangle", "the drag over, the tool is back");
    assert_eq!(s.tool_options()["cornerRadius"], json!(9.0), "with the radius set while it was aside");
    gesture(&mut s, Mods::default(), &[(Down, 100.0, 100.0), (Drag, 150.0, 130.0), (Drag, 200.0, 160.0), (Up, 200.0, 160.0)]);
    assert!(close(live_box(&s, last_id(&s)).3, 9.0), "and draws it");
}
