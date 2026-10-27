use super::*;
use crate::testutil::*;
use vectorcraft_doc::Selection;

#[test]
fn press_inside_drag_threshold_places_a_corner() {
    let (d, s) = (vectorcraft_doc::Document::new(500.0, 500.0), Selection::default());
    let paint = paint();
    let cx = ToolContext { zoom: 1.0, smart_guides: false, snap_to_point: false, snap_to_pixel: false, snap_to_grid: false, ..cx(&d, &s, &paint) };
    let mut pen = PenTool::default();
    pen.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
    assert!(pen.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 101.0, 101.0)).is_empty());
    assert_eq!(pen.pointer(&cx, &PointerEvent::new(PointerKind::Up, 101.0, 101.0)), vec![Action::Commit]);
    pen.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.0, 200.0));
    assert_eq!(pen.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 204.0, 200.0)).len(), 1);
}

#[test]
fn snapped_last_endpoint_converts_active_path_after_raw_miss() {
    let first = Point::new(100.0, 300.0);
    let last = Point::new(200.0, 400.0);
    let (mut d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[first, Point::new(200.0, 300.0), last], false));
    d.grid.spacing = 100.0;
    d.grid.subdivisions = 1;
    let paint = paint();
    let cx = ToolContext {
        zoom: 1.0,
        selection_tolerance: 8.0,
        smart_guides: false,
        snap_to_point: false,
        snap_to_pixel: false,
        snap_to_grid: true,
        ..cx(&d, &s, &paint)
    };
    let sp = &d.node(id).unwrap().path_data().unwrap().subpaths[0];
    assert!(!sp.closed);
    assert_eq!(sp.anchors.iter().map(|a| a.p).collect::<Vec<_>>(), vec![first, Point::new(200.0, 300.0), last]);
    let mut pen = PenTool::default();
    assert!(click(&mut pen, &cx, last.x, last.y, Mods::default()).is_empty());
    assert_eq!(pen.active(&cx).map(|(id, first, last, _)| (id, first, last)), Some((id, first, last)));
    assert!(!pen.busy());

    let raw = Point::new(237.0, 403.0);
    let tol = cx.tol(5.0).max(cx.point_tol());
    assert!(raw.distance(first) > tol && raw.distance(last) > tol);
    let from = Leave::segment(&cx, last, false);
    assert_eq!(DrawSnap::default().press(&cx, raw, &[id], Some(&from)), last);
    assert!(open_end_at(&cx, raw, tol, Some(id)).is_none());
    assert!(open_end_at(&cx, last, tol, Some(id)).is_none());

    assert_eq!(
        pen.pointer(&cx, &PointerEvent::new(PointerKind::Down, raw.x, raw.y)),
        vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 2, last.x, last.y)]
    );
    assert_eq!(pen.handle, Some((id, 0, 2, last)));
    assert!(pen.drag.is_none() && pen.busy());
    assert_eq!(pen.pointer(&cx, &PointerEvent::new(PointerKind::Up, raw.x, raw.y)), vec![Action::Commit]);
    assert!(pen.handle.is_none() && !pen.busy());
    assert_eq!(pen.active(&cx).map(|(id, first, last, _)| (id, first, last)), Some((id, first, last)));
}

#[test]
fn raw_other_endpoint_wins_before_snapped_active_last_endpoint() {
    let first = Point::new(100.0, 300.0);
    let last = Point::new(200.0, 400.0);
    let raw = Point::new(237.0, 403.0);
    let (mut d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[first, Point::new(200.0, 300.0), last], false));
    let other = d.alloc_id();
    let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&[raw, Point::new(337.0, 403.0)], false));
    d.insert(Some(d.layers[0].id), 2, vectorcraft_doc::Node::path(other, path, vectorcraft_doc::Appearance::default_art())).unwrap();
    d.grid.spacing = 100.0;
    d.grid.subdivisions = 1;
    let paint = paint();
    let cx = ToolContext {
        zoom: 1.0,
        selection_tolerance: 8.0,
        smart_guides: false,
        snap_to_point: false,
        snap_to_pixel: false,
        snap_to_grid: true,
        ..cx(&d, &s, &paint)
    };
    let mut pen = PenTool::default();
    assert!(click(&mut pen, &cx, last.x, last.y, Mods::default()).is_empty());
    assert_eq!(pen.active(&cx).map(|(id, first, last, _)| (id, first, last)), Some((id, first, last)));
    assert!(!pen.busy());
    let tol = cx.tol(5.0).max(cx.point_tol());
    assert!(raw.distance(first) > tol && raw.distance(last) > tol);
    assert_eq!(open_end_at(&cx, raw, tol, Some(id)), Some((other, true)));
    let from = Leave::segment(&cx, last, false);
    assert_eq!(DrawSnap::default().press(&cx, raw, &[id], Some(&from)), last);

    assert_eq!(
        pen.pointer(&cx, &PointerEvent::new(PointerKind::Down, raw.x, raw.y)),
        vec![Action::Exec("path.join".into(), json!({"ids": [id.0, other.0], "ends": ["last", "first"]}))]
    );
    assert!(!pen.busy() && pen.active(&cx).is_none());
    assert!(pen.pointer(&cx, &PointerEvent::new(PointerKind::Up, raw.x, raw.y)).is_empty());
}

