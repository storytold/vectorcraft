//! Scissors receives the view's Smart Guides setting and keeps the split endpoint selected.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::{Session, ViewInfo};
use vectorcraft_geom::{PathData, Point};
use vectorcraft_tools::{Overlay, PointerEvent, PointerKind};

fn path(session: &Session, id: NodeId) -> PathData {
    session.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().clone()
}

/// Pixel Preview disables Smart Guides at the Session boundary, rather than asking the tool to
/// interpret that view option. A selected rectangle wins the cut over the crossing line above it.
#[test]
fn scissors_snap_follows_pixel_preview_and_restores_the_cut_selection() {
    let mut observed = Vec::new();
    for pixel_preview in [false, true] {
        let mut session = Session::new();
        session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
        let rectangle =
            NodeId(session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
        let crossing = NodeId(session.execute("shape.line", &json!({"x1": 145, "y1": 90, "x2": 165, "y2": 130})).unwrap()["id"].as_u64().unwrap());
        session.execute("select.set", &json!({"ids": [rectangle.0]})).unwrap();
        let view = ViewInfo { pixel_preview, ..ViewInfo::default() };
        session.select_tool("scissors", view).unwrap();
        let original = path(&session, rectangle);
        let line = path(&session, crossing);
        let selection_before = session.doc().unwrap().selection.clone();
        let undo_before = session.doc().unwrap().history.undo.len();
        assert!(original.is_closed());

        session.pointer(&PointerEvent::new(PointerKind::Move, 152.0, 101.5), view).unwrap();
        let intersection = session.overlays(view).iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "intersect"));
        for kind in [PointerKind::Down, PointerKind::Up] {
            session.pointer(&PointerEvent::new(kind, 152.0, 101.5), view).unwrap();
        }

        let opened = path(&session, rectangle);
        assert_eq!(opened.subpaths.len(), 1);
        assert!(!opened.is_closed());
        let anchors = &opened.subpaths[0].anchors;
        assert_eq!(anchors.len(), 6);
        let cut = anchors.first().unwrap().p;
        assert_eq!(anchors.last().unwrap().p, cut, "the cut ends coincide");
        let selected = &session.doc().unwrap().selection;
        assert_eq!(selected.objects, [rectangle]);
        assert_eq!(selected.anchors.len(), 1);
        assert_eq!(selected.partial(rectangle).unwrap().iter().copied().collect::<Vec<_>>(), [(0, 0)]);
        assert!(selected.segments.is_empty(), "one cut endpoint, not a segment, is selected");
        let selection_after = selected.clone();
        assert_eq!(path(&session, crossing), line);
        assert_eq!(session.doc().unwrap().history.undo.len(), undo_before + 1);

        session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(path(&session, rectangle), original);
        assert_eq!(path(&session, crossing), line);
        assert_eq!(session.doc().unwrap().selection, selection_before);
        assert_eq!(session.doc().unwrap().history.undo.len(), undo_before);
        session.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(path(&session, rectangle), opened);
        assert_eq!(path(&session, crossing), line);
        assert_eq!(session.doc().unwrap().selection, selection_after);
        assert_eq!(session.doc().unwrap().history.undo.len(), undo_before + 1);
        observed.push((pixel_preview, cut, intersection));
    }

    // Observe both producer rows before checking the original failure: without Scissors snapping,
    // Smart Guides on cuts at (152, 100), rather than the intersection at (150, 100).
    for (pixel_preview, cut, intersection) in observed {
        let expected = Point::new(if pixel_preview { 152.0 } else { 150.0 }, 100.0);
        assert!(cut.distance(expected) < 1e-6, "Pixel Preview {pixel_preview}: cut {cut:?}, expected {expected:?}");
        assert_eq!(intersection, !pixel_preview, "Pixel Preview {pixel_preview}: intersection feedback");
    }
}
