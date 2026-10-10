//! Original tiny-geometry controls for Pen endpoint/Alt integration with continue-any-path.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use vectorcraft_doc::{Node, NodeId, NodeKind};
use vectorcraft_engine::{Session, ViewInfo};
use vectorcraft_geom::{AnchorKind, Point};
use vectorcraft_tools::{Cursor, Mods, PointerEvent, PointerKind};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s.prefs.selection_tolerance = 8.0;
    s.prefs.use_precise_cursors = false;
    s
}

fn view() -> ViewInfo {
    ViewInfo { zoom: 1.0, smart_guides: false, snap_to_grid: false, snap_to_pixel: false, snap_to_point: false, ..Default::default() }
}

fn node(s: &Session, id: NodeId) -> &Node {
    s.doc().unwrap().doc.node(id).unwrap()
}

fn path_ids(s: &Session) -> Vec<NodeId> {
    let mut ids = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if matches!(n.kind, NodeKind::Path { .. }) {
            ids.push(n.id);
        }
    });
    ids.sort_unstable();
    ids
}

fn points(s: &Session, id: NodeId) -> Vec<(f64, f64)> {
    let path = node(s, id).path_data().unwrap();
    assert_eq!(path.subpaths.len(), 1);
    assert!(!path.subpaths[0].closed);
    path.subpaths[0].anchors.iter().map(|a| (a.p.x, a.p.y)).collect()
}

fn line(s: &mut Session, from: (f64, f64), to: (f64, f64)) -> NodeId {
    let result = s.execute("shape.line", &json!({"x1": from.0, "y1": from.1, "x2": to.0, "y2": to.1})).unwrap();
    NodeId(result["id"].as_u64().unwrap())
}

fn gesture(s: &mut Session, view: ViewInfo, samples: &[(f64, f64)], mods: Mods) {
    let first = samples.first().unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, first.0, first.1).with_mods(mods), view).unwrap();
    for &(x, y) in samples.iter().skip(1) {
        s.pointer(&PointerEvent::new(PointerKind::Drag, x, y).with_mods(mods), view).unwrap();
    }
    let last = samples.last().unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, last.0, last.1).with_mods(mods), view).unwrap();
}

// B is above A. Its first endpoint coincides with A's interior corner, not either end of A.
fn overlapping_endpoint() -> (Session, NodeId, NodeId) {
    let mut s = session();
    let result = s.execute("path.create", &json!({"anchors": [{"x": 100, "y": 100}, {"x": 200, "y": 100}, {"x": 300, "y": 100}]})).unwrap();
    let a = NodeId(result["id"].as_u64().unwrap());
    let b = line(&mut s, (200.0, 100.0), (200.0, 200.0));
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.select_tool("pen", view()).unwrap();
    assert_eq!(s.cursor(Point::new(300.0, 100.0), Mods::default(), view()), Cursor::PenContinue);
    gesture(&mut s, view(), &[(300.0, 100.0)], Mods::default());
    assert_eq!(path_ids(&s), vec![a, b]);
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    (s, a, b)
}

#[test]
fn alt_at_another_endpoint_reports_conversion_and_preserves_objects_through_undo_redo() {
    let (mut s, a, b) = overlapping_endpoint();
    let before_a = node(&s, a).clone();
    let before_b = node(&s, b).clone();
    let before_undo = s.doc().unwrap().history.undo.len();
    let alt = Mods { alt: true, ..Default::default() };
    let advertised = s.cursor(Point::new(200.0, 100.0), alt, view());
    gesture(&mut s, view(), &[(200.0, 100.0), (200.0, 120.0)], alt);

    let mut converted_a = before_a.clone();
    let NodeKind::Path { path, .. } = &mut converted_a.kind else { panic!("fixture must be a path") };
    let middle = &mut path.subpaths[0].anchors[1];
    middle.kind = AnchorKind::Smooth;
    middle.h_in = Point::new(200.0, 80.0);
    middle.h_out = Point::new(200.0, 120.0);
    assert_eq!(path_ids(&s), vec![a, b], "Alt conversion must not join away either object");
    assert_eq!(node(&s, a), &converted_a, "only A's interior corner becomes smooth");
    assert_eq!(node(&s, b), &before_b, "B's endpoint is under the pointer but B must stay unchanged");
    assert_eq!(s.doc().unwrap().history.undo.len(), before_undo + 1);
    assert_eq!(s.execute("edit.undo", &json!({})).unwrap()["undone"], "Convert Anchor Point");
    assert_eq!(path_ids(&s), vec![a, b]);
    assert_eq!(node(&s, a), &before_a);
    assert_eq!(node(&s, b), &before_b);
    assert_eq!(s.doc().unwrap().history.undo.len(), before_undo);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(path_ids(&s), vec![a, b]);
    assert_eq!(node(&s, a), &converted_a);
    assert_eq!(node(&s, b), &before_b);
    assert_eq!(s.doc().unwrap().history.undo.len(), before_undo + 1);
    // Check the captured cursor last: a mismatch must not hide the actual conversion/history controls.
    assert_eq!(advertised, Cursor::PenConvert, "Alt cursor must describe the conversion actually performed, not Join");
}

