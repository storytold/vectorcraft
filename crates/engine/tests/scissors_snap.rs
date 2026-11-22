//! Scissors receives the view's Smart Guides setting and keeps the split endpoint selected.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::{Session, ViewInfo};
use vectorcraft_geom::{Affine, PathData, Point};
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

/// A command that ends an actual drag must settle its deferred move before Scissors reads the
/// document. Real-time drawing is the control: both routes cut the same moved geometry.
#[test]
fn scissors_cuts_after_a_command_settles_a_deferred_move() {
    for real_time_drawing in [true, false] {
        let mut session = Session::new();
        session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
        let rectangle =
            NodeId(session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
        let crossing = NodeId(session.execute("shape.line", &json!({"x1": 185, "y1": 110, "x2": 205, "y2": 150})).unwrap()["id"].as_u64().unwrap());
        session.execute("select.set", &json!({"ids": [rectangle.0]})).unwrap();
        session.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": real_time_drawing})).unwrap();
        let drag_view = ViewInfo { smart_guides: false, snap_to_point: false, snap_to_grid: false, snap_to_pixel: false, ..ViewInfo::default() };
        session.select_tool("selection", drag_view).unwrap();
        let original = path(&session, rectangle);
        let moved = original.transformed(Affine::translate((40.0, 20.0)));
        let line = path(&session, crossing);
        let undo_before = session.doc().unwrap().history.undo.len();

        session.pointer(&PointerEvent::new(PointerKind::Down, 150.0, 150.0), drag_view).unwrap();
        session.pointer(&PointerEvent::new(PointerKind::Drag, 190.0, 170.0), drag_view).unwrap();
        assert!(session.in_interaction(), "the pointer drag is still held");
        if real_time_drawing {
            assert_eq!(path(&session, rectangle), moved);
            assert!(!session.interaction_unapplied(), "the real-time preview is already applied");
        } else {
            assert_eq!(path(&session, rectangle), original, "the non-real-time drag has not changed the stored geometry yet");
            assert!(session.interaction_unapplied(), "an actual Selection move is pending");
            let state = session.doc().unwrap();
            let (command, params) = state.interaction.as_ref().unwrap().preview.as_ref().unwrap();
            assert_eq!(command, "object.transform");
            let matrix: Vec<f64> = params["matrix"].as_array().unwrap().iter().map(|value| value.as_f64().unwrap()).collect();
            assert_eq!(matrix, [1.0, 0.0, 0.0, 1.0, 40.0, 20.0], "the pending move keeps its exact translation");
        }
        // command.batch settles the interaction before its steps; selecting the same object
        // changes no geometry and adds no separate undo step.
        session
            .execute(
                "command.batch",
                &json!({
                    "label": "Keep selection", "commands": [{"command": "select.set", "params": {"ids": [rectangle.0]}}]
                }),
            )
            .unwrap();
        assert!(!session.in_interaction());
        assert_eq!(path(&session, rectangle), moved, "the command keeps the pending move: real-time={real_time_drawing}");
        assert!(path(&session, rectangle).is_closed());
        assert_eq!(path(&session, crossing), line);
        assert_eq!(session.doc().unwrap().history.undo.len(), undo_before + 1);

        // Finish the physical gesture before changing tools. Releasing after the command has
        // settled it must neither repeat the transform nor discard its committed geometry.
        session.pointer(&PointerEvent::new(PointerKind::Up, 190.0, 170.0), drag_view).unwrap();
        assert!(!session.in_interaction());
        assert_eq!(path(&session, rectangle), moved, "release keeps exactly one move");
        assert_eq!(path(&session, crossing), line);
        assert_eq!(session.doc().unwrap().history.undo.len(), undo_before + 1);

        let cut_view = ViewInfo::default();
        session.select_tool("scissors", cut_view).unwrap();
        session.pointer(&PointerEvent::new(PointerKind::Move, 192.0, 121.5), cut_view).unwrap();
        for kind in [PointerKind::Down, PointerKind::Up] {
            session.pointer(&PointerEvent::new(kind, 192.0, 121.5), cut_view).unwrap();
        }
        let opened = path(&session, rectangle);
        assert!(!opened.is_closed());
        let anchors = &opened.subpaths[0].anchors;
        let cut = anchors.first().unwrap().p;
        assert!(cut.distance(Point::new(190.0, 120.0)) < 1e-6, "the current moved path snaps to its intersection");
        assert_eq!(anchors.last().unwrap().p, cut);
        assert_eq!(path(&session, crossing), line);
        assert_eq!(session.doc().unwrap().history.undo.len(), undo_before + 2);
        session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(path(&session, rectangle), moved, "undoing the cut retains the earlier settled move");
        assert_eq!(path(&session, crossing), line);
        assert_eq!(session.doc().unwrap().selection.objects, [rectangle]);
        assert_eq!(session.doc().unwrap().history.undo.len(), undo_before + 1);
    }
}

/// Real Session composition: the hovered feedback must be the endpoint the actual split creates,
/// while the selected crossing owner, history and Undo/Redo contracts remain intact.
#[test]
fn review_8655_scissors_session_feedback_is_the_committed_cut_endpoint() {
    let mut observed = Vec::new();
    for pixel_preview in [true, false] {
        let mut session = Session::new();
        session.execute("file.new", &json!({"width":800,"height":600})).unwrap();
        let rectangle =
            NodeId(session.execute("shape.rectangle", &json!({"x":100,"y":100,"width":100,"height":100})).unwrap()["id"].as_u64().unwrap());
        let crossing = NodeId(session.execute("shape.line", &json!({"x1":145,"y1":90,"x2":165,"y2":130})).unwrap()["id"].as_u64().unwrap());
        session.execute("select.set", &json!({"ids":[rectangle.0]})).unwrap();
        let view = ViewInfo { pixel_preview, ..ViewInfo::default() };
        session.select_tool("scissors", view).unwrap();
        let original = path(&session, rectangle);
        let other = path(&session, crossing);
        let selection_before = session.doc().unwrap().selection.clone();
        let undo_before = session.doc().unwrap().history.undo.len();
        let raw = Point::new(152.0, 101.5);
        session.pointer(&PointerEvent::new(PointerKind::Move, raw.x, raw.y), view).unwrap();
        let overlays = session.overlays(view);
        let markers: Vec<_> = overlays
            .iter()
            .filter_map(|o| match o {
                Overlay::Anchor { p, color, filled: false, size } if *color == vectorcraft_tools::draw2::FEEDBACK && *size == 7.0 => Some(*p),
                _ => None,
            })
            .collect();
        let intersection = overlays.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "intersect"));
        assert_eq!(intersection, !pixel_preview, "CONTROL: native view boundary retains Smart Guides policy");
        for kind in [PointerKind::Down, PointerKind::Up] {
            session.pointer(&PointerEvent::new(kind, raw.x, raw.y), view).unwrap();
        }
        let opened = path(&session, rectangle);
        assert!(!opened.is_closed());
        let anchors = &opened.subpaths[0].anchors;
        assert_eq!(anchors.len(), 6);
        let cut = anchors[0].p;
        assert_eq!(anchors.last().unwrap().p, cut);
        let expected = Point::new(if pixel_preview { 152.0 } else { 150.0 }, 100.0);
        assert!(cut.distance(expected) < 1e-6, "CONTROL: actual cut geometry respects view snapping");
        assert!(cut.distance(raw) > 1.0, "CONTROL: feedback is projected to the path, not raw input");
        let selection_after = session.doc().unwrap().selection.clone();
        assert_eq!(selection_after.objects, [rectangle]);
        assert_eq!(selection_after.partial(rectangle).unwrap().iter().copied().collect::<Vec<_>>(), [(0, 0)]);
        assert!(selection_after.segments.is_empty());
        assert_eq!(path(&session, crossing), other);
        assert_eq!(session.doc().unwrap().history.undo.len(), undo_before + 1);
        assert!(
            session.overlays(view).iter().all(|o| !matches!(o, Overlay::Anchor { color, filled:false, size, .. }
            if *color == vectorcraft_tools::draw2::FEEDBACK && *size == 7.0)),
            "CONTROL: Up clears cut feedback"
        );
        session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(path(&session, rectangle), original);
        assert_eq!(path(&session, crossing), other);
        assert_eq!(session.doc().unwrap().selection, selection_before);
        assert_eq!(session.doc().unwrap().history.undo.len(), undo_before);
        session.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(path(&session, rectangle), opened);
        assert_eq!(path(&session, crossing), other);
        assert_eq!(session.doc().unwrap().selection, selection_after);
        assert_eq!(session.doc().unwrap().history.undo.len(), undo_before + 1);
        observed.push((pixel_preview, markers, cut));
    }
    let endpoint_controls = review_8655_session_endpoint_controls();
    for (pixel_preview, markers, cut) in observed {
        assert_eq!(markers.len(), 1, "REGRESSION: Pixel Preview{pixel_preview} needs one hollow7px FEEDBACK anchor");
        assert!(markers[0].distance(cut) < 1e-6, "REGRESSION: preview{markers:?} must mark the actual committed endpoint{cut:?}");
    }
    for (markers, cut) in endpoint_controls {
        assert_eq!(markers.len(), 1, "a successful interior/canonicalized cut has one marker");
        assert!(markers[0].distance(cut) < 1e-6, "marker{markers:?} must equal the actual Engine cut{cut:?}");
    }
}

