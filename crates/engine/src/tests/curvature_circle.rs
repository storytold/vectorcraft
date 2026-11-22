//! `shape.curvatureCircle`: the circle that touches a path at a point and bends as the path bends
//! there, and the Curvature Circle tool that runs it.

use serde_json::{Value, json};
use vectorcraft_doc::LiveShape;
use vectorcraft_geom::{Point, Rect};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use crate::*;

const C: &str = "shape.curvatureCircle";

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
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

fn center_of(v: &Value) -> Point {
    Point::new(v["cx"].as_f64().unwrap(), v["cy"].as_f64().unwrap())
}

/// Whether the geometric bounds of `id` are `want` to within 10⁻⁶ pt.
fn bounds_are(s: &Session, id: NodeId, want: Rect) -> bool {
    let b = s.doc().unwrap().doc.bounds_of(&[id], false).unwrap();
    [(b.x0, want.x0), (b.y0, want.y0), (b.x1, want.x1), (b.y1, want.y1)].iter().all(|(a, w)| (a - w).abs() < 1e-6)
}

/// A circle of radius 100 centered at (200, 200).
fn circle(s: &mut Session) -> NodeId {
    id_of(&s.execute("shape.ellipse", &json!({"x": 100, "y": 100, "width": 200, "height": 200})).unwrap())
}

/// The parabola y = 200 + (x − 200)² / 200 from x = 100 to 300, one cubic: its vertex is at
/// (200, 200), where it bends with radius 100 about (200, 300).
fn parabola(s: &mut Session) -> NodeId {
    let (a, b) = (166.666_666_666_666_66, 233.333_333_333_333_34);
    let d = format!("M100 250 C{a} 183.333333333333334 {b} 183.333333333333334 300 250");
    id_of(&s.execute("path.create", &json!({ "d": d })).unwrap())
}

/// On a circle, the curvature circle is the circle itself: a live ellipse with the current paint,
/// selected, in one undo step. On the other side, it is the same size and touches the circle from
/// outside.
#[test]
fn the_curvature_circle_of_a_circle_is_the_circle() {
    let mut s = session();
    let path = circle(&mut s);
    let (undo, count) = (undo_len(&s), object_count(&s));
    for seg in 0..4 {
        let v = s.execute(C, &json!({"id": path.0, "segment": seg, "t": 0.5})).unwrap();
        assert!(v["nominal"].as_bool().unwrap() && (v["radius"].as_f64().unwrap() - 100.0).abs() < 1e-9, "{v}");
        assert!(center_of(&v).distance(Point::new(200.0, 200.0)) < 1e-9, "{v}");
        let id = id_of(&v);
        assert!(bounds_are(&s, id, Rect::new(100.0, 100.0, 300.0, 300.0)));
        let node = s.doc().unwrap().doc.node(id).unwrap().clone();
        let vectorcraft_doc::NodeKind::Path { live: Some(live), .. } = &node.kind else { panic!("a live shape: {node:?}") };
        assert!(matches!(**live, LiveShape::Ellipse { .. }));
        assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
    }
    assert_eq!((undo_len(&s), object_count(&s)), (undo + 4, count + 4));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(object_count(&s), count + 3);
    let v = s.execute(C, &json!({"id": path.0, "segment": 1, "t": 0.5, "convex": true})).unwrap();
    assert!((center_of(&v).distance(Point::new(200.0, 200.0)) - 200.0).abs() < 1e-9 && (v["radius"].as_f64().unwrap() - 100.0).abs() < 1e-9, "{v}");
}

/// At the vertex of a parabola, the circle has the parabola's curvature, radius 100, on the side
/// it bends toward, or on the other side with `convex`.
#[test]
fn the_curvature_circle_at_a_parabola_vertex() {
    let mut s = session();
    let path = parabola(&mut s);
    let v = s.execute(C, &json!({"id": path.0, "segment": 0, "t": 0.5})).unwrap();
    assert!(!v["nominal"].as_bool().unwrap() && (v["radius"].as_f64().unwrap() - 100.0).abs() < 1e-6, "{v}");
    assert!(center_of(&v).distance(Point::new(200.0, 300.0)) < 1e-6, "{v}");
    let v = s.execute(C, &json!({"id": path.0, "segment": 0, "t": 0.5, "convex": true})).unwrap();
    assert!(center_of(&v).distance(Point::new(200.0, 100.0)) < 1e-6, "{v}");
}

