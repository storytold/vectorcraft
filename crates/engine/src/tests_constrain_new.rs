//! General › Constrain Angle sets the angle new objects are drawn at (G12, #394, second batch):
//! type, grids, polygons and stars at their base orientation, shapes made by a click through the
//! size dialogs, and the Line Segment tool's Shift steps. 0° and whole turns draw exactly as
//! before; quarter turns are exact.

use serde_json::{Value, json};
use vectorcraft_doc::{LiveShape, NodeKind, TextKind};
use vectorcraft_geom::{PathData, Point, Rect};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use super::*;

use PointerKind::{Down, Drag, Up};

fn new_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn set_angle(s: &mut Session, deg: f64) {
    s.execute("prefs.set", &json!({"key": "constrainAngle", "value": deg})).unwrap();
}

fn gesture(s: &mut Session, mods: Mods, events: &[(PointerKind, f64, f64)]) {
    for (k, x, y) in events {
        s.pointer(&PointerEvent::new(*k, *x, *y).with_mods(mods), ViewInfo { smart_guides: false, ..ViewInfo::default() }).unwrap();
    }
}

fn last_id(s: &Session) -> NodeId {
    *s.doc().unwrap().selection.objects.first().unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// The angles (degrees, counter-clockwise on the page, in 0..90) of the straight segments of `p`.
fn segment_angles(p: &PathData) -> Vec<f64> {
    p.subpaths
        .iter()
        .flat_map(|sp| sp.anchors.windows(2).map(|w| w[1].p - w[0].p))
        .filter(|v| v.hypot() > 1e-9)
        .map(|v| (-v.y).atan2(v.x).to_degrees().rem_euclid(90.0))
        .collect()
}

/// The paths of the group `id` (a grid), on the page.
fn grid_paths(s: &Session, id: NodeId) -> Vec<PathData> {
    let n = s.doc().unwrap().doc.node(id).unwrap();
    n.children()
        .unwrap()
        .iter()
        .map(|c| match &c.kind {
            NodeKind::Path { path, .. } => path.clone(),
            other => panic!("a path: {other:?}"),
        })
        .collect()
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

/// Rectangular and polar grids are drawn along the turned axes: at 30° the frame's sides run at
/// 30° and 120°, at 90° the grid covers the drag exactly, 20 wide and 60 high along the turned
/// axes; at 0° and a whole turn the grid is the same as before, bit for bit.
#[test]
fn constrain_angle_draws_grids_along_the_turned_axes() {
    let mut s = new_doc();
    let draw = |s: &mut Session, tool: &str, deg: f64| {
        set_angle(s, deg);
        s.select_tool(tool, ViewInfo::default()).unwrap();
        gesture(s, Mods::default(), &[(Down, 100.0, 100.0), (Drag, 130.0, 110.0), (Drag, 160.0, 120.0), (Up, 160.0, 120.0)]);
        last_id(s)
    };
    for tool in ["rectangularGrid", "polarGrid"] {
        let flat = draw(&mut s, tool, 0.0);
        let flat_paths = grid_paths(&s, flat);
        for turn in [360.0, -360.0] {
            let id = draw(&mut s, tool, turn);
            assert_eq!(grid_paths(&s, id), flat_paths, "{tool} at {turn}°: as at 0°");
        }
        let b = bounds(&s, flat);
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (100.0, 100.0, 160.0, 120.0), "{tool} at 0°");
        // A quarter turn: the drag's box exactly, no floating-point noise.
        let id = draw(&mut s, tool, 90.0);
        let b = bounds(&s, id);
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (100.0, 100.0, 160.0, 120.0), "{tool} at 90°: {b:?}");
    }
    // At 30° the rectangular grid's lines run along the turned axes.
    let id = draw(&mut s, "rectangularGrid", 30.0);
    for p in grid_paths(&s, id) {
        for a in segment_angles(&p) {
            assert!(close(a, 30.0), "a grid line at {a}°");
        }
    }
    // Through the command: `rotation` turns a grid about its centre.
    let id = NodeId(
        s.execute("shape.rectangularGrid", &json!({"x": 0, "y": 0, "width": 40, "height": 20, "rotation": -90})).unwrap()["id"].as_u64().unwrap(),
    );
    let b = bounds(&s, id);
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (10.0, -10.0, 30.0, 30.0));
}