fn review_8655_session_markers(overlays: &[Overlay]) -> Vec<Point> {
    overlays
        .iter()
        .filter_map(|o| match o {
            Overlay::Anchor { p, color, filled: false, size } if *color == vectorcraft_tools::draw2::FEEDBACK && *size == 7.0 => Some(*p),
            _ => None,
        })
        .collect()
}

fn review_8655_session_endpoint_controls() -> Vec<(Vec<Point>, Point)> {
    let mut session = Session::new();
    session.execute("file.new", &json!({"width":800,"height":600})).unwrap();
    let line = NodeId(session.execute("shape.line", &json!({"x1":100,"y1":300,"x2":200,"y2":300})).unwrap()["id"].as_u64().unwrap());
    session.execute("select.set", &json!({"ids":[line.0]})).unwrap();
    let view = ViewInfo { smart_guides: false, snap_to_point: false, snap_to_grid: false, snap_to_pixel: false, ..ViewInfo::default() };
    session.select_tool("scissors", view).unwrap();
    let original = path(&session, line);
    let document = json!(session.doc().unwrap().doc.as_ref());
    let selection = session.doc().unwrap().selection.clone();
    let undo = session.doc().unwrap().history.undo.len();
    let redo = session.doc().unwrap().history.redo.len();
    assert!(!original.is_closed());
    assert_eq!(original.subpaths[0].anchors.len(), 2, "CONTROL: ordinary minimal drawable open line");
    for endpoint in [Point::new(100.0, 300.0), Point::new(200.0, 300.0)] {
        session.pointer(&PointerEvent::new(PointerKind::Move, endpoint.x, endpoint.y), view).unwrap();
        let endpoint_markers = review_8655_session_markers(&session.overlays(view));
        let error = session.pointer(&PointerEvent::new(PointerKind::Down, endpoint.x, endpoint.y), view).unwrap_err();
        assert!(error.to_string().contains("cannot split an open path at its end point"), "CONTROL: preserve baseline endpoint error: {error}");
        assert_eq!(json!(session.doc().unwrap().doc.as_ref()), document, "a failed endpoint cut changes no document data");
        assert_eq!(session.doc().unwrap().selection, selection);
        assert_eq!(session.doc().unwrap().history.undo.len(), undo);
        assert_eq!(session.doc().unwrap().history.redo.len(), redo);
        assert!(endpoint_markers.is_empty(), "an endpoint that Engine rejects has no successful-cut feedback");
        session.pointer(&PointerEvent::new(PointerKind::Up, endpoint.x, endpoint.y), view).unwrap();
    }
    let interior = Point::new(150.0, 300.0);
    session.pointer(&PointerEvent::new(PointerKind::Move, interior.x, interior.y), view).unwrap();
    let interior_markers = review_8655_session_markers(&session.overlays(view));
    for kind in [PointerKind::Down, PointerKind::Up] {
        session.pointer(&PointerEvent::new(kind, interior.x, interior.y), view).unwrap();
    }
    let right_id = session.doc().unwrap().selection.objects[0];
    assert_ne!(right_id, line);
    let right = path(&session, right_id);
    let left = path(&session, line);
    let actual_interior = right.subpaths[0].anchors[0].p;
    assert!(actual_interior.distance(interior) < 1e-6);
    assert_eq!(left.subpaths[0].anchors.last().unwrap().p, actual_interior);
    assert_eq!(session.doc().unwrap().history.undo.len(), undo + 1);
    session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(path(&session, line), original);
    assert!(session.doc().unwrap().doc.node(right_id).is_none());
    assert_eq!(session.doc().unwrap().selection, selection);

    let (canonical_markers, canonical) = review_8655_session_canonical_control(session, view);
    vec![(interior_markers, actual_interior), (canonical_markers, canonical)]
}

