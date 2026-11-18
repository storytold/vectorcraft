//! `shape.circleThroughPoints`: a circle through two or three given points, or through the two or
//! three anchor points selected with Direct Selection, the Ellipse tool's three Shift-clicks that
//! run it, and the Circle tool.

use serde_json::{Value, json};
use vectorcraft_doc::{LiveShape, Node, NodeKind};
use vectorcraft_geom::Point;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::*;

const C: &str = "shape.circleThroughPoints";

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

/// Whether the geometric bounds of `id` are `want` to within 10⁻⁹ pt.
fn bounds_are(s: &Session, id: NodeId, want: vectorcraft_geom::Rect) -> bool {
    let b = s.doc().unwrap().doc.bounds_of(&[id], false).unwrap();
    [(b.x0, want.x0), (b.y0, want.y0), (b.x1, want.x1), (b.y1, want.y1)].iter().all(|(a, w)| (a - w).abs() < 1e-9)
}

/// The largest distance of `points` from the path of `id`.
fn farthest_from_path(s: &Session, id: NodeId, points: &[Point]) -> f64 {
    let n = node(s, id);
    let path = n.path_data().unwrap();
    points.iter().map(|p| path.nearest(*p).unwrap().4).fold(0.0, f64::max)
}

/// (0, 0), (80, 0) and (0, 60) lie on the circle centered at (40, 30) with radius 50 (their
/// right angle stands on the diameter). The circle is a live ellipse with the current paint, is
/// selected, passes through the points to within the four-curve circle's error (about 0.03% of the
/// radius), and one undo removes it.
#[test]
fn the_circle_through_three_given_points_is_exact_and_one_undo_step() {
    let mut s = session();
    let reference = id_of(&s.execute("shape.ellipse", &json!({"x": 200, "y": 200, "width": 10, "height": 10})).unwrap());
    let undo = undo_len(&s);
    let points = [Point::new(0.0, 0.0), Point::new(80.0, 0.0), Point::new(0.0, 60.0)];
    let out = s.execute(C, &json!({"points": [[0, 0], [80, 0], [0, 60]]})).unwrap();
    assert_eq!((out["cx"].as_f64(), out["cy"].as_f64(), out["radius"].as_f64()), (Some(40.0), Some(30.0), Some(50.0)), "{out}");
    let id = id_of(&out);
    let n = node(&s, id);
    match &n.kind {
        NodeKind::Path { live: Some(live), .. } => match live.as_ref() {
            LiveShape::Ellipse { w, h, pie, .. } => assert_eq!((*w, *h, *pie), (100.0, 100.0, (0.0, 360.0))),
            other => panic!("not a live ellipse: {other:?}"),
        },
        other => panic!("not a live shape: {other:?}"),
    }
    assert!(bounds_are(&s, id, vectorcraft_geom::Rect::new(-10.0, -20.0, 90.0, 80.0)));
    assert!(farthest_from_path(&s, id, &points) < 50.0 * 3e-4, "{}", farthest_from_path(&s, id, &points));
    assert_eq!(n.appearance, node(&s, reference).appearance, "the current paint, as the Ellipse tool draws");
    assert_eq!(s.doc().unwrap().selection.objects, [id]);
    assert_eq!(undo_len(&s), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(id).is_none());
}

/// Two points that are the same, or three on one line, are refused with a message; nothing is
/// drawn and no undo step is recorded. Points nearly on one line still give their circle.
#[test]
fn coincident_or_collinear_points_are_refused() {
    let mut s = session();
    let (undo, count) = (undo_len(&s), object_count(&s));
    for points in [json!([[10, 10], [10, 10], [50, 80]]), json!([[0, 0], [50, 50], [100, 100]]), json!([[5, 5], [5, 5], [5, 5]])] {
        let e = s.execute(C, &json!({ "points": points })).unwrap_err().to_string();
        assert!(e.contains("no circle passes through these points"), "{points}: {e}");
        assert_eq!((undo_len(&s), object_count(&s)), (undo, count), "{points}");
    }
    // 0.01 pt off a chord 200 pt long: the sagitta formula r = (h² + (L/2)²) / 2h.
    let out = s.execute(C, &json!({"points": [[100, 200], [300, 200], [200, 199.99]]})).unwrap();
    let want = (0.01f64 * 0.01 + 100.0 * 100.0) / 0.02;
    let (cx, cy, r) = (out["cx"].as_f64().unwrap(), out["cy"].as_f64().unwrap(), out["radius"].as_f64().unwrap());
    assert!((r - want).abs() / want < 1e-9 && (cx - 200.0).abs() < 1e-6 && (cy - (199.99 + want)).abs() / want < 1e-9, "{out}");
}