#[test]
fn snapped_first_endpoint_closes_active_path_after_raw_miss() {
    let first = Point::new(100.0, 300.0);
    let last = Point::new(200.0, 400.0);
    let (mut d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[first, Point::new(200.0, 300.0), last], false));
    d.grid.spacing = 100.0;
    d.grid.subdivisions = 1;
    let paint = paint();
    let cx = ToolContext {
        zoom: 1.0,
        selection_tolerance: 8.0,
        smart_guides: false,
        snap_to_point: false,
        snap_to_pixel: false,
        snap_to_grid: true,
        ..cx(&d, &s, &paint)
    };
    let sp = &d.node(id).unwrap().path_data().unwrap().subpaths[0];
    assert!(!sp.closed);
    assert_eq!(sp.anchors.iter().map(|a| a.p).collect::<Vec<_>>(), vec![first, Point::new(200.0, 300.0), last]);
    let mut pen = PenTool::default();
    assert!(click(&mut pen, &cx, last.x, last.y, Mods::default()).is_empty());
    assert_eq!(pen.active(&cx).map(|(id, first, last, _)| (id, first, last)), Some((id, first, last)));
    assert!(!pen.busy());

    let raw = Point::new(63.0, 303.0);
    let tol = cx.tol(5.0).max(cx.point_tol());
    assert!(raw.distance(first) > tol && raw.distance(last) > tol);
    let from = Leave::segment(&cx, last, false);
    assert_eq!(DrawSnap::default().press(&cx, raw, &[id], Some(&from)), first);
    assert!(open_end_at(&cx, raw, tol, Some(id)).is_none());
    assert!(open_end_at(&cx, first, tol, Some(id)).is_none());

    assert_eq!(
        pen.pointer(&cx, &PointerEvent::new(PointerKind::Down, raw.x, raw.y)),
        vec![Action::Begin("Close Path".into()), Action::Preview("path.close".into(), json!({"id": id.0}))]
    );
    let place = pen.drag.unwrap();
    assert!(place.closing);
    assert_eq!((place.path, place.at), (Some(id), first));
    assert_eq!(pen.pointer(&cx, &PointerEvent::new(PointerKind::Up, raw.x, raw.y)), vec![Action::Commit]);
    assert!(!pen.busy() && pen.active(&cx).is_none());
}

#[test]
fn raw_other_endpoint_wins_before_snapped_active_first_endpoint() {
    let first = Point::new(100.0, 300.0);
    let last = Point::new(200.0, 400.0);
    let raw = Point::new(63.0, 303.0);
    let (mut d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[first, Point::new(200.0, 300.0), last], false));
    let other = d.alloc_id();
    let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&[raw, Point::new(63.0, 403.0)], false));
    d.insert(Some(d.layers[0].id), 2, vectorcraft_doc::Node::path(other, path, vectorcraft_doc::Appearance::default_art())).unwrap();
    d.grid.spacing = 100.0;
    d.grid.subdivisions = 1;
    let paint = paint();
    let cx = ToolContext {
        zoom: 1.0,
        selection_tolerance: 8.0,
        smart_guides: false,
        snap_to_point: false,
        snap_to_pixel: false,
        snap_to_grid: true,
        ..cx(&d, &s, &paint)
    };
    let mut pen = PenTool::default();
    assert!(click(&mut pen, &cx, last.x, last.y, Mods::default()).is_empty());
    assert_eq!(pen.active(&cx).map(|(id, first, last, _)| (id, first, last)), Some((id, first, last)));
    assert!(!pen.busy());
    let tol = cx.tol(5.0).max(cx.point_tol());
    assert!(raw.distance(first) > tol && raw.distance(last) > tol);
    assert_eq!(open_end_at(&cx, raw, tol, Some(id)), Some((other, true)));
    let from = Leave::segment(&cx, last, false);
    assert_eq!(DrawSnap::default().press(&cx, raw, &[id], Some(&from)), first);

    assert_eq!(
        pen.pointer(&cx, &PointerEvent::new(PointerKind::Down, raw.x, raw.y)),
        vec![Action::Exec("path.join".into(), json!({"ids": [id.0, other.0], "ends": ["last", "first"]}))]
    );
    assert!(!pen.busy() && pen.active(&cx).is_none());
    assert!(pen.pointer(&cx, &PointerEvent::new(PointerKind::Up, raw.x, raw.y)).is_empty());
}

#[test]
fn first_click_creates_path() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = PenTool::default();
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
    assert_eq!(a[0], Action::Begin("Pen".into()));
    assert!(matches!(&a[1], Action::Preview(c, _) if c == "path.create"));
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 10.0, 10.0)), vec![Action::Commit]);
}

#[test]
fn alt_pressed_mid_drag_keeps_the_incoming_handle() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let line = vectorcraft_geom::shapes::line(Point::new(10.0, 300.0), Point::new(60.0, 300.0));
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = PenTool { drawing: true, ..PenTool::default() };
    let alt = Mods { alt: true, ..Mods::default() };
    let handles = |acts: Vec<Action>| match acts.as_slice() {
        [Action::Preview(c, v)] if c == "path.appendAnchor" => (v["in"].clone(), v["out"].clone()),
        other => panic!("not an anchor preview: {other:?}"),
    };
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 400.0, 420.0));
    let drag = PointerEvent::new(PointerKind::Drag, 450.0, 420.0);
    assert_eq!(handles(t.pointer(&cx, &drag)), (json!([350.0, 420.0]), json!([450.0, 420.0])));
    // Alt goes down: the curve shaped so far stays, and only the outgoing handle moves on.
    let drag = PointerEvent::new(PointerKind::Drag, 450.0, 470.0).with_mods(alt);
    assert_eq!(handles(t.pointer(&cx, &drag)), (json!([350.0, 420.0]), json!([450.0, 470.0])));
    let drag = PointerEvent::new(PointerKind::Drag, 400.0, 480.0).with_mods(alt);
    assert_eq!(handles(t.pointer(&cx, &drag)), (json!([350.0, 420.0]), json!([400.0, 480.0])));
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 400.0, 480.0).with_mods(alt)), vec![Action::Commit]);
    // Alt held from the start: the new anchor gets an outgoing handle only.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 440.0).with_mods(alt));
    let drag = PointerEvent::new(PointerKind::Drag, 340.0, 440.0).with_mods(alt);
    assert_eq!(handles(t.pointer(&cx, &drag)), (json!([300.0, 440.0]), json!([340.0, 440.0])));
}

