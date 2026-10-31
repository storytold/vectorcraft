//! A click after a smooth anchor: the new corner has no handles, so the segment curves from the previous anchor's side only.

use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::{Session, ViewInfo};
use vectorcraft_geom::Point;
use vectorcraft_tools::{PointerEvent, PointerKind};

#[test]
fn click_after_a_smooth_anchor_curves_from_its_side_only() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let view = ViewInfo { zoom: 1.0, smart_guides: false, snap_to_grid: false, snap_to_pixel: false, snap_to_point: false, ..Default::default() };
    s.select_tool("pen", view).unwrap();
    let mut at = |kind, x, y| s.pointer(&PointerEvent::new(kind, x, y), view).unwrap();
    // Press and drag out a smooth anchor at (100, 100), then click a corner at (300, 100).
    at(PointerKind::Down, 100.0, 100.0);
    at(PointerKind::Drag, 150.0, 60.0);
    at(PointerKind::Up, 150.0, 60.0);
    at(PointerKind::Down, 300.0, 100.0);
    at(PointerKind::Up, 300.0, 100.0);

    let mut path = None;
    s.doc().unwrap().doc.walk(|n| {
        if let NodeKind::Path { path: p, .. } = &n.kind {
            path = Some(p.clone());
        }
    });
    let sp = &path.unwrap().subpaths[0];
    let (a, b) = (&sp.anchors[0], &sp.anchors[1]);
    assert_eq!(a.h_out, Point::new(150.0, 60.0), "the outgoing handle stays where the drag left it");
    assert_eq!(a.h_in, Point::new(50.0, 140.0), "mirrored through the anchor");
    assert!(!b.has_in() && !b.has_out(), "a click places a corner without handles");
    assert!(!sp.segment_is_line(0), "the segment is a curve, shaped by the previous anchor's handle only");
    assert_eq!(sp.segment(0).p2, b.p);
}