/// A straight segment, a cusp, a corner anchor and a bend so slight that its circle would reach
/// past the canvas are errors, and nothing is drawn.
#[test]
fn straight_segments_cusps_and_huge_circles_are_refused() {
    let mut s = session();
    let line = id_of(&s.execute("shape.line", &json!({"x1": 10, "y1": 20, "x2": 130, "y2": 70})).unwrap());
    let cusp = id_of(&s.execute("path.create", &json!({"d": "M0 0 C100 100 0 100 100 0"})).unwrap());
    let gentle = id_of(&s.execute("path.create", &json!({"d": "M0 0 C100 0.0000001 200 0.0000001 300 0"})).unwrap());
    // A curve that meets a line at a corner anchor (150, 0).
    let corner = id_of(&s.execute("path.create", &json!({"d": "M0 0 C50 -50 100 -50 150 0 L300 0"})).unwrap());
    let (undo, count) = (undo_len(&s), object_count(&s));
    for (id, t, want) in [
        (line, 0.5, "the path is straight there, so no circle matches its curvature"),
        (cusp, 0.5, "the path has a cusp there, so it has no single curvature"),
        (corner, 1.0, "the path has a corner there, so it has no single curvature"),
        (gentle, 0.5, "the circle that matches the path there would reach past the canvas"),
    ] {
        let e = s.execute(C, &json!({"id": id.0, "segment": 0, "t": t})).unwrap_err().to_string();
        assert!(e.contains(want), "{e}");
    }
    assert_eq!((undo_len(&s), object_count(&s)), (undo, count));
}

/// Params that are missing or out of range are errors, and nothing is drawn.
#[test]
fn bad_params_are_refused() {
    let mut s = session();
    let path = circle(&mut s);
    let (undo, count) = (undo_len(&s), object_count(&s));
    for (p, want) in [
        (json!({"segment": 0, "t": 0.5}), "missing id"),
        (json!({"id": path.0, "t": 0.5}), "missing segment"),
        (json!({"id": path.0, "segment": 0}), "missing number `t`"),
        (json!({"id": path.0, "segment": 0, "t": 1.5}), "`t` must be from 0 to 1"),
        (json!({"id": path.0, "segment": 0, "t": -0.1}), "`t` must be from 0 to 1"),
        (json!({"id": path.0, "segment": 0, "t": "0.5"}), "`t` must be a number"),
        (json!({"id": path.0, "segment": 4, "t": 0.5}), "no such segment"),
        (json!({"id": path.0, "subpath": 1, "segment": 0, "t": 0.5}), "no such subpath"),
        (json!({"id": path.0, "segment": 0, "t": 0.5, "convex": "yes"}), "`convex` must be true or false"),
    ] {
        let e = s.execute(C, &p).unwrap_err().to_string();
        assert!(e.contains(want), "{p}: {e}");
    }
    assert!(s.execute(C, &json!({"id": 9999, "segment": 0, "t": 0.5})).is_err());
    assert_eq!((undo_len(&s), object_count(&s)), (undo, count));
}

/// A guide has no curvature circle, as the tool leaves guides out; an error names the command.
#[test]
fn a_guide_is_refused() {
    let mut s = session();
    let path = circle(&mut s);
    s.execute("view.guides.make", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(path).unwrap().is_guide());
    let (undo, count) = (undo_len(&s), object_count(&s));
    let e = s.execute(C, &json!({"id": path.0, "segment": 0, "t": 0.5})).unwrap_err().to_string();
    assert!(e.contains("a guide has no curvature circle") && e.contains(C), "{e}");
    assert_eq!((undo_len(&s), object_count(&s)), (undo, count));
}

/// With Align New Objects to Pixel Grid on, the circle still touches the path where it was asked:
/// it is not pixel-aligned, so the grid does not move it.
#[test]
fn the_circle_is_not_moved_onto_the_pixel_grid() {
    let mut s = session();
    s.execute("document.setup", &json!({"alignNewObjectsToPixelGrid": true})).unwrap();
    let path = id_of(&s.execute("shape.ellipse", &json!({"x": 100.3, "y": 100.2, "width": 200.4, "height": 200.4})).unwrap());
    let v = s.execute(C, &json!({"id": path.0, "segment": 0, "t": 0.5})).unwrap();
    let (c, r) = (center_of(&v), v["radius"].as_f64().unwrap());
    let id = id_of(&v);
    assert!(bounds_are(&s, id, Rect::new(c.x - r, c.y - r, c.x + r, c.y + r)), "{v}");
    assert!(!s.doc().unwrap().doc.node(id).unwrap().extras().pixel_aligned);
}

fn view() -> ViewInfo {
    ViewInfo { smart_guides: false, ..ViewInfo::default() }
}