/// Shift keeps a polygon or star at its base orientation, which is upright turned by the
/// Constrain Angle: at 30° the polygon is turned 30° counter-clockwise, at 90° the star's first
/// point is exactly left of its centre, at 0° and whole turns both are upright as before.
#[test]
fn constrain_angle_turns_the_base_orientation_of_polygons_and_stars() {
    let mut s = new_doc();
    let shift = Mods { shift: true, ..Mods::default() };
    let draw = |s: &mut Session, tool: &str, deg: f64| {
        set_angle(s, deg);
        s.select_tool(tool, ViewInfo::default()).unwrap();
        gesture(s, shift, &[(Down, 100.0, 100.0), (Drag, 130.0, 140.0), (Drag, 130.0, 140.0), (Up, 130.0, 140.0)]);
        last_id(s)
    };
    let polygon_turn = |s: &Session, id: NodeId| {
        let NodeKind::Path { live: Some(live), .. } = &s.doc().unwrap().doc.node(id).unwrap().kind else { panic!("a polygon") };
        let LiveShape::Polygon { xf, .. } = &**live else { panic!("a polygon") };
        let [a, b, ..] = xf.as_coeffs();
        -b.atan2(a).to_degrees()
    };
    let path = |s: &Session, id: NodeId| match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Path { path, .. } => path.clone(),
        other => panic!("a path: {other:?}"),
    };
    let flat = draw(&mut s, "polygon", 0.0);
    assert_eq!(polygon_turn(&s, flat), 0.0);
    let flat_star = draw(&mut s, "star", 0.0);
    for turn in [360.0, -360.0] {
        let id = draw(&mut s, "polygon", turn);
        assert_eq!(path(&s, id), path(&s, flat), "polygon at {turn}°");
        let id = draw(&mut s, "star", turn);
        assert_eq!(path(&s, id), path(&s, flat_star), "star at {turn}°");
    }
    let id = draw(&mut s, "polygon", 30.0);
    assert!(close(polygon_turn(&s, id), 30.0), "{}", polygon_turn(&s, id));
    let id = draw(&mut s, "polygon", 90.0);
    assert!(close(polygon_turn(&s, id), 90.0));
    // The 50 pt star's first point: straight up at 0°, straight left at 90°, exactly.
    assert_eq!(path(&s, flat_star).subpaths[0].anchors[0].p, Point::new(100.0, 50.0));
    let id = draw(&mut s, "star", 90.0);
    assert_eq!(path(&s, id).subpaths[0].anchors[0].p, Point::new(50.0, 100.0));
    // Without Shift the drag turns them, whatever the preference.
    set_angle(&mut s, 30.0);
    s.select_tool("polygon", ViewInfo::default()).unwrap();
    gesture(&mut s, Mods::default(), &[(Down, 100.0, 100.0), (Drag, 150.0, 100.0), (Up, 150.0, 100.0)]);
    assert!(close(polygon_turn(&s, last_id(&s)), -90.0), "the drag points the polygon right");
}

/// New type is set along the turned axes: point type turns about its first baseline's start, an
/// area's box is measured along the axes; 0° and whole turns as before, 90° exact.
#[test]
fn constrain_angle_sets_new_type_along_the_turned_axes() {
    let mut s = new_doc();
    let xf = |s: &Session, id: NodeId| match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (t.xf, t.kind.clone()),
        other => panic!("type: {other:?}"),
    };
    // Each at its own place, so no click lands in type drawn before.
    let type_at = |s: &mut Session, deg: f64, area: bool, y: f64| {
        set_angle(s, deg);
        s.select_tool("type", ViewInfo::default()).unwrap();
        if area {
            gesture(s, Mods::default(), &[(Down, 20.0, y), (Drag, 70.0, y + 50.0), (Drag, 120.0, y + 60.0), (Up, 120.0, y + 60.0)]);
        } else {
            gesture(s, Mods::default(), &[(Down, 300.0, y), (Up, 300.0, y)]);
        }
        let id = last_id(s);
        s.select_tool("selection", ViewInfo::default()).unwrap();
        let (m, kind) = xf(s, id);
        // Where it was put, and how it is turned and shaped there.
        (m.as_coeffs()[4..].to_vec(), m.as_coeffs()[..4].to_vec(), kind, m)
    };
    let (at, flat_point, ..) = type_at(&mut s, 0.0, false, 20.0);
    assert_eq!(at, [300.0, 20.0]);
    let (at, flat_area, flat_frame, _) = type_at(&mut s, 0.0, true, 20.0);
    assert_eq!(at, [20.0, 20.0]);
    for (turn, y) in [(360.0, 110.0), (-360.0, 200.0)] {
        let (at, turned, ..) = type_at(&mut s, turn, false, y);
        assert_eq!((at, turned), (vec![300.0, y], flat_point.clone()), "point type at {turn}°");
        let (at, turned, frame, _) = type_at(&mut s, turn, true, y);
        assert_eq!((at, turned, frame), (vec![20.0, y], flat_area.clone(), flat_frame.clone()), "area type at {turn}°");
    }
    // Point type at 30°: its baseline runs 30° up from the horizontal, from the click.
    let (at, turned, ..) = type_at(&mut s, 30.0, false, 290.0);
    assert!(close(-turned[1].atan2(turned[0]).to_degrees(), 30.0) && at == [300.0, 290.0], "{turned:?} at {at:?}");
    // Area type at 90°: 100 right and 60 down on the page is 60 by 100 along the turned axes, and
    // the frame covers the drag exactly.
    let (_, _, kind, m) = type_at(&mut s, 90.0, true, 290.0);
    let TextKind::Area { frame } = kind else { panic!("area type") };
    let local = frame.bounds().unwrap();
    assert_eq!((local.width(), local.height()), (60.0, 100.0));
    let page = frame.transformed(m).bounds().unwrap();
    assert_eq!((page.x0, page.y0, page.x1, page.y1), (20.0, 290.0, 120.0, 350.0));
    // Through the command.
    let r = s.execute("text.create", &json!({"x": 10, "y": 20, "text": "", "rotation": -90})).unwrap();
    let (m, _) = xf(&s, NodeId(r["id"].as_u64().unwrap()));
    assert_eq!(m.as_coeffs(), [0.0, -1.0, 1.0, 0.0, 10.0, 20.0]);
    assert!(s.execute("text.create", &json!({"x": 10, "y": 20, "text": "", "rotation": "x"})).is_err());
}