#[test]
fn space_pressed_inside_the_drag_threshold_moves_the_anchor_by_the_next_move_only() {
    let (d, s) = (vectorcraft_doc::Document::new(500.0, 500.0), Selection::default());
    let paint = paint();
    let cx = ToolContext { zoom: 1.0, smart_guides: false, snap_to_point: false, snap_to_pixel: false, snap_to_grid: false, ..cx(&d, &s, &paint) };
    let space = Mods { space: true, ..Mods::default() };
    let mut pen = PenTool::default();
    pen.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
    // Still inside the threshold, Space held: nothing happens yet.
    assert!(pen.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 102.0, 100.0).with_mods(space)).is_empty());
    // The move that crosses it moves the anchor by that move (2 px to 4 px), not by the 4 px since the press.
    let acts = pen.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 104.0, 100.0).with_mods(space));
    let Some(Action::Preview(c, v)) = acts.first() else { panic!("not a preview: {acts:?}") };
    assert_eq!(c, "path.create");
    assert_eq!(v["anchors"][0]["x"], json!(102.0), "moved by the delta");
    assert_eq!(v["anchors"][0]["y"], json!(100.0));
}

#[test]
fn space_moves_the_anchor_being_placed() {
    let (mut d, _) = doc_with_rect();
    let space = Mods { space: true, ..Mods::default() };
    // The first anchor of a new path.
    let (s, p) = (Selection::default(), paint());
    let cx0 = cx(&d, &s, &p);
    let mut t = PenTool::default();
    t.pointer(&cx0, &PointerEvent::new(PointerKind::Down, 400.0, 420.0));
    t.pointer(&cx0, &PointerEvent::new(PointerKind::Drag, 450.0, 420.0));
    let created = |acts: Vec<Action>| match acts.as_slice() {
        [Action::Preview(c, v)] if c == "path.create" => v["anchors"][0].clone(),
        other => panic!("not a create preview: {other:?}"),
    };
    let a = created(t.pointer(&cx0, &PointerEvent::new(PointerKind::Drag, 450.0, 380.0).with_mods(space)));
    assert_eq!(a, json!({"x": 400.0, "y": 380.0, "out": [450.0, 380.0], "in": [350.0, 380.0]}), "moved, handles and all");
    // Space released: the drag shapes the handles round the anchor's new place.
    let a = created(t.pointer(&cx0, &PointerEvent::new(PointerKind::Drag, 460.0, 380.0)));
    assert_eq!(a, json!({"x": 400.0, "y": 380.0, "out": [460.0, 380.0], "in": [340.0, 380.0]}));
    // An anchor added to an open path, its handles broken by Alt: the kept incoming handle
    // moves with it.
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let line = vectorcraft_geom::shapes::line(Point::new(10.0, 300.0), Point::new(60.0, 300.0));
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.set([id]);
    let cx = cx(&d, &s, &p);
    let mut t = PenTool { drawing: true, ..PenTool::default() };
    let alt = Mods { alt: true, ..Mods::default() };
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 400.0, 420.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 450.0, 420.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 450.0, 470.0).with_mods(alt));
    let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 450.0, 450.0).with_mods(Mods { space: true, ..alt }));
    assert_eq!(
        acts,
        vec![Action::Preview("path.appendAnchor".into(), json!({"id": id.0, "x": 400.0, "y": 400.0, "in": [350.0, 400.0], "out": [450.0, 450.0]}))]
    );
}

/// A document with an open path of `anchors`, selected, being drawn by a Pen.
fn drawing(anchors: vectorcraft_geom::SubPath) -> (vectorcraft_doc::Document, NodeId, Selection) {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let path = vectorcraft_geom::PathData::single(anchors);
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.set([id]);
    (d, id, s)
}

fn set_out(id: NodeId, ai: usize, x: f64, y: f64) -> Action {
    Action::Preview("path.setHandle".into(), json!({"id": id.0, "subpath": 0, "anchor": ai, "which": "out", "x": x, "y": y, "independent": true}))
}

#[test]
fn clicking_the_last_anchor_retracts_its_handle_and_dragging_pulls_a_new_one() {
    // The last anchor is smooth: handles at 40 and 80 either side of (60, 300).
    let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(10.0, 300.0), Point::new(60.0, 300.0)], false);
    sp.anchors[1].h_in = Point::new(40.0, 300.0);
    sp.anchors[1].h_out = Point::new(80.0, 300.0);
    let (d, id, s) = drawing(sp);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = PenTool { drawing: true, ..PenTool::default() };
    assert_eq!(t.cursor(&cx, Point::new(61.0, 301.0), Mods::default()), Cursor::PenConvert);
    // A click retracts the outgoing handle and adds no anchor.
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 61.0, 301.0));
    assert_eq!(a, vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 1, 60.0, 300.0)]);
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 61.0, 301.0)), vec![Action::Commit]);
    // A drag pulls a new outgoing handle out.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 60.0, 300.0));
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 70.0, 280.0)), vec![set_out(id, 1, 70.0, 280.0)]);
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 70.0, 280.0)), vec![Action::Commit]);
    // Elsewhere a click still adds an anchor, and the first anchor still closes the path.
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.0, 400.0));
    assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, _)] if c == "path.appendAnchor"), "{a:?}");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 200.0, 400.0));
    assert_eq!(t.cursor(&cx, Point::new(10.0, 300.0), Mods::default()), Cursor::PenClose);
}