#[test]
fn plain_click_at_same_endpoint_joins_paths_with_undo_redo() {
    let (mut s, a, b) = overlapping_endpoint();
    let before_a = node(&s, a).clone();
    let before_b = node(&s, b).clone();
    let before_undo = s.doc().unwrap().history.undo.len();
    assert_eq!(s.cursor(Point::new(200.0, 100.0), Mods::default(), view()), Cursor::PenJoin);
    gesture(&mut s, view(), &[(200.0, 100.0)], Mods::default());
    assert_eq!(path_ids(&s), vec![a]);
    assert!(s.doc().unwrap().doc.node(b).is_none());
    assert_eq!(points(&s, a), [(100.0, 100.0), (200.0, 100.0), (300.0, 100.0), (200.0, 100.0), (200.0, 200.0)]);
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    assert_eq!(s.doc().unwrap().history.undo.len(), before_undo + 1);
    let joined = node(&s, a).clone();
    assert_eq!(s.execute("edit.undo", &json!({})).unwrap()["undone"], "Join");
    assert_eq!(path_ids(&s), vec![a, b]);
    assert_eq!(node(&s, a), &before_a);
    assert_eq!(node(&s, b), &before_b);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(path_ids(&s), vec![a]);
    assert_eq!(node(&s, a), &joined);
    assert!(s.doc().unwrap().doc.node(b).is_none());
}

fn grid_continue(preselected: bool, snap_to_grid: bool) {
    let mut s = session();
    let mut prefs = s.prefs.clone();
    prefs.gridline_every = 100.0;
    prefs.grid_subdivisions = 1;
    s.apply_prefs(prefs);
    assert_eq!(s.doc().unwrap().doc.grid.spacing, 100.0);
    assert_eq!(s.doc().unwrap().doc.grid.subdivisions, 1);
    let id = line(&mut s, (13.0, 303.0), (63.0, 303.0));
    let original = node(&s, id).clone();
    s.execute("select.set", &json!({"ids": if preselected { vec![id.0] } else { vec![] }})).unwrap();
    let view = ViewInfo { snap_to_grid, ..view() };
    s.select_tool("pen", view).unwrap();
    assert_eq!(s.cursor(Point::new(63.0, 303.0), Mods::default(), view), Cursor::PenContinue);
    let journal_start = s.journal.len();
    gesture(&mut s, view, &[(63.0, 303.0)], Mods::default());
    assert_eq!(path_ids(&s), vec![id], "clicking the advertised endpoint must continue it, not create a snapped path");
    assert_eq!(node(&s, id), &original);
    assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
    assert!(!s.journal[journal_start..].iter().any(|(command, _)| command == "path.create"));
    let before_append = s.doc().unwrap().history.undo.len();
    gesture(&mut s, view, &[(200.0, 300.0)], Mods::default());
    assert_eq!(path_ids(&s), vec![id]);
    assert_eq!(points(&s, id), [(13.0, 303.0), (63.0, 303.0), (200.0, 300.0)]);
    assert_eq!(s.doc().unwrap().history.undo.len(), before_append + 1);
    let extended = node(&s, id).clone();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, id), &original);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(node(&s, id), &extended);
}

#[test]
fn unselected_grid_snapped_endpoint_continues_at_raw_pointer() {
    grid_continue(false, true);
}

#[test]
fn preselected_grid_snapped_endpoint_continues_at_raw_pointer() {
    grid_continue(true, true);
}

#[test]
fn unselected_endpoint_continues_with_grid_snap_disabled() {
    grid_continue(false, false);
}

