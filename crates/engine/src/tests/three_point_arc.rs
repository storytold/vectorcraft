//! `shape.threePointArc`: an open circular arc between two ends, through a third point or
//! leaving an end toward it, and the Three-Point Arc tool's clicks that run it.

use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::Point;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::*;

const C: &str = "shape.threePointArc";

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn object_count(s: &Session) -> usize {
    let mut n = 0;
    s.doc().unwrap().doc.walk(|_| n += 1);
    n
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// The largest distance of `points` from the path of `id`.
fn farthest_from_path(s: &Session, id: NodeId, points: &[Point]) -> f64 {
    let n = node(s, id);
    let path = n.path_data().unwrap();
    points.iter().map(|p| path.nearest(*p).unwrap().4).fold(0.0, f64::max)
}

/// The arc from (0, 0) to (80, 0) through (40, -40) is the upper half of the circle about
/// (40, 0) with radius 40: an open path from one end to the other, through the third point, with
/// no fill and a stroke, selected, and one undo step.
#[test]
fn the_arc_through_a_third_point_is_an_open_path_and_one_undo_step() {
    let mut s = session();
    let undo = undo_len(&s);
    let out = s.execute(C, &json!({"start": [0, 0], "end": [80, 0], "through": [40, -40]})).unwrap();
    assert_eq!((out["cx"].as_f64(), out["cy"].as_f64(), out["radius"].as_f64()), (Some(40.0), Some(0.0), Some(40.0)), "{out}");
    // From (0, 0) over the top to (80, 0): clockwise on the page.
    assert!((out["sweep"].as_f64().unwrap() - 180.0).abs() < 1e-9, "{out}");
    let id = id_of(&out);
    let n = node(&s, id);
    let NodeKind::Path { path, live: None, .. } = &n.kind else { panic!("not a plain path: {:?}", n.kind) };
    let sp = &path.subpaths[0];
    assert!(!sp.closed && path.subpaths.len() == 1);
    assert_eq!((sp.anchors.first().map(|a| a.p), sp.anchors.last().map(|a| a.p)), (Some(Point::ORIGIN), Some(Point::new(80.0, 0.0))));
    assert!(farthest_from_path(&s, id, &[Point::new(40.0, -40.0)]) < 1e-9);
    assert!(n.appearance.fill_paint().is_none(), "no fill");
    assert!(!n.appearance.stroke_paint().is_none(), "a stroke");
    assert_eq!(s.doc().unwrap().selection.objects, [id]);
    assert_eq!(undo_len(&s), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(id).is_none());
    // A third point between the quarter turns is an anchor too, so the path meets it exactly.
    let via = Point::new(1000.0 * 20f64.to_radians().cos(), 1000.0 * 20f64.to_radians().sin());
    let out = s.execute(C, &json!({"start": [1000, 0], "end": [0, 1000], "through": [via.x, via.y]})).unwrap();
    let id = id_of(&out);
    assert!(node(&s, id).path_data().unwrap().subpaths[0].anchors.iter().any(|a| a.p == via));
    assert!(farthest_from_path(&s, id, &[via]) < 1e-9);
}

/// With `toward`, the arc leaves `start` heading at the point (its first handle points there),
/// or with `at: "end"` arrives at `end` along the line from the point to `end` (run backward, the
/// arc leaves `end` heading at the point).
#[test]
fn the_arc_toward_a_point_has_that_tangent_at_its_end() {
    let mut s = session();
    let out = s.execute(C, &json!({"start": [0, 0], "end": [80, 0], "toward": [30, -30]})).unwrap();
    let n = node(&s, id_of(&out));
    let first = n.path_data().unwrap().subpaths[0].anchors[0];
    let d = (first.h_out - first.p).normalize();
    assert!((d.dot(vectorcraft_geom::Vec2::new(1.0, -1.0).normalize()) - 1.0).abs() < 1e-12, "{d:?}");
    // A quarter turn's tangent: the center is at (40, 40) and the radius √3200.
    assert!((out["radius"].as_f64().unwrap() - 3200f64.sqrt()).abs() < 1e-9, "{out}");
    let out = s.execute(C, &json!({"start": [0, 0], "end": [80, 0], "toward": [50, -30], "at": "end"})).unwrap();
    let n = node(&s, id_of(&out));
    let last = *n.path_data().unwrap().subpaths[0].anchors.last().unwrap();
    let d = (last.p - last.h_in).normalize();
    assert!((d.dot(vectorcraft_geom::Vec2::new(1.0, 1.0).normalize()) - 1.0).abs() < 1e-12, "{d:?}");
    // The direction from the start and from the end mirror each other.
    assert!((out["cx"].as_f64().unwrap() - 40.0).abs() < 1e-9 && (out["cy"].as_f64().unwrap() - 40.0).abs() < 1e-9, "{out}");
}

/// Points that are the same, a third point on the line through the ends, and an arc reaching past
/// the canvas are refused with a message; nothing is drawn and no undo step is added.
#[test]
fn points_with_no_arc_are_refused() {
    let mut s = session();
    let (undo, count) = (undo_len(&s), object_count(&s));
    for p in [
        json!({"start": [0, 0], "end": [0, 0], "through": [10, 10]}),
        json!({"start": [0, 0], "end": [80, 0], "through": [40, 0]}),
        json!({"start": [0, 0], "end": [80, 0], "through": [120, 0]}),
        json!({"start": [0, 0], "end": [80, 0], "through": [80, 0]}),
        json!({"start": [0, 0], "end": [80, 0], "toward": [0, 0]}),
        json!({"start": [0, 0], "end": [80, 0], "toward": [-50, 0]}),
        json!({"start": [0, 0], "end": [80, 0], "toward": [40, 0], "at": "end"}),
        json!({"start": [0, 0], "end": [0, 0], "toward": [10, 10]}),
    ] {
        let e = s.execute(C, &p).unwrap_err().to_string();
        assert!(e.contains("no arc fits these points"), "{p}: {e}");
    }
    // Nearly in line beyond the end: the circle's center is about 1.2·10⁷ pt down the page, and the
    // arc goes round the far side of it.
    let far = json!({"start": [0, 0], "end": [80, 0], "through": [200, 0.001]});
    let e = s.execute(C, &far).unwrap_err().to_string();
    assert!(e.contains("result would exceed the canvas"), "{e}");
    assert_eq!((undo_len(&s), object_count(&s)), (undo, count));
}

/// Missing or malformed points, both or neither of `through` and `toward`, an `at` that is not
/// "start" or "end", and an `at` with `through` are errors; nothing is drawn.
#[test]
fn malformed_params_are_refused() {
    let mut s = session();
    let count = object_count(&s);
    for (p, want) in [
        (json!({"end": [80, 0], "through": [40, 40]}), "`start` must be [x, y]"),
        (json!({"start": [0, 0], "end": [80], "through": [40, 40]}), "`end` must be [x, y]"),
        (json!({"start": [0, 0], "end": [80, 0], "through": [40, "a"]}), "`through` must be [x, y]"),
        (json!({"start": [0, 0], "end": [80, 0], "through": null}), "`through` must be [x, y]"),
        (json!({"start": [0, 0], "end": [80, 0], "toward": [10, 0, 0]}), "`toward` must be [x, y]"),
        (json!({"start": [0, 0], "end": [80, 0]}), "give exactly one of `through` and `toward`"),
        (json!({"start": [0, 0], "end": [80, 0], "through": [40, 40], "toward": [0, 40]}), "give exactly one of `through` and `toward`"),
        (json!({"start": [0, 0], "end": [80, 0], "toward": [0, 40], "at": "middle"}), "`at` must be \"start\" or \"end\""),
        (json!({"start": [0, 0], "end": [80, 0], "toward": [0, 40], "at": null}), "`at` must be \"start\" or \"end\""),
        (json!({"start": [0, 0], "end": [80, 0], "through": [40, 40], "at": "end"}), "give `at` only with `toward`"),
        (json!({"start": [0, 0], "end": [80, 0], "through": [40, 40], "at": "start"}), "give `at` only with `toward`"),
    ] {
        let e = s.execute(C, &p).unwrap_err().to_string();
        assert!(e.contains(want), "{p}: {e}");
    }
    assert_eq!(object_count(&s), count);
}

/// With Align New Objects to Pixel Grid on, the ends stay on the given points.
#[test]
fn the_arc_is_not_moved_onto_the_pixel_grid() {
    let mut s = session();
    s.execute("document.setup", &json!({"alignNewObjectsToPixelGrid": true})).unwrap();
    let id = id_of(&s.execute(C, &json!({"start": [0.3, 0.2], "end": [80.6, 0.1], "through": [40.4, -39.7]})).unwrap());
    let n = node(&s, id);
    let sp = &n.path_data().unwrap().subpaths[0];
    assert_eq!(sp.anchors.first().map(|a| a.p), Some(Point::new(0.3, 0.2)));
    assert_eq!(sp.anchors.last().map(|a| a.p), Some(Point::new(80.6, 0.1)));
    assert!(!n.extras().pixel_aligned);
}

fn view() -> ViewInfo {
    ViewInfo { smart_guides: false, ..ViewInfo::default() }
}

fn click(s: &mut Session, x: f64, y: f64) {
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, x, y).with_mods(Mods::default()), view()).unwrap();
    }
}