/// #544: picking the terminal anchor uses the pointer, even when new points snap elsewhere.
#[test]
fn terminal_anchor_conversion_precedes_grid_snapping() {
    let (mut d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(13.0, 303.0), Point::new(63.0, 303.0)], false));
    d.grid.spacing = 100.0;
    d.grid.subdivisions = 1;
    let p = paint();
    let cx = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
    let mut t = PenTool { drawing: true, ..PenTool::default() };
    assert_eq!(t.cursor(&cx, Point::new(63.0, 303.0), Mods::default()), Cursor::PenConvert);
    assert_eq!(
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 63.0, 303.0)),
        vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 1, 63.0, 303.0)]
    );
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 63.0, 303.0)), vec![Action::Commit]);
    // The same raw picking applies when resuming the path and when closing it.
    let mut t = PenTool::default();
    assert_eq!(t.cursor(&cx, Point::new(63.0, 303.0), Mods::default()), Cursor::PenContinue);
    assert!(click(&mut t, &cx, 63.0, 303.0, Mods::default()).is_empty());
    assert_eq!(t.active(&cx).map(|a| a.0), Some(id));
    assert_eq!(
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 13.0, 303.0)),
        vec![Action::Begin("Close Path".into()), Action::Preview("path.close".into(), json!({"id": id.0}))]
    );
}

#[test]
fn smart_guides_cannot_steal_a_terminal_anchor_press() {
    let (mut d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(10.0, 300.0), Point::new(60.0, 300.0)], false));
    let other = d.alloc_id();
    let rect = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(66.0, 300.0, 76.0, 310.0));
    d.insert(Some(d.layers[0].id), 2, vectorcraft_doc::Node::path(other, rect, vectorcraft_doc::Appearance::default_art())).unwrap();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let pos = Point::new(63.0, 300.0);
    assert_eq!(crate::guides::snap_draw(&cx, pos, &[id]).0, Point::new(66.0, 300.0));
    let mut t = PenTool { drawing: true, ..PenTool::default() };
    assert_eq!(t.cursor(&cx, pos, Mods::default()), Cursor::PenConvert);
    assert_eq!(
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, pos.x, pos.y)),
        vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 1, 60.0, 300.0)]
    );
}

/// Nearby endpoints share a pick radius: use the closest one for cursor and press.
#[test]
fn overlapping_endpoint_tolerances_pick_the_nearest_end() {
    let paint = paint();
    let mut failures = vec![];
    for zoom in [0.5, 1.0, 2.0] {
        let first = Point::new(100.0, 300.0);
        let last = Point::new(100.0 + 6.0 / zoom, 300.0);
        let (d, id, selection) = drawing(vectorcraft_geom::SubPath::polyline(&[first, last], false));
        for grid in [false, true] {
            let cx = ToolContext {
                zoom,
                selection_tolerance: 8.0,
                smart_guides: false,
                snap_to_point: false,
                snap_to_grid: grid,
                ..cx(&d, &selection, &paint)
            };
            for shift in [false, true] {
                let mods = Mods { shift, ..Mods::default() };
                // Include off-anchor clicks in both halves of the overlap and the midpoint.
                for offset in [6.0, 0.0, 1.0, 3.0, 5.0] {
                    let at = Point::new(first.x + offset / zoom, first.y);
                    let first_end = offset <= 3.0;
                    let mut pen = PenTool { drawing: true, ..PenTool::default() };
                    let cursor = if first_end { Cursor::PenClose } else { Cursor::PenConvert };
                    let actual_cursor = pen.cursor(&cx, at, mods);
                    let expected = if first_end {
                        vec![Action::Begin("Close Path".into()), Action::Preview("path.close".into(), json!({"id": id.0}))]
                    } else {
                        vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 1, last.x, last.y)]
                    };
                    let actual = pen.pointer(&cx, &PointerEvent::new(PointerKind::Down, at.x, at.y).with_mods(mods));
                    if actual_cursor != cursor || actual != expected {
                        failures.push((zoom, grid, shift, offset, actual_cursor, actual));
                    }
                    assert_eq!(pen.pointer(&cx, &PointerEvent::new(PointerKind::Up, at.x, at.y)), vec![Action::Commit]);
                }
            }
        }
    }
    assert!(failures.is_empty(), "cursor and press must pick the closest eligible endpoint: {failures:?}");
}

#[test]
fn overlapping_endpoint_tolerances_continue_from_the_nearest_end() {
    let paint = paint();
    let mut failures = vec![];
    for zoom in [0.5, 1.0, 2.0] {
        let first = Point::new(100.0, 300.0);
        let last = Point::new(100.0 + 6.0 / zoom, 300.0);
        let (d, id, selection) = drawing(vectorcraft_geom::SubPath::polyline(&[first, last], false));
        for grid in [false, true] {
            let cx = ToolContext {
                zoom,
                selection_tolerance: 8.0,
                smart_guides: false,
                snap_to_point: false,
                snap_to_grid: grid,
                ..cx(&d, &selection, &paint)
            };
            for offset in [6.0, 0.0, 1.0, 3.0, 5.0] {
                let at = Point::new(first.x + offset / zoom, first.y);
                let mut pen = PenTool::default();
                assert_eq!(pen.cursor(&cx, at, Mods::default()), Cursor::PenContinue);
                let expected = if offset < 3.0 { vec![Action::Exec("path.reverse".into(), json!({}))] } else { vec![] };
                let actual = click(&mut pen, &cx, at.x, at.y, Mods::default());
                if actual != expected {
                    failures.push((zoom, grid, offset, actual));
                }
                assert_eq!(pen.active(&cx).map(|a| a.0), Some(id));
            }
        }
    }
    assert!(failures.is_empty(), "resuming must reverse only for the closest first endpoint: {failures:?}");
}