fn nearest_first_continue(preselected: bool) {
    let mut s = session();
    let id = line(&mut s, (100.0, 300.0), (106.0, 300.0));
    s.execute("select.set", &json!({"ids": if preselected { vec![id.0] } else { vec![] }})).unwrap();
    s.select_tool("pen", view()).unwrap();
    // Both ends are within the explicit 8 px tolerance, but the first is exactly under the pointer.
    assert_eq!(s.cursor(Point::new(100.0, 300.0), Mods::default(), view()), Cursor::PenContinue);
    gesture(&mut s, view(), &[(100.0, 300.0)], Mods::default());
    assert_eq!(path_ids(&s), vec![id]);
    assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
    gesture(&mut s, view(), &[(80.0, 300.0)], Mods::default());
    assert_eq!(path_ids(&s), vec![id]);
    assert_eq!(points(&s, id), [(106.0, 300.0), (100.0, 300.0), (80.0, 300.0)], "continue from the nearer first end");
}

#[test]
fn unselected_short_path_continues_nearest_first_endpoint() {
    nearest_first_continue(false);
}

#[test]
fn preselected_short_path_continues_nearest_first_endpoint() {
    nearest_first_continue(true);
}

#[test]
fn short_other_path_joins_nearest_endpoint_and_keeps_last_on_tie() {
    // Both ends of B are within 8 px. First/last must win by distance; the midpoint keeps Last.
    // Snapping is disabled here to isolate endpoint choice; active joins keep upstream's snapped input.
    for (x, first, last) in [(100.0, 100.0, 106.0), (106.0, 106.0, 100.0), (103.0, 106.0, 100.0)] {
        let mut s = session();
        let a = line(&mut s, (20.0, 300.0), (40.0, 300.0));
        let b = line(&mut s, (100.0, 300.0), (106.0, 300.0));
        s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
        s.select_tool("pen", view()).unwrap();
        gesture(&mut s, view(), &[(40.0, 300.0)], Mods::default());
        let before_a = node(&s, a).clone();
        let before_b = node(&s, b).clone();
        let before_undo = s.doc().unwrap().history.undo.len();

        assert_eq!(s.cursor(Point::new(x, 300.0), Mods::default(), view()), Cursor::PenJoin);
        gesture(&mut s, view(), &[(x, 300.0)], Mods::default());
        assert_eq!(path_ids(&s), vec![a], "x={x}");
        assert!(s.doc().unwrap().doc.node(b).is_none());
        assert_eq!(points(&s, a), [(20.0, 300.0), (40.0, 300.0), (first, 300.0), (last, 300.0)], "x={x}");
        assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
        assert_eq!(s.doc().unwrap().history.undo.len(), before_undo + 1);
        let joined = node(&s, a).clone();

        assert_eq!(s.execute("edit.undo", &json!({})).unwrap()["undone"], "Join");
        assert_eq!(path_ids(&s), vec![a, b]);
        assert_eq!(node(&s, a), &before_a);
        assert_eq!(node(&s, b), &before_b);
        assert_eq!(s.doc().unwrap().history.undo.len(), before_undo);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(path_ids(&s), vec![a]);
        assert_eq!(node(&s, a), &joined);
        assert!(s.doc().unwrap().doc.node(b).is_none());
        assert_eq!(s.doc().unwrap().history.undo.len(), before_undo + 1);
    }
}