/// The top-left corner `x`, `y` of a size dialog's params: the point it places at the click.
fn corner(p: &Value) -> Point {
    Point::new(p["x"].as_f64().unwrap_or(0.0), p["y"].as_f64().unwrap_or(0.0))
}

/// A click's size dialog draws along the turned axes too ([`constrain_click`]): the box keeps
/// its corner at the click, a polygon or star turns about its centre; at 0° and whole turns the
/// params are as they were.
#[test]
fn constrain_angle_turns_shapes_made_by_a_click() {
    let mut s = new_doc();
    let rect = json!({"x": 10, "y": 200, "width": 100, "height": 50});
    for turn in [0.0, 360.0, -360.0] {
        set_angle(&mut s, turn);
        for (cmd, p) in [("shape.rectangle", rect.clone()), ("shape.polygon", json!({"cx": 10, "cy": 10, "radius": 50, "sides": 6}))] {
            assert_eq!(constrain_click(&s, cmd, p.clone(), corner(&p)), p, "{cmd} at {turn}°");
        }
    }
    set_angle(&mut s, 90.0);
    let run = |s: &mut Session, cmd: &str, p: Value| {
        let at = corner(&p);
        let p = constrain_click(s, cmd, p, at);
        NodeId(s.execute(cmd, &p).unwrap()["id"].as_u64().unwrap())
    };
    for cmd in ["shape.rectangle", "shape.ellipse", "shape.rectangularGrid", "shape.polarGrid"] {
        // 100 along the turned x axis (up the page) and 50 along the turned y (right).
        let id = run(&mut s, cmd, rect.clone());
        let b = bounds(&s, id);
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (10.0, 100.0, 60.0, 200.0), "{cmd} at 90°");
    }
    let p = constrain_click(&s, "shape.star", json!({"cx": 0, "cy": 0, "radius1": 50, "radius2": 25, "points": 5}), Point::ZERO);
    assert_eq!(p["rotation"], json!(-90.0));
    // An explicit rotation is the caller's.
    assert_eq!(constrain_click(&s, "shape.polygon", json!({"rotation": 5}), Point::ZERO)["rotation"], json!(5));
    // Malformed params go through as they are, for the command to refuse.
    assert_eq!(constrain_click(&s, "shape.polygon", json!([1]), Point::ZERO), json!([1]));
    assert_eq!(constrain_click(&s, "shape.rectangle", json!({"x": "a"}), Point::ZERO), json!({"x": "a"}));
    set_angle(&mut s, 30.0);
    let id = run(&mut s, "shape.rectangle", rect.clone());
    let NodeKind::Path { path, .. } = &s.doc().unwrap().doc.node(id).unwrap().kind else { panic!() };
    // The click's corner stays put, the sides run along 30° and 120°.
    assert!(path.subpaths[0].anchors.iter().any(|a| close(a.p.x, 10.0) && close(a.p.y, 200.0)), "{path:?}");
    for a in segment_angles(path) {
        assert!(close(a, 30.0), "a side at {a}°");
    }
}

