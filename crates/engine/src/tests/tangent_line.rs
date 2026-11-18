//! Snap to Tangent with the Line Segment tool: a line dragged from one circle to another lands on
//! their common tangent, drawn by `shape.line` as one undo step.

use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_geom::{Point, Vec2};
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::*;

use PointerKind::{Down, Drag, Up};

#[test]
fn a_line_segment_drag_draws_the_common_tangent_as_one_undo_step() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 500, "height": 500})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 60, "y": 210, "width": 80, "height": 80})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 240, "y": 190, "width": 120, "height": 120})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let count = |s: &Session| s.doc().unwrap().doc.layers[0].children().map_or(0, |c| c.len());
    assert_eq!(count(&s), 2);
    // Circles about (100, 250), radius 40, and (300, 250), radius 60: the outer tangent above them
    // has the normal (-0.1, -√0.99) and touches them at center + radius × normal.
    let n = Vec2::new(-0.1, -(0.99f64).sqrt());
    let (on_a, on_b) = (Point::new(100.0, 250.0) + n * 40.0, Point::new(300.0, 250.0) + n * 60.0);
    s.select_tool("lineSegment", ViewInfo::default()).unwrap();
    let to = on_b + Vec2::new(0.5, 0.7);
    for (k, x, y) in [(Down, 100.0, 210.0), (Drag, 200.0, 200.0), (Drag, to.x, to.y), (Up, to.x, to.y)] {
        s.pointer(&PointerEvent::new(k, x, y), ViewInfo::default()).unwrap();
    }
    assert_eq!(count(&s), 3);
    let id = *s.doc().unwrap().selection.objects.first().unwrap();
    let NodeKind::Path { path, .. } = &s.doc().unwrap().doc.node(id).unwrap().kind else { panic!("a path") };
    let ends: Vec<Point> = path.subpaths[0].anchors.iter().map(|a| a.p).collect();
    assert_eq!(ends.len(), 2);
    // Within what the drawn circles' 0.03% radius error allows.
    assert!((ends[0] - on_a).hypot() < 0.15 && (ends[1] - on_b).hypot() < 0.15, "{ends:?}, want {on_a:?} {on_b:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(count(&s), 2, "one undo removes the line");
}