fn review_8655_session_canonical_control(mut session: Session, view: ViewInfo) -> (Vec<Point>, Point) {
    let closed = NodeId(
        session
            .execute(
                "path.create",
                &json!({"d":"M99.99995 100 C133.33328333333333 100 166.66661666666666 100 199.99995 100 L199.99995 200 L99.99995 200 Z"}),
            )
            .unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    let mut prefs = session.prefs.clone();
    prefs.gridline_every = 10.0;
    prefs.grid_subdivisions = 1;
    session.apply_prefs(prefs);
    assert_eq!(session.doc().unwrap().doc.grid.spacing, 10.0);
    assert_eq!(session.doc().unwrap().doc.grid.subdivisions, 1);
    session.execute("select.set", &json!({"ids":[closed.0]})).unwrap();
    let grid = ViewInfo { snap_to_grid: true, ..view };
    let original_closed = path(&session, closed);
    assert!(original_closed.is_closed());
    let canonical = original_closed.subpaths[0].anchors[0].p;
    let (_, segment, t, projected, _) = original_closed.nearest(Point::new(100.0, 100.0)).unwrap();
    assert_eq!(segment, 0);
    assert!(t > 0.0 && t <= 1e-6, "CONTROL: stored native cubic reaches actual canonicalization threshold: {t}");
    assert!(projected.distance(canonical) > 1e-5, "CONTROL: projected point differs from canonical stored anchor");
    let raw = Point::new(100.1, 100.2);
    session.pointer(&PointerEvent::new(PointerKind::Move, raw.x, raw.y), grid).unwrap();
    let canonical_markers = review_8655_session_markers(&session.overlays(grid));
    let before = session.doc().unwrap().history.undo.len();
    for kind in [PointerKind::Down, PointerKind::Up] {
        session.pointer(&PointerEvent::new(kind, raw.x, raw.y), grid).unwrap();
    }
    let opened = path(&session, closed);
    assert!(!opened.is_closed());
    let anchors = &opened.subpaths[0].anchors;
    assert_eq!(anchors.first().unwrap().p, canonical, "Engine commits the original anchor rather than curve.eval(t)");
    assert_eq!(anchors.last().unwrap().p, canonical);
    assert_eq!(session.doc().unwrap().history.undo.len(), before + 1);
    session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(path(&session, closed), original_closed);
    session.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(path(&session, closed), opened);
    (canonical_markers, canonical)
}