fn grid_join(from: (f64, f64), to: (f64, f64), at: (f64, f64), snap_to_grid: bool, expected_cursor: Cursor) {
    let mut s = session();
    let mut prefs = s.prefs.clone();
    prefs.gridline_every = 100.0;
    prefs.grid_subdivisions = 1;
    s.apply_prefs(prefs);
    assert_eq!(s.doc().unwrap().doc.grid.spacing, 100.0);
    assert_eq!(s.doc().unwrap().doc.grid.subdivisions, 1);
    let v = ViewInfo { snap_to_grid, ..view() };
    let a = line(&mut s, (20.0, 100.0), (40.0, 100.0));
    let b = line(&mut s, from, to);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.select_tool("pen", v).unwrap();
    assert_eq!(s.cursor(Point::new(40.0, 100.0), Mods::default(), v), Cursor::PenContinue);
    gesture(&mut s, v, &[(40.0, 100.0)], Mods::default());
    assert_eq!(path_ids(&s), vec![a, b]);
    let before_a = node(&s, a).clone();
    let before_b = node(&s, b).clone();
    let before_undo = s.doc().unwrap().history.undo.len();

    assert_eq!(s.cursor(Point::new(at.0, at.1), Mods::default(), v), expected_cursor);
    gesture(&mut s, v, &[at], Mods::default());
    assert_eq!(path_ids(&s), vec![a], "the endpoint press joins B into A");
    assert!(s.doc().unwrap().doc.node(b).is_none());
    assert_eq!(points(&s, a), [(20.0, 100.0), (40.0, 100.0), from, to], "join B's first end, not its farther last end");
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    assert_eq!(s.doc().unwrap().history.undo.len(), before_undo + 1);
    let joined = node(&s, a).clone();

    assert_eq!(s.execute("edit.undo", &json!({})).unwrap()["undone"], "Join");
    assert_eq!(path_ids(&s), vec![a, b]);
    assert_eq!(node(&s, a), &before_a);
    assert_eq!(node(&s, b), &before_b);
    assert_eq!(s.doc().unwrap().history.undo.len(), before_undo);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(path_ids(&s), vec![a]);
    assert_eq!(node(&s, a), &joined);
    assert!(s.doc().unwrap().doc.node(b).is_none());
    assert_eq!(s.doc().unwrap().history.undo.len(), before_undo + 1);
}

#[test]
fn off_grid_other_endpoint_joins_without_grid_snapping() {
    grid_join((63.0, 303.0), (113.0, 303.0), (63.0, 303.0), false, Cursor::PenJoin);
}

#[test]
fn off_grid_other_endpoint_joins_before_grid_snapping() {
    // The raw pointer hits B's first end; (100, 300) from grid snapping hits neither end.
    grid_join((63.0, 303.0), (113.0, 303.0), (63.0, 303.0), true, Cursor::PenJoin);
}

#[test]
fn overlapping_other_endpoints_join_nearest_raw_end_before_grid_snapping() {
    // Raw picks (94, 303), but snapping to (100, 300) would wrongly favor (100, 303).
    grid_join((94.0, 303.0), (100.0, 303.0), (94.0, 303.0), true, Cursor::PenJoin);
}

#[test]
fn another_path_join_keeps_snapped_target_when_raw_pointer_misses() {
    // Upstream also joins through snapping when raw hits no endpoint. Keep that fallback.
    grid_join((100.0, 300.0), (150.0, 300.0), (63.0, 303.0), true, Cursor::Pen);
}

fn snapped_only_continue(preselected: bool) {
    for first in [true, false] {
        let mut s = session();
        let mut prefs = s.prefs.clone();
        prefs.gridline_every = 100.0;
        prefs.grid_subdivisions = 1;
        s.apply_prefs(prefs);
        assert_eq!(s.doc().unwrap().doc.grid.spacing, 100.0);
        assert_eq!(s.doc().unwrap().doc.grid.subdivisions, 1);
        let id = line(&mut s, (100.0, 300.0), (200.0, 300.0));
        let original = node(&s, id).clone();
        s.execute("select.set", &json!({"ids": if preselected { vec![id.0] } else { vec![] }})).unwrap();
        let v = ViewInfo { snap_to_grid: true, ..view() };
        s.select_tool("pen", v).unwrap();
        let at = if first { (63.0, 303.0) } else { (237.0, 303.0) };
        let end = if first { Point::new(100.0, 300.0) } else { Point::new(200.0, 300.0) };
        assert_eq!(s.cursor(end, Mods::default(), v), Cursor::PenContinue, "the endpoint is eligible");
        assert_eq!(s.cursor(Point::new(at.0, at.1), Mods::default(), v), Cursor::Pen, "the raw pointer misses both ends");
        let journal_start = s.journal.len();
        let before_continue = s.doc().unwrap().history.undo.len();
        gesture(&mut s, v, &[at], Mods::default());

        assert_eq!(path_ids(&s), vec![id], "snapped-only continuation must keep one original path: selected={preselected}, first={first}");
        assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
        let expected = if first { [(200.0, 300.0), (100.0, 300.0)] } else { [(100.0, 300.0), (200.0, 300.0)] };
        assert_eq!(points(&s, id), expected, "continue from the snapped end: selected={preselected}, first={first}");
        assert!(!s.journal[journal_start..].iter().any(|(command, _)| command == "path.create"));
        assert!(!s.in_interaction() && !s.tool_busy());
        assert_eq!(s.doc().unwrap().history.undo.len(), before_continue + usize::from(first));
        let continued = node(&s, id).clone();
        if !first {
            assert_eq!(continued, original, "continuing from the last end does not edit the path");
        }

        let before_append = s.doc().unwrap().history.undo.len();
        let next = if first { (0.0, 300.0) } else { (300.0, 300.0) };
        gesture(&mut s, v, &[next], Mods::default());
        assert_eq!(path_ids(&s), vec![id]);
        assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
        assert_eq!(points(&s, id), [expected[0], expected[1], next]);
        assert!(!s.journal[journal_start..].iter().any(|(command, _)| command == "path.create"));
        assert!(!s.in_interaction() && !s.tool_busy());
        assert_eq!(s.doc().unwrap().history.undo.len(), before_append + 1);
        let extended = node(&s, id).clone();
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(path_ids(&s), vec![id]);
        assert_eq!(node(&s, id), &continued);
        assert_eq!(s.doc().unwrap().history.undo.len(), before_append);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(path_ids(&s), vec![id]);
        assert_eq!(node(&s, id), &extended);
        assert_eq!(s.doc().unwrap().history.undo.len(), before_append + 1);
    }
}