#[test]
fn coincident_endpoints_keep_the_prior_tie_choice() {
    let at = Point::new(100.0, 300.0);
    let (d, id, selection) = drawing(vectorcraft_geom::SubPath::polyline(&[at, at], false));
    let paint = paint();
    for zoom in [0.5, 1.0, 2.0] {
        let cx = ToolContext { zoom, smart_guides: false, snap_to_point: false, ..cx(&d, &selection, &paint) };
        let mut drawing = PenTool { drawing: true, ..PenTool::default() };
        assert_eq!(drawing.cursor(&cx, at, Mods::default()), Cursor::PenClose);
        assert_eq!(
            click(&mut drawing, &cx, at.x, at.y, Mods::default()),
            vec![Action::Begin("Close Path".into()), Action::Preview("path.close".into(), json!({"id": id.0}))]
        );
        let mut resuming = PenTool::default();
        assert_eq!(resuming.cursor(&cx, at, Mods::default()), Cursor::PenContinue);
        assert!(click(&mut resuming, &cx, at.x, at.y, Mods::default()).is_empty());
    }
}

#[test]
fn terminal_anchor_uses_the_selection_tolerance_at_each_zoom() {
    let (d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(10.0, 300.0), Point::new(60.0, 300.0)], false));
    let p = paint();
    for zoom in [0.5, 2.0] {
        let cx = ToolContext { zoom, selection_tolerance: 8.0, smart_guides: false, snap_to_point: false, ..cx(&d, &s, &p) };
        let pos = Point::new(60.0, 300.0 + 6.0 / zoom);
        let mut t = PenTool { drawing: true, ..PenTool::default() };
        assert_eq!(t.cursor(&cx, pos, Mods::default()), Cursor::PenConvert);
        assert_eq!(
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, pos.x, pos.y)),
            vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 1, 60.0, 300.0)]
        );
        let narrow = ToolContext { selection_tolerance: 1.0, ..cx };
        // The upstream point picker retains a minimum screen radius at low tolerance.
        let near = Point::new(60.0, 300.0 + 4.0 / zoom);
        let mut narrow_pen = PenTool { drawing: true, ..PenTool::default() };
        assert_eq!(narrow_pen.cursor(&narrow, near, Mods::default()), Cursor::PenConvert);
        assert_eq!(
            click(&mut narrow_pen, &narrow, near.x, near.y, Mods::default()),
            vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 1, 60.0, 300.0)]
        );
        assert_eq!(narrow_pen.cursor(&narrow, Point::new(60.0, 300.0 + 6.0 / zoom), Mods::default()), Cursor::PenDrawing);
    }
}

/// Upstream's point radius grows with the drawn anchor; cursor and press keep agreeing.
#[test]
fn enlarged_anchor_radius_is_used_for_endpoints_and_alt_conversion() {
    let (d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(10.0, 300.0), Point::new(60.0, 300.0)], false));
    let p = paint();
    for zoom in [0.5, 1.0, 2.0] {
        let cx = ToolContext { zoom, selection_tolerance: 1.0, anchor_size: 7, smart_guides: false, snap_to_point: false, ..cx(&d, &s, &p) };
        let at = Point::new(60.0 + 6.0 / zoom, 300.0);
        let mut terminal = PenTool { drawing: true, ..PenTool::default() };
        assert_eq!(terminal.cursor(&cx, at, Mods::default()), Cursor::PenConvert);
        assert_eq!(
            click(&mut terminal, &cx, at.x, at.y, Mods::default()),
            vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 1, 60.0, 300.0)]
        );
        let mut continuing = PenTool::default();
        assert_eq!(continuing.cursor(&cx, at, Mods::default()), Cursor::PenContinue);
        assert!(click(&mut continuing, &cx, at.x, at.y, Mods::default()).is_empty());
        assert_eq!(continuing.active(&cx).map(|a| a.0), Some(id));
        let mut closing = PenTool { drawing: true, ..PenTool::default() };
        let first = Point::new(10.0 - 6.0 / zoom, 300.0);
        assert_eq!(closing.cursor(&cx, first, Mods::default()), Cursor::PenClose);
        assert_eq!(
            click(&mut closing, &cx, first.x, first.y, Mods::default()),
            vec![Action::Begin("Close Path".into()), Action::Preview("path.close".into(), json!({"id": id.0}))]
        );
        let mut alt_pen = PenTool { drawing: true, ..PenTool::default() };
        let alt = Mods { alt: true, ..Mods::default() };
        assert!(alt_pen.pointer(&cx, &PointerEvent::new(PointerKind::Down, at.x, at.y).with_mods(alt)).is_empty());
        assert!(alt_pen.busy(), "Alt must reach the enlarged terminal-anchor target");
        assert!(alt_pen.convert.is_some());
        let away_from_segment = Point::new(35.0, 300.0 + 4.0 / zoom);
        assert!(!alt_converts(&cx, away_from_segment, alt), "segment picking retains the smaller selection radius");
    }
}