/// The Line Segment tool's Shift steps of 45° count from the Constrain Angle.
#[test]
fn constrain_angle_steps_the_line_segment_tool() {
    let mut s = new_doc();
    let shift = Mods { shift: true, ..Mods::default() };
    let line = |s: &mut Session, deg: f64, (x, y): (f64, f64)| {
        set_angle(s, deg);
        s.select_tool("lineSegment", ViewInfo::default()).unwrap();
        gesture(s, shift, &[(Down, 100.0, 100.0), (Drag, 150.0, 102.0), (Drag, x, y), (Up, x, y)]);
        let NodeKind::Path { path, .. } = &s.doc().unwrap().doc.node(last_id(s)).unwrap().kind else { panic!() };
        let v = path.subpaths[0].anchors[1].p - path.subpaths[0].anchors[0].p;
        (-v.y).atan2(v.x).to_degrees()
    };
    let level = (200.0, 105.0);
    assert_eq!(line(&mut s, 0.0, level), 0.0);
    assert_eq!(line(&mut s, 360.0, level), 0.0);
    // Nearly level: the step nearest it, counted from 30°, is −15°.
    assert!(close(line(&mut s, 30.0, level), -15.0));
    assert!(close(line(&mut s, 30.0, (200.0, 45.0)), 30.0), "along the turned x axis");
    assert!(close(line(&mut s, 90.0, level), 0.0));
}

fn selection_rotation(s: &Session) -> f64 {
    // What `document.inspect` reports as `selectionRotation`.
    let st = s.doc().unwrap();
    st.doc.bbox_angle(&st.selection.objects)
}

fn bbox_angle(s: &Session, id: NodeId) -> f64 {
    s.doc().unwrap().doc.node(id).unwrap().bbox_angle
}

/// Objects drawn along a Constrain Angle keep a bounding box turned with them, as after the Rotate
/// tool: at 30° the selection's rotation is 30°, the same as a rectangle turned 30° by
/// `object.rotate`; at 0° and whole turns the box stays on the page (exactly 0, not −0).
#[test]
fn constrain_angle_turns_the_new_objects_bounding_box() {
    let mut s = new_doc();
    s.execute("shape.rectangle", &json!({"x": 300, "y": 300, "width": 40, "height": 20})).unwrap();
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    let rotated = selection_rotation(&s);
    assert!(close(rotated, 30.0), "{rotated}");
    let shift = Mods { shift: true, ..Mods::default() };
    type Gesture = [(PointerKind, f64, f64); 4];
    let draws: [(&str, Mods, Gesture); 6] = [
        ("rectangle", Mods::default(), [(Down, 20.0, 20.0), (Drag, 50.0, 40.0), (Drag, 80.0, 60.0), (Up, 80.0, 60.0)]),
        ("ellipse", Mods::default(), [(Down, 20.0, 120.0), (Drag, 50.0, 140.0), (Drag, 80.0, 160.0), (Up, 80.0, 160.0)]),
        ("polygon", shift, [(Down, 150.0, 50.0), (Drag, 170.0, 70.0), (Drag, 180.0, 80.0), (Up, 180.0, 80.0)]),
        ("star", shift, [(Down, 150.0, 150.0), (Drag, 170.0, 170.0), (Drag, 180.0, 180.0), (Up, 180.0, 180.0)]),
        ("rectangularGrid", Mods::default(), [(Down, 220.0, 20.0), (Drag, 250.0, 40.0), (Drag, 280.0, 60.0), (Up, 280.0, 60.0)]),
        ("polarGrid", Mods::default(), [(Down, 220.0, 120.0), (Drag, 250.0, 140.0), (Drag, 280.0, 160.0), (Up, 280.0, 160.0)]),
    ];
    for deg in [0.0, 360.0, 30.0] {
        set_angle(&mut s, deg);
        for (tool, mods, events) in &draws {
            s.select_tool(tool, ViewInfo::default()).unwrap();
            gesture(&mut s, *mods, events);
            let id = last_id(&s);
            if deg == 30.0 {
                assert!(close(selection_rotation(&s), rotated), "{tool}: {}", selection_rotation(&s));
            } else {
                assert_eq!(bbox_angle(&s, id).to_bits(), 0.0f64.to_bits(), "{tool} at {deg}°");
            }
        }
    }
    // A polygon turned by the drag's direction, not the Constrain Angle, keeps its box on the page.
    s.select_tool("polygon", ViewInfo::default()).unwrap();
    gesture(&mut s, Mods::default(), &[(Down, 150.0, 250.0), (Drag, 160.0, 260.0), (Drag, 190.0, 270.0), (Up, 190.0, 270.0)]);
    assert_eq!(bbox_angle(&s, last_id(&s)).to_bits(), 0.0f64.to_bits(), "a polygon turned by the drag");
    // A grid's lines turn their boxes with it, and so does new type.
    let id =
        NodeId(s.execute("shape.polarGrid", &json!({"x": 0, "y": 0, "width": 40, "height": 20, "rotation": -30})).unwrap()["id"].as_u64().unwrap());
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    assert!(n.children().unwrap().iter().all(|c| close(c.bbox_angle, 30.0)));
    s.execute("text.create", &json!({"x": 10, "y": 20, "text": "Hi", "rotation": -30})).unwrap();
    assert!(close(selection_rotation(&s), 30.0));
    s.execute("text.create", &json!({"x": 10, "y": 20, "text": "Hi"})).unwrap();
    assert_eq!(selection_rotation(&s).to_bits(), 0.0f64.to_bits());
}