#[test]
fn selected_snapped_only_endpoint_continues_when_raw_pointer_misses() {
    snapped_only_continue(true);
}

#[test]
fn unselected_snapped_only_endpoint_continues_when_raw_pointer_misses() {
    snapped_only_continue(false);
}

#[test]
fn raw_unselected_endpoint_precedes_snapped_selected_endpoint() {
    let mut s = session();
    let mut prefs = s.prefs.clone();
    prefs.gridline_every = 100.0;
    prefs.grid_subdivisions = 1;
    s.apply_prefs(prefs);
    assert_eq!(s.doc().unwrap().doc.grid.spacing, 100.0);
    assert_eq!(s.doc().unwrap().doc.grid.subdivisions, 1);
    let selected = line(&mut s, (100.0, 300.0), (150.0, 300.0));
    let raw = line(&mut s, (13.0, 303.0), (63.0, 303.0));
    let original_selected = node(&s, selected).clone();
    let original_raw = node(&s, raw).clone();
    s.execute("select.set", &json!({"ids": [selected.0]})).unwrap();
    let v = ViewInfo { snap_to_grid: true, ..view() };
    s.select_tool("pen", v).unwrap();
    assert_eq!(s.cursor(Point::new(100.0, 300.0), Mods::default(), v), Cursor::PenContinue);
    assert_eq!(s.cursor(Point::new(63.0, 303.0), Mods::default(), v), Cursor::PenContinue);
    let journal_start = s.journal.len();
    let before_append = s.doc().unwrap().history.undo.len();
    gesture(&mut s, v, &[(63.0, 303.0)], Mods::default());

    assert_eq!(s.doc().unwrap().selection.objects, vec![raw], "the raw unselected end wins over the snapped selected end");
    assert_eq!(path_ids(&s), vec![selected, raw]);
    assert_eq!(node(&s, selected), &original_selected);
    assert_eq!(node(&s, raw), &original_raw);
    assert!(!s.journal[journal_start..].iter().any(|(command, _)| command == "path.create"));
    assert!(!s.in_interaction() && !s.tool_busy());
    assert_eq!(s.doc().unwrap().history.undo.len(), before_append);

    gesture(&mut s, v, &[(200.0, 400.0)], Mods::default());
    assert_eq!(path_ids(&s), vec![selected, raw]);
    assert_eq!(s.doc().unwrap().selection.objects, vec![raw]);
    assert_eq!(node(&s, selected), &original_selected);
    assert_eq!(points(&s, raw), [(13.0, 303.0), (63.0, 303.0), (200.0, 400.0)]);
    assert!(!s.journal[journal_start..].iter().any(|(command, _)| command == "path.create"));
    assert!(!s.in_interaction() && !s.tool_busy());
    assert_eq!(s.doc().unwrap().history.undo.len(), before_append + 1);
    let extended = node(&s, raw).clone();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(path_ids(&s), vec![selected, raw]);
    assert_eq!(node(&s, selected), &original_selected);
    assert_eq!(node(&s, raw), &original_raw);
    assert_eq!(s.doc().unwrap().history.undo.len(), before_append);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(path_ids(&s), vec![selected, raw]);
    assert_eq!(node(&s, selected), &original_selected);
    assert_eq!(node(&s, raw), &extended);
    assert_eq!(s.doc().unwrap().history.undo.len(), before_append + 1);
}