/// #516: Alt on the terminal anchor borrows Anchor Point, just like the other anchors.
#[test]
fn alt_on_the_terminal_anchor_pulls_symmetric_handles() {
    let (d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0)], false));
    let p = paint();
    let cx = cx(&d, &s, &p);
    let alt = Mods { alt: true, ..Mods::default() };
    let mut t = PenTool { drawing: true, ..PenTool::default() };
    assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.0, 300.0).with_mods(alt)).is_empty());
    assert_eq!(
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 230.0, 330.0).with_mods(alt)),
        vec![
            Action::Begin("Convert Anchor Point".into()),
            Action::Preview("path.convertAnchor".into(), json!({"id": id.0, "subpath": 0, "anchor": 1, "to": "smooth", "x": 230.0, "y": 330.0}))
        ]
    );
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 230.0, 330.0)), vec![Action::Commit]);
    assert!(
        matches!(click(&mut t, &cx, 300.0, 400.0, Mods::default()).as_slice(), [Action::Begin(_), Action::Preview(c, _)] if c == "path.appendAnchor")
    );
}

#[test]
fn ending_during_a_pen_gesture_commits_and_clears_it() {
    let (d, _, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0)], false));
    let p = paint();
    let cx = cx(&d, &s, &p);
    for terminal in [false, true] {
        for key in [Some(ToolKey::Escape), Some(ToolKey::Enter), None] {
            let mut t = PenTool { drawing: true, ..PenTool::default() };
            let x = if terminal { 200.0 } else { 300.0 };
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, x, 300.0));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x + 20.0, 330.0));
            let acts = match key {
                Some(k) => t.key(&cx, k, Mods::default()),
                None => t.deactivate(&cx),
            };
            assert_eq!(acts, vec![Action::Commit]);
            assert!(!t.busy());
            assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, x + 20.0, 330.0)).is_empty());
            assert_eq!(t.cursor(&cx, Point::new(400.0, 400.0), Mods::default()), Cursor::Pen);
        }
    }
}

/// #501: a press on the first anchor closes the path, however far the drag then goes: the
/// closing preview leaves the path closed (so not one the pen extends), and each move still
/// previews the close with symmetric handles (Alt breaks them), released as a close.
#[test]
fn a_drag_from_the_first_anchor_always_closes() {
    let pts = [Point::new(100.0, 300.0), Point::new(200.0, 300.0), Point::new(200.0, 400.0)];
    let (mut d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&pts, false));
    let p = paint();
    let mut t = PenTool { drawing: true, ..PenTool::default() };
    let a = t.pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, 101.0, 301.0));
    assert_eq!(a, vec![Action::Begin("Close Path".into()), Action::Preview("path.close".into(), json!({"id": id.0}))]);
    // What the tool sees once the preview applied: the path closed.
    d.node_mut(id).unwrap().path_data_mut().unwrap().subpaths[0].closed = true;
    let c = cx(&d, &s, &p);
    let close =
        |x: f64, y: f64, alt: bool| vec![Action::Preview("path.close".into(), json!({"id": id.0, "in": [200.0 - x, 600.0 - y], "independent": alt}))];
    for (x, y) in [(105.0, 290.0), (130.0, 260.0), (160.0, 240.0)] {
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Drag, x, y)), close(x, y, false), "at ({x}, {y})");
    }
    let alt = Mods { alt: true, ..Mods::default() };
    assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 170.0, 250.0).with_mods(alt)), close(170.0, 250.0, true));
    assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 170.0, 250.0)), vec![Action::Commit]);
    assert!(!t.busy() && t.active(&c).is_none(), "the path is done");
}

#[test]
fn a_one_anchor_path_does_not_close_on_itself() {
    let (d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(10.0, 300.0)], false));
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = PenTool { drawing: true, ..PenTool::default() };
    assert_eq!(t.cursor(&cx, Point::new(10.0, 300.0), Mods::default()), Cursor::PenConvert);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 300.0));
    assert_eq!(a, vec![Action::Begin("Convert Anchor Point".into()), set_out(id, 0, 10.0, 300.0)]);
}

#[test]
fn clicking_a_selected_blend_spine_adds_a_point() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let key = |id: NodeId, x: f64| {
        let r = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(x, 300.0, x + 20.0, 320.0));
        std::sync::Arc::new(vectorcraft_doc::Node::path(id, r, vectorcraft_doc::Appearance::default_art()))
    };
    let (g, k1, k2) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    d.insert(Some(l), 1, vectorcraft_doc::Node::new(g, NodeKind::Blend { children: vec![key(k1, 100.0), key(k2, 200.0)], spec: Default::default() }))
        .unwrap();
    let mut s = Selection::default();
    s.set([g]);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = PenTool::default();
    assert_eq!(t.cursor(&cx, Point::new(160.0, 311.0), Mods::default()), Cursor::PenAdd);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 160.0, 311.0));
    assert_eq!(a, vec![Action::Exec("object.blend.spine.addAnchor".into(), json!({"id": g.0, "x": 160.0, "y": 310.0}))]);
    // A key's own point is left alone (the click snaps to the spine line).
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 110.0, 310.0)), vec![]);
}

fn click(t: &mut PenTool, cx: &ToolContext, x: f64, y: f64, m: Mods) -> Vec<Action> {
    let acts = t.pointer(cx, &PointerEvent::new(PointerKind::Down, x, y).with_mods(m));
    t.pointer(cx, &PointerEvent::new(PointerKind::Up, x, y).with_mods(m));
    acts
}