/// With the Three-Point Arc tool, three clicks draw the arc in one undo step; `]` before the third
/// click makes the third point give the direction instead. Escape drops the ends.
#[test]
fn three_clicks_with_the_tool_draw_the_arc() {
    let mut s = session();
    s.select_tool("threePointArc", view()).unwrap();
    let (undo, count) = (undo_len(&s), object_count(&s));
    click(&mut s, 100.0, 200.0);
    click(&mut s, 180.0, 200.0);
    assert_eq!((undo_len(&s), object_count(&s)), (undo, count), "two ends draw nothing yet");
    click(&mut s, 140.0, 160.0);
    assert_eq!(undo_len(&s), undo + 1);
    let id = *s.doc().unwrap().selection.objects.first().unwrap();
    assert!(farthest_from_path(&s, id, &[Point::new(100.0, 200.0), Point::new(140.0, 160.0), Point::new(180.0, 200.0)]) < 1e-9);
    // Escape after the ends: the next clicks start afresh.
    click(&mut s, 0.0, 0.0);
    click(&mut s, 10.0, 300.0);
    s.tool_key(ToolKey::Escape, Mods::default(), view()).unwrap();
    click(&mut s, 100.0, 300.0);
    click(&mut s, 180.0, 300.0);
    s.tool_key(ToolKey::BracketRight, Mods::default(), view()).unwrap();
    let undo = undo_len(&s);
    // Straight up from the first end: the upper half circle about (140, 300).
    click(&mut s, 100.0, 250.0);
    assert_eq!(undo_len(&s), undo + 1);
    let id = *s.doc().unwrap().selection.objects.first().unwrap();
    assert!(farthest_from_path(&s, id, &[Point::new(140.0, 260.0)]) < 1e-9);
}

/// `tool.setOption` (the Tool Options dialog's Save) sets the tool's `direction` option, and
/// refuses a value that is not true or false, or an option the tool doesn't have.
#[test]
fn set_option_sets_the_direction_and_refuses_bad_values() {
    let mut s = session();
    for on in [true, false] {
        let set = s.set_tool_option_cmd(&json!({"tool": "threePointArc", "values": {"direction": on}})).unwrap();
        assert_eq!(set["direction"], json!(on));
        assert_eq!(s.tool_options_of("threePointArc")["direction"], json!(on));
    }
    let cases = [
        (json!({"tool": "threePointArc", "key": "direction", "value": "yes"}), "tool `threePointArc`: `direction` must be true or false"),
        (json!({"tool": "threePointArc", "key": "bogus", "value": true}), "tool `threePointArc`: unknown option `bogus`"),
    ];
    for (p, want) in cases {
        assert_eq!(s.set_tool_option_cmd(&p).unwrap_err(), want, "{p}");
    }
    assert_eq!(s.tool_options_of("threePointArc")["direction"], json!(false));
}