/// The grid commands refuse a `rotation` that is not a number.
#[test]
fn grid_rotation_must_be_a_number() {
    let mut s = new_doc();
    for cmd in ["shape.rectangularGrid", "shape.polarGrid"] {
        for bad in [json!("x"), Value::Null] {
            assert!(s.execute(cmd, &json!({"x": 0, "y": 0, "width": 40, "height": 20, "rotation": bad})).is_err(), "{cmd}: {bad}");
        }
    }
}

/// Area type dragged at 30°: its frame runs along the turned axes with the drag's start and end
/// at opposite corners; vertical type takes a rotation as horizontal type does.
#[test]
fn constrain_angle_sets_area_and_vertical_type() {
    let mut s = new_doc();
    set_angle(&mut s, 30.0);
    s.select_tool("type", ViewInfo::default()).unwrap();
    gesture(&mut s, Mods::default(), &[(Down, 100.0, 100.0), (Drag, 150.0, 150.0), (Drag, 250.0, 200.0), (Up, 250.0, 200.0)]);
    let id = last_id(&s);
    let NodeKind::Text(t) = &s.doc().unwrap().doc.node(id).unwrap().kind else { panic!("type") };
    let [a, b, ..] = t.xf.as_coeffs();
    assert!(close(-b.atan2(a).to_degrees(), 30.0), "{:?}", t.xf);
    let TextKind::Area { frame } = &t.kind else { panic!("area type") };
    // The drag (150 right, 100 down) measured along axes turned 30° counter-clockwise.
    let r = 30f64.to_radians();
    let (w, h) = (150.0 * r.cos() - 100.0 * r.sin(), 150.0 * r.sin() + 100.0 * r.cos());
    let local = frame.bounds().unwrap();
    assert!(close(local.width(), w.abs()) && close(local.height(), h.abs()), "{local:?} vs {w} × {h}");
    let corners: Vec<Point> = [(local.x0, local.y0), (local.x1, local.y0), (local.x0, local.y1), (local.x1, local.y1)]
        .iter()
        .map(|&(x, y)| t.xf * Point::new(x, y))
        .collect();
    for p in [Point::new(100.0, 100.0), Point::new(250.0, 200.0)] {
        assert!(corners.iter().any(|c| close(c.x, p.x) && close(c.y, p.y)), "{p:?} in {corners:?}");
    }
    assert!(close(selection_rotation(&s), 30.0));
    s.select_tool("selection", ViewInfo::default()).unwrap();
    // Vertical type with a rotation.
    let r = s.execute("text.create", &json!({"x": 300, "y": 50, "text": "Up", "vertical": true, "rotation": -90})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    let NodeKind::Text(t) = &s.doc().unwrap().doc.node(id).unwrap().kind else { panic!("type") };
    assert!(t.vertical);
    assert_eq!(t.xf.as_coeffs(), [0.0, -1.0, 1.0, 0.0, 300.0, 50.0]);
    assert_eq!(bbox_angle(&s, id), 90.0);
}

/// A grid dialog placed by its reference point keeps that point at the click when the grid turns
/// along the Constrain Angle: at 90° with the centre as the reference, the box turns about it.
#[test]
fn constrain_angle_turns_a_click_made_grid_about_its_reference_point() {
    let mut s = new_doc();
    set_angle(&mut s, 90.0);
    // A 100 × 50 grid centred on the click at (200, 200).
    let p = json!({"x": 150, "y": 175, "width": 100, "height": 50});
    let p = constrain_click(&s, "shape.rectangularGrid", p, Point::new(200.0, 200.0));
    let id = NodeId(s.execute("shape.rectangularGrid", &p).unwrap()["id"].as_u64().unwrap());
    let b = bounds(&s, id);
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (175.0, 150.0, 225.0, 250.0), "turned about the click: {b:?}");
}