#[test]
fn clicking_a_selected_path_adds_or_deletes_an_anchor() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let pts = [Point::new(300.0, 400.0), Point::new(350.0, 300.0), Point::new(400.0, 400.0)];
    let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&pts, false));
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let off = ToolContext { auto_add_delete: false, ..cx(&d, &s, &p) };
    let cx = cx(&d, &s, &p);
    let none = Mods::default();
    let mut t = PenTool::default();
    // On a segment: add an anchor there.
    assert_eq!(t.cursor(&cx, Point::new(325.0, 351.0), none), Cursor::PenAdd);
    let a = click(&mut t, &cx, 325.0, 351.0, none);
    assert!(matches!(&a[..], [Action::Exec(c, v)] if c == "path.insertAnchor" && v["id"] == id.0 && v["segment"] == 0), "{a:?}");
    // On an anchor: delete it.
    assert_eq!(t.cursor(&cx, Point::new(350.0, 302.0), none), Cursor::PenDelete);
    let a = click(&mut t, &cx, 350.0, 302.0, none);
    assert_eq!(a, vec![Action::Exec("path.removeAnchor".into(), json!({"id": id.0, "subpath": 0, "anchor": 1}))]);
    // An end still continues the path.
    assert_eq!(t.cursor(&cx, Point::new(400.0, 400.0), none), Cursor::PenContinue);
    // Shift, or the preference turned off, starts a new path instead.
    let shift = Mods { shift: true, ..Mods::default() };
    assert_eq!(t.cursor(&cx, Point::new(325.0, 351.0), shift), Cursor::Pen);
    assert_eq!(t.cursor(&off, Point::new(350.0, 302.0), none), Cursor::Pen);
    let a = click(&mut t, &off, 350.0, 302.0, none);
    assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, _)] if c == "path.create"), "{a:?}");
    // While a path is being drawn, a click goes on drawing it.
    let a = click(&mut t, &cx, 325.0, 351.0, none);
    assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, _)] if c == "path.appendAnchor"), "{a:?}");
    let mut t = PenTool::default();
    let a = click(&mut t, &cx, 325.0, 351.0, shift);
    assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, _)] if c == "path.create"), "{a:?}");
}

/// Each anchor snaps to Smart Guides (#506): hovering shows where it goes (the rubber band ends
/// there), in line with another object's centre; Shift slides it along its 45° step into line.
#[test]
fn anchors_snap_to_smart_guides() {
    let (d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(10.0, 300.0), Point::new(60.0, 300.0)], false));
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = PenTool { drawing: true, ..PenTool::default() };
    t.pointer(&cx, &PointerEvent::new(PointerKind::Move, 152.0, 318.0));
    let o = t.overlays(&cx);
    assert!(o.iter().any(|o| matches!(o, Overlay::Label { text, p, .. } if text == "align" && *p == Point::new(150.0, 318.0))), "{o:?}");
    assert!(
        o.iter()
            .any(|o| matches!(o, Overlay::Path { path, .. } if path.elements().last().and_then(|e| e.end_point()) == Some(Point::new(150.0, 318.0))))
    );
    let from = Point::new(60.0, 300.0);
    let to = Point::new(150.0, 318.0);
    assert!(o.iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &cx.segment_measure_label(from, to))));
    let a = click(&mut t, &cx, 152.0, 318.0, Mods::default());
    assert_eq!(a[1], Action::Preview("path.appendAnchor".into(), json!({"id": id.0, "x": 150.0, "y": 318.0})));
    let shift = Mods { shift: true, ..Mods::default() };
    let a = click(&mut t, &cx, 152.0, 310.0, shift);
    assert_eq!(a[1], Action::Preview("path.appendAnchor".into(), json!({"id": id.0, "x": 150.0, "y": 300.0})));
    // The first anchor of a new path lands on the rect's corner.
    let mut t = PenTool::default();
    let a = click(&mut t, &cx, 102.0, 99.0, Mods::default());
    assert_eq!(a[1], Action::Preview("path.create".into(), json!({"anchors": [{"x": 100.0, "y": 100.0}]})));
}

#[test]
fn alt_over_a_shown_handle_or_an_anchor_converts() {
    // The middle anchor is smooth: handles at (150, 100) and (250, 100) round (200, 100).
    let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 200.0), Point::new(200.0, 100.0), Point::new(300.0, 200.0)], false);
    sp.anchors[1] = vectorcraft_geom::Anchor::smooth(Point::new(200.0, 100.0), Point::new(250.0, 100.0));
    let (d, id, mut s) = drawing(sp);
    let p = paint();
    let alt = Mods { alt: true, ..Mods::default() };
    let t = PenTool::default();
    let cx0 = cx(&d, &s, &p);
    // Alt lends the Anchor Point tool, and its caret with it.
    assert_eq!(t.cursor(&cx0, Point::new(250.0, 101.0), alt), Cursor::AnchorPoint);
    assert_eq!(t.cursor(&cx0, Point::new(200.0, 101.0), alt), Cursor::AnchorPoint);
    assert_eq!(t.cursor(&cx0, Point::new(250.0, 101.0), Mods::default()), Cursor::Pen);
    assert_eq!(t.cursor(&cx0, Point::new(150.0, 300.0), alt), Cursor::Pen);
    // A drag of the handle moves it alone, as one gesture.
    let mut t = PenTool::default();
    assert_eq!(t.pointer(&cx0, &PointerEvent::new(PointerKind::Down, 250.0, 101.0).with_mods(alt)), vec![]);
    assert!(t.busy());
    let a = t.pointer(&cx0, &PointerEvent::new(PointerKind::Drag, 260.0, 140.0).with_mods(alt));
    assert_eq!(
        a,
        vec![
            Action::Begin("Reshape".into()),
            Action::Preview(
                "path.setHandle".into(),
                json!({"id": id.0, "subpath": 0, "anchor": 1, "which": "out", "x": 260.0, "y": 140.0, "independent": true})
            )
        ]
    );
    // Alt let go mid-drag: the gesture goes on to its end.
    assert_eq!(t.pointer(&cx0, &PointerEvent::new(PointerKind::Up, 260.0, 140.0)), vec![Action::Commit]);
    assert!(!t.busy());
    // With other anchors direct-selected the handles of this one are hidden, and not grabbed.
    s.anchors.insert(id, std::collections::BTreeSet::from([(0, 0)]));
    let cx1 = cx(&d, &s, &p);
    assert_eq!(t.cursor(&cx1, Point::new(250.0, 101.0), alt), Cursor::Pen);
}