/// Two points are the ends of a diameter: (20, 40) and (100, 100) give the circle centered at
/// (60, 70) with radius 50, through both points, in one undo step. Two points that are the same
/// are refused, and so is a circle that would reach past the canvas, from two points or from three
/// nearly on one line; nothing is drawn and no undo step is recorded.
#[test]
fn two_points_are_the_ends_of_a_diameter() {
    let mut s = session();
    let undo = undo_len(&s);
    let out = s.execute(C, &json!({"points": [[20, 40], [100, 100]]})).unwrap();
    assert_eq!((out["cx"].as_f64(), out["cy"].as_f64(), out["radius"].as_f64()), (Some(60.0), Some(70.0), Some(50.0)), "{out}");
    let id = id_of(&out);
    assert!(bounds_are(&s, id, vectorcraft_geom::Rect::new(10.0, 20.0, 110.0, 120.0)));
    assert!(farthest_from_path(&s, id, &[Point::new(20.0, 40.0), Point::new(100.0, 100.0)]) < 50.0 * 3e-4);
    assert_eq!(s.doc().unwrap().selection.objects, [id]);
    assert_eq!(undo_len(&s), undo + 1);
    let (undo, count) = (undo_len(&s), object_count(&s));
    let e = s.execute(C, &json!({"points": [[30, 30], [30, 30]]})).unwrap_err().to_string();
    assert!(e.contains("no circle passes through these points"), "{e}");
    // 4,100,000 pt across, past the canvas's 4,000,000 pt from the origin; and three points 0.001 pt
    // off a chord 200 pt long, whose circle has a radius of about 5,000,000 pt.
    for points in [json!([[0, 0], [4_100_000, 0]]), json!([[0, 0], [200, 0], [100, 0.001]])] {
        let e = s.execute(C, &json!({ "points": points })).unwrap_err().to_string();
        assert!(e.contains("result would exceed the canvas"), "{points}: {e}");
    }
    assert_eq!((undo_len(&s), object_count(&s)), (undo, count));
}

/// Points that are not two or three pairs of finite numbers are an error, not a fallback.
#[test]
fn malformed_points_are_refused() {
    let mut s = session();
    let undo = undo_len(&s);
    for points in [
        json!([[0, 0]]),
        json!([]),
        json!([[0, 0], [10]]),
        json!([[0, 0], [10, 0], [0, 10], [5, 5]]),
        json!([[0, 0], [10, 0], [0]]),
        json!([[0, 0], [10, 0], ["0", 10]]),
        json!("0,0 10,0 0,10"),
        json!([[0, 0], [10, 0], [0, 10, 3]]),
    ] {
        let e = s.execute(C, &json!({ "points": points })).unwrap_err().to_string();
        assert!(e.contains("`points` must be two or three [x, y] pairs of numbers"), "{points}: {e}");
    }
    assert_eq!(undo_len(&s), undo);
}

/// With `points` left out, the two or three anchor points selected with Direct Selection give the
/// circle, from one path or several; any other number of them is an error.
#[test]
fn three_direct_selected_anchor_points_give_the_circle() {
    let mut s = session();
    let anchors: Vec<Value> = [(0, 0), (80, 0), (0, 60), (50, 120)].iter().map(|(x, y)| json!({"x": x, "y": y})).collect();
    let path = id_of(&s.execute("path.create", &json!({ "anchors": anchors })).unwrap());
    s.execute("select.anchors", &json!({"id": path.0, "anchors": [[0, 0], [0, 1], [0, 2]], "mode": "set"})).unwrap();
    let out = s.execute(C, &json!({})).unwrap();
    assert_eq!((out["cx"].as_f64(), out["cy"].as_f64(), out["radius"].as_f64()), (Some(40.0), Some(30.0), Some(50.0)), "{out}");
    // Anchors of two paths.
    let other = id_of(&s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 60}, {"x": 0, "y": 200}]})).unwrap());
    s.execute("select.anchors", &json!({"id": path.0, "anchors": [[0, 0], [0, 1]], "mode": "set"})).unwrap();
    s.execute("select.anchors", &json!({"id": other.0, "anchors": [[0, 0]], "mode": "add"})).unwrap();
    let out = s.execute(C, &json!({"points": null})).unwrap();
    assert_eq!((out["cx"].as_f64(), out["cy"].as_f64()), (Some(40.0), Some(30.0)), "{out}");
    // Two anchors are the ends of a diameter: (0, 0) and (80, 0).
    s.execute("select.anchors", &json!({"id": path.0, "anchors": [[0, 0], [0, 1]], "mode": "set"})).unwrap();
    let out = s.execute(C, &json!({})).unwrap();
    assert_eq!((out["cx"].as_f64(), out["cy"].as_f64(), out["radius"].as_f64()), (Some(40.0), Some(0.0), Some(40.0)), "{out}");
    // One or four anchors, or a path of four anchors selected whole.
    for picked in [json!([[0, 0]]), json!([[0, 0], [0, 1], [0, 2], [0, 3]])] {
        s.execute("select.anchors", &json!({"id": path.0, "anchors": picked, "mode": "set"})).unwrap();
        let e = s.execute(C, &json!({})).unwrap_err().to_string();
        assert!(e.contains("direct-select two or three anchor points, or give `points`"), "{picked}: {e}");
    }
    s.execute("select.set", &json!({"ids": [path.0]})).unwrap();
    assert!(s.execute(C, &json!({})).is_err());
    // A path of three anchors selected whole counts all three, as the other anchor commands count
    // them; with an object that is not a path also selected, there is no circle.
    let triangle = id_of(&s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 0}, {"x": 80, "y": 0}, {"x": 0, "y": 60}]})).unwrap());
    s.execute("select.set", &json!({"ids": [triangle.0]})).unwrap();
    let out = s.execute(C, &json!({})).unwrap();
    assert_eq!((out["cx"].as_f64(), out["cy"].as_f64(), out["radius"].as_f64()), (Some(40.0), Some(30.0), Some(50.0)), "{out}");
    s.execute("edit.undo", &json!({})).unwrap();
    let text = id_of(&s.execute("text.create", &json!({"x": 300, "y": 300, "text": "A"})).unwrap());
    s.execute("select.set", &json!({"ids": [triangle.0, text.0]})).unwrap();
    assert!(s.execute(C, &json!({})).is_err());
    let undo = undo_len(&s);
    // Two selected segments hold three ends, which are not selected anchor points.
    s.execute("select.segments", &json!({"id": path.0, "segments": [[0, 0], [0, 1]], "mode": "set"})).unwrap();
    assert_eq!(s.doc().unwrap().selection.anchors.get(&path).map(|a| a.len()), Some(3));
    let e = s.execute(C, &json!({})).unwrap_err().to_string();
    assert!(e.contains("direct-select two or three anchor points, or give `points`"), "{e}");
    assert_eq!(undo_len(&s), undo);
}