fn click(s: &mut Session, mods: Mods, x: f64, y: f64) {
    for k in [PointerKind::Move, PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, x, y).with_mods(mods), view()).unwrap();
    }
}

/// The path the last `shape.curvatureCircle` drew on.
fn drawn_on(s: &Session) -> NodeId {
    let (cmd, p) = s.journal.last().unwrap().clone();
    assert_eq!(cmd, C);
    id_of(&p)
}

/// With the Curvature Circle tool, a click on a path draws the circle that matches it there, in
/// one undo step; with Shift, on the other side. A click on a straight line or away from every
/// path draws nothing.
#[test]
fn the_curvature_circle_tool_draws_the_circle_where_it_clicks() {
    let mut s = session();
    let path = circle(&mut s);
    s.execute("shape.line", &json!({"x1": 10, "y1": 380, "x2": 390, "y2": 380})).unwrap();
    s.select_tool("curvatureCircle", view()).unwrap();
    let (undo, count) = (undo_len(&s), object_count(&s));
    click(&mut s, Mods::default(), 301.0, 200.0);
    assert_eq!(undo_len(&s), undo + 1);
    let id = *s.doc().unwrap().selection.objects.first().unwrap();
    assert_ne!(id, path);
    assert!(bounds_are(&s, id, Rect::new(100.0, 100.0, 300.0, 300.0)));
    let shift = Mods { shift: true, ..Mods::default() };
    click(&mut s, shift, 300.0, 200.0);
    let id = *s.doc().unwrap().selection.objects.first().unwrap();
    assert!(bounds_are(&s, id, Rect::new(300.0, 100.0, 500.0, 300.0)));
    assert_eq!(object_count(&s), count + 2);
    click(&mut s, Mods::default(), 200.0, 381.0);
    click(&mut s, Mods::default(), 20.0, 20.0);
    assert_eq!((undo_len(&s), object_count(&s)), (undo + 2, count + 2));
}

/// After a click at a parabola's vertex, a click at (280, 236), 2.4 pt from the circle just drawn
/// and about 3.1 pt from the parabola, passes over that circle and draws the parabola's circle
/// there; after an Undo, the pointer reaches the circle again.
#[test]
fn the_next_click_passes_over_the_circle_the_tool_drew() {
    let mut s = session();
    parabola(&mut s);
    s.select_tool("curvatureCircle", view()).unwrap();
    click(&mut s, Mods::default(), 200.0, 200.0);
    let first = *s.doc().unwrap().selection.objects.first().unwrap();
    assert!(bounds_are(&s, first, Rect::new(100.0, 200.0, 300.0, 400.0)));
    click(&mut s, Mods::default(), 280.0, 236.0);
    let second = *s.doc().unwrap().selection.objects.first().unwrap();
    assert_ne!(second, first);
    let r = s.doc().unwrap().doc.bounds_of(&[second], false).unwrap().width() / 2.0;
    assert!(r > 150.0, "the parabola's circle at x = 280 is larger than at its vertex: {r}");
    s.execute("edit.undo", &json!({})).unwrap();
    let count = object_count(&s);
    click(&mut s, Mods::default(), 280.0, 241.0);
    assert_eq!(object_count(&s), count + 1, "the circle drawn first is within reach again");
    let third = *s.doc().unwrap().selection.objects.first().unwrap();
    assert!(bounds_are(&s, third, Rect::new(100.0, 200.0, 300.0, 400.0)), "the click drew the first circle's own circle");
    assert_eq!(drawn_on(&s), first, "the click hit the first circle");
}

/// After Deselect, the pointer reaches the circle the tool drew again, with no edit in between,
/// and still does when the circle is selected again before the pointer moves.
#[test]
fn after_deselect_the_pointer_reaches_the_circle_the_tool_drew() {
    for reselect in [false, true] {
        let mut s = session();
        let path = parabola(&mut s);
        s.select_tool("curvatureCircle", view()).unwrap();
        click(&mut s, Mods::default(), 200.0, 200.0);
        let first = *s.doc().unwrap().selection.objects.first().unwrap();
        assert_eq!(drawn_on(&s), path);
        let undo = undo_len(&s);
        s.execute("select.none", &json!({})).unwrap();
        if reselect {
            s.execute("select.set", &json!({"ids": [first.0]})).unwrap();
        }
        assert_eq!(undo_len(&s), undo, "Deselect is not an edit");
        click(&mut s, Mods::default(), 280.0, 236.0);
        assert_eq!(drawn_on(&s), first, "the click hit the circle drawn first ({reselect})");
    }
}