/// #494: Alt held over a segment of a selected path reshapes it as the Anchor Point tool does,
/// while drawing too; Alt away from paths still places an anchor.
#[test]
fn alt_over_a_segment_reshapes_it() {
    let (d, id, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0)], false));
    let p = paint();
    let cx = cx(&d, &s, &p);
    let alt = Mods { alt: true, ..Mods::default() };
    let mut t = PenTool { drawing: true, ..PenTool::default() };
    assert_eq!(t.cursor(&cx, Point::new(150.0, 302.0), alt), Cursor::ReshapeSegment);
    assert_eq!(t.cursor(&cx, Point::new(150.0, 302.0), Mods::default()), Cursor::PenDrawing);
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 302.0).with_mods(alt)), vec![]);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 262.0).with_mods(alt));
    assert_eq!(t.cursor(&cx, Point::new(150.0, 262.0), alt), Cursor::ReshapeSegment, "kept during the reshape");
    assert_eq!(a[0], Action::Begin("Reshape".into()));
    assert!(
        matches!(&a[1], Action::Preview(c, v) if c == "path.reshapeSegment" && v["id"] == id.0 && v["segment"] == 0 && v["dy"] == -40.0 && v["perpendicular"] == false),
        "{a:?}"
    );
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 262.0)), vec![Action::Commit]);
    // Shift held too keeps the handles at right angles to the line between the anchors.
    let alt_shift = Mods { shift: true, ..alt };
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 302.0).with_mods(alt));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 160.0, 262.0).with_mods(alt_shift));
    assert!(matches!(&a[1], Action::Preview(c, v) if c == "path.reshapeSegment" && v["perpendicular"] == true), "{a:?}");
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 160.0, 262.0)), vec![Action::Commit]);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 250.0, 400.0).with_mods(alt));
    assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, _)] if c == "path.appendAnchor"), "{a:?}");
}

/// #494: the Pen goes on drawing the path it drew while that path stays selected (after a Cmd
/// drag with Direct Selection), but never another open path selected meanwhile.
#[test]
fn the_pen_draws_on_the_path_it_began_only() {
    let (mut d, a, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(10.0, 300.0), Point::new(60.0, 300.0)], false));
    let l = d.layers[0].id;
    let b = d.alloc_id();
    let line = vectorcraft_geom::shapes::line(Point::new(10.0, 400.0), Point::new(60.0, 400.0));
    d.insert(Some(l), 2, vectorcraft_doc::Node::path(b, line, vectorcraft_doc::Appearance::default_art())).unwrap();
    let p = paint();
    let mut t = PenTool { drawing: true, ..PenTool::default() };
    let cx_a = cx(&d, &s, &p);
    let appended = |acts: &[Action]| match acts {
        [Action::Begin(_), Action::Preview(c, v)] if c == "path.appendAnchor" => v["id"].as_u64(),
        _ => None,
    };
    assert_eq!(appended(&click(&mut t, &cx_a, 100.0, 320.0, Mods::default())), Some(a.0));
    // Path B selected instead: the next click starts a new path.
    let mut sb = Selection::default();
    sb.set([b]);
    let cx_b = cx(&d, &sb, &p);
    assert_eq!(t.cursor(&cx_b, Point::new(150.0, 350.0), Mods::default()), Cursor::Pen);
    // An end of B shows what a click there does: it goes on drawing B.
    assert_eq!(t.cursor(&cx_b, Point::new(60.0, 400.0), Mods::default()), Cursor::PenContinue);
    let acts = click(&mut t, &cx_b, 150.0, 350.0, Mods::default());
    assert!(matches!(&acts[..], [Action::Begin(_), Action::Preview(c, _)] if c == "path.create"), "{acts:?}");
}

/// The new-path asterisk shows until a path is started; while one is being drawn the pen has no
/// badge, and the asterisk comes back once the path is finished.
#[test]
fn the_new_path_asterisk_shows_only_before_a_path_is_started() {
    let (d, _, s) = drawing(vectorcraft_geom::SubPath::polyline(&[Point::new(10.0, 300.0)], false));
    let p = paint();
    let cx = cx(&d, &s, &p);
    let empty = Point::new(250.0, 450.0);
    let mut t = PenTool::default();
    assert_eq!(t.cursor(&cx, empty, Mods::default()), Cursor::Pen, "no path started");
    t.drawing = true;
    assert_eq!(t.cursor(&cx, empty, Mods::default()), Cursor::PenDrawing, "after the first anchor");
    let a = click(&mut t, &cx, empty.x, empty.y, Mods::default());
    assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, _)] if c == "path.appendAnchor"), "{a:?}");
    assert_eq!(t.cursor(&cx, empty, Mods::default()), Cursor::PenDrawing);
    assert_eq!(t.cursor(&cx, empty, Mods { shift: true, ..Mods::default() }), Cursor::PenDrawing, "Shift constrains the next anchor");
    t.key(&cx, ToolKey::Enter, Mods::default());
    assert_eq!(t.cursor(&cx, empty, Mods::default()), Cursor::Pen, "the path is finished");
    assert_eq!(Cursor::PenDrawing.precise(), Cursor::Crosshair);
}