/// With Align New Objects to Pixel Grid on, the circle is still drawn through the points: it is
/// not pixel-aligned, so the grid does not move it.
#[test]
fn the_circle_is_not_moved_onto_the_pixel_grid() {
    let mut s = session();
    s.execute("document.setup", &json!({"alignNewObjectsToPixelGrid": true})).unwrap();
    let points = [Point::new(0.3, 0.2), Point::new(80.6, 0.1), Point::new(0.4, 60.7)];
    let pairs: Vec<Value> = points.iter().map(|p| json!([p.x, p.y])).collect();
    let id = id_of(&s.execute(C, &json!({ "points": pairs })).unwrap());
    assert!(farthest_from_path(&s, id, &points) < 51.0 * 3e-4, "{}", farthest_from_path(&s, id, &points));
    assert!(!node(&s, id).extras().pixel_aligned);
}

fn view() -> ViewInfo {
    ViewInfo { smart_guides: false, ..ViewInfo::default() }
}

fn click(s: &mut Session, mods: Mods, x: f64, y: f64) {
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, x, y).with_mods(mods), view()).unwrap();
    }
}

/// With the Ellipse tool, three Shift-clicks draw the circle through the three points, in one
/// undo step; Escape drops the points placed so far.
#[test]
fn three_shift_clicks_with_the_ellipse_tool_draw_the_circle_through_them() {
    let mut s = session();
    s.select_tool("ellipse", view()).unwrap();
    let shift = Mods { shift: true, ..Mods::default() };
    let (undo, count) = (undo_len(&s), object_count(&s));
    click(&mut s, shift, 100.0, 100.0);
    click(&mut s, shift, 180.0, 100.0);
    assert_eq!((undo_len(&s), object_count(&s)), (undo, count), "two points draw nothing yet");
    click(&mut s, shift, 100.0, 160.0);
    assert_eq!(undo_len(&s), undo + 1);
    let id = *s.doc().unwrap().selection.objects.first().unwrap();
    assert!(bounds_are(&s, id, vectorcraft_geom::Rect::new(90.0, 80.0, 190.0, 180.0)));
    // Escape after two points: the next three clicks start afresh.
    click(&mut s, shift, 0.0, 0.0);
    click(&mut s, shift, 10.0, 300.0);
    s.tool_key(ToolKey::Escape, Mods::default(), view()).unwrap();
    click(&mut s, shift, 200.0, 200.0);
    click(&mut s, shift, 280.0, 200.0);
    let undo = undo_len(&s);
    click(&mut s, shift, 200.0, 260.0);
    assert_eq!(undo_len(&s), undo + 1);
    let id = *s.doc().unwrap().selection.objects.first().unwrap();
    assert!(bounds_are(&s, id, vectorcraft_geom::Rect::new(190.0, 180.0, 290.0, 280.0)));
}
