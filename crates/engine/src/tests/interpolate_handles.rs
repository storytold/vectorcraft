//! #7832: General › Interpolate Control Points When Moving Anchors (`interpolateControlPoints`).
//! With it on, a Direct Selection drag or an arrow-key nudge of selected anchors re-maps the
//! handles of each segment with one moved end, so the segment keeps its shape relative to its
//! ends. Off (the default), the handles of a moved anchor move with it and the others stay.

use serde_json::json;
use vectorcraft_geom::{Anchor, PathData, Point};
use vectorcraft_tools::{PointerEvent, PointerKind};

use super::history;
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn interpolate(s: &mut Session, on: bool) {
    s.execute("prefs.set", &json!({"key": "interpolateControlPoints", "value": on})).unwrap();
}

fn path_of(s: &Session, id: NodeId) -> PathData {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().clone()
}

fn anchor(s: &Session, id: NodeId, ai: usize) -> Anchor {
    path_of(s, id).subpaths[0].anchors[ai]
}

fn create(s: &mut Session, anchors: serde_json::Value) -> NodeId {
    NodeId(s.execute("path.create", &json!({ "anchors": anchors })).unwrap()["id"].as_u64().unwrap())
}

/// A curve from (100, 100) to (300, 100) whose handles lie on its chord.
fn curve(s: &mut Session) -> NodeId {
    create(s, json!([{"x": 100, "y": 100, "out": [150, 100]}, {"x": 300, "y": 100, "in": [250, 100]}]))
}

fn select(s: &mut Session, id: NodeId, anchors: serde_json::Value) {
    s.execute("select.anchors", &json!({"id": id.0, "anchors": anchors, "mode": "set"})).unwrap();
}

/// A Direct Selection click on (`x`, `y`), then a drag from there to (`to_x`, `to_y`).
fn drag(s: &mut Session, (x, y): (f64, f64), (to_x, to_y): (f64, f64)) {
    let v = ViewInfo { smart_guides: false, ..ViewInfo::default() };
    s.select_tool("directSelection", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, x, y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, x, y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, x, y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, (x + to_x) / 2.0, (y + to_y) / 2.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, to_x, to_y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, to_x, to_y), v).unwrap();
}

#[test]
fn the_preference_is_off_by_default_and_the_handles_then_move_rigidly() {
    let mut s = session();
    assert!(!s.prefs.interpolate_control_points);
    let id = curve(&mut s);
    select(&mut s, id, json!([[0, 0]]));
    s.execute("object.nudge", &json!({"dx": 20, "dy": 8})).unwrap();
    let (a, b) = (anchor(&s, id, 0), anchor(&s, id, 1));
    assert_eq!(a.p, Point::new(120.0, 108.0));
    assert_eq!(a.h_out, Point::new(170.0, 108.0), "the moved anchor's handle moves with it");
    assert_eq!(b.h_in, Point::new(250.0, 100.0), "the other end's handle stays");
}

#[test]
fn a_nudge_with_the_preference_on_re_maps_both_handles_of_the_segment() {
    let mut s = session();
    interpolate(&mut s, true);
    let id = curve(&mut s);
    select(&mut s, id, json!([[0, 0]]));
    let before = history(&s).0;
    s.execute("object.nudge", &json!({"dx": 20, "dy": 8})).unwrap();
    assert_eq!(history(&s).0, before + 1, "one undo step");
    let (a, b) = (anchor(&s, id, 0), anchor(&s, id, 1));
    // The chord goes from (200, 0) to (180, -8): a scale by 0.9 and a turn, so the handles a
    // quarter of the way from each end stay on the new chord, a quarter of the way along it.
    assert_eq!(a.p, Point::new(120.0, 108.0));
    assert_eq!(a.h_in, a.p, "a retracted handle stays retracted");
    assert_eq!(a.h_out, Point::new(165.0, 106.0));
    assert_eq!(b.p, Point::new(300.0, 100.0));
    assert_eq!(b.h_in, Point::new(255.0, 102.0));
    assert_eq!(b.h_out, b.p);
}

#[test]
fn a_direct_selection_drag_with_the_preference_on_re_maps_the_handles_in_one_undo_step() {
    let mut s = session();
    interpolate(&mut s, true);
    let id = curve(&mut s);
    let original = path_of(&s, id);
    drag(&mut s, (100.0, 100.0), (120.0, 108.0));
    let (a, b) = (anchor(&s, id, 0), anchor(&s, id, 1));
    assert_eq!(a.p, Point::new(120.0, 108.0));
    assert_eq!(a.h_out, Point::new(165.0, 106.0));
    assert_eq!(b.h_in, Point::new(255.0, 102.0));
    // One undo step puts the path back as it was before the drag.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(path_of(&s, id), original);
}

#[test]
fn a_direct_selection_drag_with_the_preference_off_keeps_the_rigid_move() {
    let mut s = session();
    let id = curve(&mut s);
    drag(&mut s, (100.0, 100.0), (120.0, 108.0));
    let (a, b) = (anchor(&s, id, 0), anchor(&s, id, 1));
    assert_eq!(a.h_out, Point::new(170.0, 108.0));
    assert_eq!(b.h_in, Point::new(250.0, 100.0));
}

#[test]
fn a_segment_with_both_ends_moved_moves_rigidly() {
    let mut s = session();
    interpolate(&mut s, true);
    let id = create(
        &mut s,
        json!([
            {"x": 100, "y": 100, "out": [150, 60]},
            {"x": 300, "y": 100, "in": [250, 60], "out": [350, 150]},
            {"x": 500, "y": 100, "in": [450, 140]},
        ]),
    );
    select(&mut s, id, json!([[0, 0], [0, 1]]));
    s.execute("object.nudge", &json!({"dx": 10, "dy": 5})).unwrap();
    let (a, b, c) = (anchor(&s, id, 0), anchor(&s, id, 1), anchor(&s, id, 2));
    assert_eq!((a.p, a.h_out), (Point::new(110.0, 105.0), Point::new(160.0, 65.0)));
    assert_eq!((b.p, b.h_in), (Point::new(310.0, 105.0), Point::new(260.0, 65.0)));
    // The segment to the unmoved third anchor is re-mapped: its chord goes from (200, 0) to
    // (190, -5), a scale by 0.95 with a turn of -0.025 per unit.
    close(b.h_out, Point::new(358.75, 151.25));
    close(c.h_in, Point::new(453.5, 139.25));
    assert_eq!(c.p, Point::new(500.0, 100.0));
}

fn close(p: Point, q: Point) {
    assert!((p.x - q.x).abs() < 1e-9 && (p.y - q.y).abs() < 1e-9, "{p:?} is not {q:?}");
}

/// A closed square from (100, 100) to (300, 300) whose handles lie on its sides, a quarter of
/// the way along.
fn square(s: &mut Session) -> NodeId {
    let anchors = json!([
        {"x": 100, "y": 100, "in": [100, 150], "out": [150, 100]},
        {"x": 300, "y": 100, "in": [250, 100], "out": [300, 150]},
        {"x": 300, "y": 300, "in": [300, 250], "out": [250, 300]},
        {"x": 100, "y": 300, "in": [150, 300], "out": [100, 250]},
    ]);
    NodeId(s.execute("path.create", &json!({ "anchors": anchors, "closed": true })).unwrap()["id"].as_u64().unwrap())
}

#[test]
fn moving_the_first_anchor_of_a_closed_path_re_maps_the_closing_segment_too() {
    let mut s = session();
    interpolate(&mut s, true);
    let id = square(&mut s);
    select(&mut s, id, json!([[0, 0]]));
    s.execute("object.nudge", &json!({"dx": 0, "dy": -200})).unwrap();
    let (a, b, c, d) = (anchor(&s, id, 0), anchor(&s, id, 1), anchor(&s, id, 2), anchor(&s, id, 3));
    assert_eq!(a.p, Point::new(100.0, -100.0));
    // The segment to the second anchor: its chord goes from (200, 0) to (200, 200).
    close(a.h_out, Point::new(150.0, -50.0));
    close(b.h_in, Point::new(250.0, 50.0));
    // The closing segment from the fourth anchor: its chord goes from (0, -200) to (0, -400).
    close(d.h_out, Point::new(100.0, 200.0));
    close(a.h_in, Point::new(100.0, 0.0));
    // The segment between the third and fourth anchors is not beside the moved anchor.
    assert_eq!((c.h_out, d.h_in), (Point::new(250.0, 300.0), Point::new(150.0, 300.0)));
    assert_eq!((b.h_out, c.h_in), (Point::new(300.0, 150.0), Point::new(300.0, 250.0)));
}

#[test]
fn moving_the_last_anchor_of_an_open_path_re_maps_only_the_segment_before_it() {
    let mut s = session();
    interpolate(&mut s, true);
    let id = create(
        &mut s,
        json!([
            {"x": 100, "y": 100, "out": [150, 100]},
            {"x": 300, "y": 100, "in": [250, 100], "out": [350, 50]},
            {"x": 500, "y": 100, "in": [450, 100], "out": [550, 50]},
        ]),
    );
    select(&mut s, id, json!([[0, 2]]));
    s.execute("object.nudge", &json!({"dx": 0, "dy": 200})).unwrap();
    let (a, b, c) = (anchor(&s, id, 0), anchor(&s, id, 1), anchor(&s, id, 2));
    assert_eq!(c.p, Point::new(500.0, 300.0));
    // The chord from the second (corner) anchor goes from (200, 0) to (200, 200).
    close(b.h_out, Point::new(400.0, 100.0));
    close(c.h_in, Point::new(450.0, 250.0));
    // The last anchor's own outgoing handle leads to no segment and moves with it.
    assert_eq!(c.h_out, Point::new(550.0, 250.0));
    assert_eq!((a.h_out, b.h_in), (Point::new(150.0, 100.0), Point::new(250.0, 100.0)));
}

#[test]
fn move_and_transform_of_anchors_re_map_the_handles_in_one_undo_step() {
    for (cmd, params) in [("object.move", json!({"dx": 20, "dy": 8})), ("object.transform", json!({"matrix": [1, 0, 0, 1, 20, 8], "anchors": true}))]
    {
        let mut s = session();
        interpolate(&mut s, true);
        let id = curve(&mut s);
        select(&mut s, id, json!([[0, 0]]));
        let (before, original) = (history(&s).0, path_of(&s, id));
        s.execute(cmd, &params).unwrap();
        assert_eq!(history(&s).0, before + 1, "{cmd}: one undo step");
        let (a, b) = (anchor(&s, id, 0), anchor(&s, id, 1));
        assert_eq!(a.p, Point::new(120.0, 108.0), "{cmd}");
        close(a.h_out, Point::new(165.0, 106.0));
        close(b.h_in, Point::new(255.0, 102.0));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(path_of(&s, id), original, "{cmd}: undone");
    }
}

#[test]
fn set_bounds_and_guide_move_of_anchors_follow_the_preference_and_the_param() {
    for (pref, param, re_mapped) in [(false, None, false), (true, None, true), (true, Some(false), false), (false, Some(true), true)] {
        for cmd in ["object.setBounds", "guide.move"] {
            let mut s = session();
            interpolate(&mut s, pref);
            let id = curve(&mut s);
            select(&mut s, id, json!([[0, 0]]));
            let mut params = if cmd == "object.setBounds" {
                json!({"x": 120, "y": 108})
            } else {
                let g = s.execute("guide.add", &json!({"vertical": true, "pos": 50})).unwrap()["index"].as_u64().unwrap();
                s.execute("guide.select", &json!({"indexes": [g], "toggle": true})).unwrap();
                json!({"dx": 20, "dy": 8})
            };
            if let Some(on) = param {
                params["interpolate"] = json!(on);
            }
            let (before, original) = (history(&s).0, path_of(&s, id));
            s.execute(cmd, &params).unwrap();
            assert_eq!(history(&s).0, before + 1, "{cmd}: one undo step");
            let (a, b) = (anchor(&s, id, 0), anchor(&s, id, 1));
            assert_eq!(a.p, Point::new(120.0, 108.0), "{cmd}");
            if re_mapped {
                close(a.h_out, Point::new(165.0, 106.0));
                close(b.h_in, Point::new(255.0, 102.0));
            } else {
                assert_eq!((a.h_out, b.h_in), (Point::new(170.0, 108.0), Point::new(250.0, 100.0)), "{cmd}, pref {pref}, param {param:?}");
            }
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(path_of(&s, id), original, "{cmd}: undone");
        }
    }
    let mut s = session();
    let id = curve(&mut s);
    // Both anchors, so that a new width scales them and does not only move them.
    select(&mut s, id, json!([[0, 0], [0, 1]]));
    s.execute("guide.add", &json!({"vertical": true, "pos": 50})).unwrap();
    s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
    let original = path_of(&s, id);
    for (cmd, params) in [
        ("object.setBounds", json!({"x": 120, "interpolate": "yes"})),
        ("object.setBounds", json!({"width": 50, "interpolate": 1})),
        ("guide.move", json!({"dx": 1, "interpolate": "yes"})),
    ] {
        let e = s.execute(cmd, &params).unwrap_err().to_string();
        assert!(e.contains("`interpolate` must be true or false"), "{cmd}: {e}");
    }
    assert_eq!(path_of(&s, id), original);
    // With only guides selected, guide.move rejects the value too and leaves the guide.
    s.execute("select.none", &json!({})).unwrap();
    s.execute("guide.select", &json!({"indexes": [0]})).unwrap();
    let e = s.execute("guide.move", &json!({"dx": 1, "interpolate": "yes"})).unwrap_err().to_string();
    assert!(e.contains("`interpolate` must be true or false"), "guide.move: {e}");
    assert_eq!(s.doc().unwrap().doc.ruler_guides()[0].pos, 50.0);
}

#[test]
fn the_interpolate_param_overrides_the_preference_and_rejects_other_values() {
    for (pref, param, re_mapped) in [(true, false, false), (false, true, true)] {
        let mut s = session();
        interpolate(&mut s, pref);
        let id = curve(&mut s);
        select(&mut s, id, json!([[0, 0]]));
        s.execute("path.moveAnchors", &json!({"dx": 20, "dy": 8, "interpolate": param})).unwrap();
        let b = anchor(&s, id, 1);
        assert_eq!(b.h_in != Point::new(250.0, 100.0), re_mapped, "pref {pref}, param {param}: {:?}", b.h_in);
    }
    let mut s = session();
    let id = curve(&mut s);
    select(&mut s, id, json!([[0, 0]]));
    let original = path_of(&s, id);
    for (cmd, params) in [
        ("path.moveAnchors", json!({"dx": 1, "dy": 0, "interpolate": "yes"})),
        ("object.nudge", json!({"dx": 1, "dy": 0, "interpolate": "yes"})),
        ("object.move", json!({"dx": 1, "dy": 0, "interpolate": "yes"})),
        ("object.transform", json!({"matrix": [1, 0, 0, 1, 1, 0], "anchors": true, "interpolate": 1})),
        ("object.transformAgain", json!({"interpolate": "yes"})),
    ] {
        let e = s.execute(cmd, &params).unwrap_err().to_string();
        assert!(e.contains("`interpolate` must be true or false"), "{cmd}: {e}");
    }
    assert_eq!(path_of(&s, id), original);
}

#[test]
fn a_replayed_move_re_maps_the_handles_as_it_did_whatever_the_preference_is_then() {
    let mut s = session();
    interpolate(&mut s, true);
    let id = curve(&mut s);
    select(&mut s, id, json!([[0, 0]]));
    let start = s.journal.len();
    s.execute("object.nudge", &json!({"dx": 20, "dy": 8})).unwrap();
    let moved = path_of(&s, id);
    let (cmd, params) = s.journal.since(start).last().cloned().unwrap();
    assert_eq!(params["interpolate"], json!(true), "{cmd}: journaled");
    // Undo, turn the preference off and play the journal entry again.
    s.execute("edit.undo", &json!({})).unwrap();
    interpolate(&mut s, false);
    select(&mut s, id, json!([[0, 0]]));
    s.replay(&cmd, &params).unwrap();
    assert_eq!(path_of(&s, id), moved);
}

#[test]
fn a_saved_step_without_interpolate_moves_the_handles_rigidly_whatever_the_preference() {
    // A step recorded before the preference existed has no `interpolate`; with the preference on,
    // it still plays as it did when it was recorded.
    for cmd in ["path.moveAnchors", "object.nudge", "object.move"] {
        let mut s = session();
        interpolate(&mut s, true);
        let id = curve(&mut s);
        select(&mut s, id, json!([[0, 0]]));
        s.replay(cmd, &json!({"dx": 1, "dy": 1})).unwrap();
        let (a, b) = (anchor(&s, id, 0), anchor(&s, id, 1));
        assert_ne!(a.p, Point::new(100.0, 100.0), "{cmd}: the anchor moves");
        assert_eq!((a.h_out.x - a.p.x, a.h_out.y - a.p.y), (50.0, 0.0), "{cmd}: the moved anchor's handle moves with it");
        assert_eq!(b.h_in, Point::new(250.0, 100.0), "{cmd}: the other end's handle stays");
        // Run directly, the same params follow the preference.
        s.execute(cmd, &json!({"dx": 1, "dy": 1})).unwrap();
        assert_ne!(anchor(&s, id, 1).h_in, Point::new(250.0, 100.0), "{cmd}: executed, the handles are re-mapped");
    }
}

#[test]
fn transform_again_keeps_the_setting_its_move_used() {
    let mut s = session();
    interpolate(&mut s, true);
    let id = curve(&mut s);
    select(&mut s, id, json!([[0, 0]]));
    s.execute("object.nudge", &json!({"dx": 20, "dy": 8})).unwrap();
    interpolate(&mut s, false);
    s.execute("object.transformAgain", &json!({})).unwrap();
    let mut t = session();
    interpolate(&mut t, true);
    let other = curve(&mut t);
    select(&mut t, other, json!([[0, 0]]));
    t.execute("object.nudge", &json!({"dx": 20, "dy": 8})).unwrap();
    t.execute("object.nudge", &json!({"dx": 20, "dy": 8})).unwrap();
    assert_eq!(path_of(&s, id), path_of(&t, other));
}

#[test]
fn a_segment_whose_ends_are_at_the_same_place_moves_rigidly() {
    let mut s = session();
    interpolate(&mut s, true);
    let id = create(&mut s, json!([{"x": 100, "y": 100, "out": [150, 50]}, {"x": 100, "y": 100, "in": [50, 150]}]));
    select(&mut s, id, json!([[0, 0]]));
    s.execute("object.nudge", &json!({"dx": 10, "dy": 0})).unwrap();
    let (a, b) = (anchor(&s, id, 0), anchor(&s, id, 1));
    assert_eq!((a.p, a.h_out), (Point::new(110.0, 100.0), Point::new(160.0, 50.0)));
    assert_eq!((b.p, b.h_in), (Point::new(100.0, 100.0), Point::new(50.0, 150.0)));
}

#[test]
fn a_move_onto_the_other_end_moves_the_handles_rigidly() {
    let mut s = session();
    interpolate(&mut s, true);
    let id = curve(&mut s);
    select(&mut s, id, json!([[0, 0]]));
    s.execute("object.nudge", &json!({"dx": 200, "dy": 0})).unwrap();
    let (a, b) = (anchor(&s, id, 0), anchor(&s, id, 1));
    assert_eq!((a.p, a.h_out), (Point::new(300.0, 100.0), Point::new(350.0, 100.0)));
    assert_eq!(b.h_in, Point::new(250.0, 100.0));
}

#[test]
fn a_nudge_of_zero_with_the_preference_on_records_no_undo_step() {
    let mut s = session();
    interpolate(&mut s, true);
    let id = curve(&mut s);
    select(&mut s, id, json!([[0, 0]]));
    let (before, original) = (history(&s), path_of(&s, id));
    s.execute("object.nudge", &json!({"dx": 0, "dy": 0})).unwrap();
    assert_eq!(history(&s), before);
    assert_eq!(path_of(&s, id), original);
}

#[test]
fn a_smooth_anchor_keeps_its_handles_in_line() {
    let mut s = session();
    interpolate(&mut s, true);
    let id = create(
        &mut s,
        json!([
            {"x": 100, "y": 100, "out": [150, 100]},
            {"x": 300, "y": 100, "in": [250, 100], "out": [350, 100], "smooth": true},
            {"x": 500, "y": 100, "in": [450, 100]},
        ]),
    );
    select(&mut s, id, json!([[0, 0]]));
    s.execute("object.nudge", &json!({"dx": 0, "dy": -100})).unwrap();
    let b = anchor(&s, id, 1);
    assert_eq!(b.p, Point::new(300.0, 100.0));
    // The segment to the third anchor did not move, so its handle stays where it was.
    assert_eq!(b.h_out, Point::new(350.0, 100.0));
    // The incoming handle grows with the chord from the moved anchor (from 200 to 100√5) and
    // turns back in line with the outgoing one.
    assert_eq!(b.h_in.y, 100.0, "in line: {:?}", b.h_in);
    assert!((300.0 - b.h_in.x - 25.0 * 5f64.sqrt()).abs() < 1e-9, "{:?}", b.h_in);
}

#[test]
fn a_moved_smooth_anchor_keeps_a_retracted_handle_exactly_retracted() {
    let mut s = session();
    interpolate(&mut s, true);
    let id = create(
        &mut s,
        json!([
            {"x": 0, "y": 100, "out": [50, 100]},
            {"x": 200, "y": 100, "in": [200, 100], "out": [250, 150], "smooth": true},
            {"x": 400, "y": 100, "in": [350, 100]},
        ]),
    );
    select(&mut s, id, json!([[0, 1]]));
    s.execute("object.nudge", &json!({"dx": 20, "dy": 8.1})).unwrap();
    let b = anchor(&s, id, 1);
    assert_eq!(b.p, Point::new(220.0, 108.1));
    // Re-mapping the retracted handle by its segment's map would leave it a rounding error away
    // from the anchor, and lining the handles up would then turn the outgoing one by that error.
    assert_eq!(b.h_in, b.p, "the retracted handle stays retracted");
    // The chord to the third anchor goes from (200, 0) to (180, -8.1).
    close(b.h_out, Point::new(267.025, 151.075));
}

#[test]
fn transform_again_repeats_a_move_with_the_handles_re_mapped() {
    let twice = |again: bool| {
        let mut s = session();
        interpolate(&mut s, true);
        let id = curve(&mut s);
        select(&mut s, id, json!([[0, 0]]));
        s.execute("object.nudge", &json!({"dx": 20, "dy": 8})).unwrap();
        if again {
            s.execute("object.transformAgain", &json!({})).unwrap();
        } else {
            s.execute("object.nudge", &json!({"dx": 20, "dy": 8})).unwrap();
        }
        path_of(&s, id)
    };
    let (repeated, nudged) = (twice(true), twice(false));
    assert_eq!(repeated.subpaths[0].anchors[0].p, Point::new(140.0, 116.0));
    assert_eq!(repeated, nudged);
}

#[test]
fn a_transform_that_scales_by_a_hair_still_scales_the_anchors() {
    let mut s = session();
    interpolate(&mut s, true);
    let id = curve(&mut s);
    select(&mut s, id, json!([[0, 0]]));
    let k = 1.000_000_000_5;
    s.execute("object.transform", &json!({"matrix": [k, 0, 0, 1, 0, 0], "anchors": true})).unwrap();
    let a = anchor(&s, id, 0);
    assert_eq!(a.p, Point::new(100.0 * k, 100.0));
    assert_eq!(a.h_out, Point::new(150.0 * k, 100.0));
}

#[test]
fn a_smooth_anchor_whose_re_mapped_handles_point_the_same_way_moves_them_with_it() {
    let mut s = session();
    interpolate(&mut s, true);
    let id = create(
        &mut s,
        json!([
            {"x": 100, "y": 100, "out": [150, 100]},
            {"x": 300, "y": 100, "in": [250, 100], "out": [350, 100], "smooth": true},
            {"x": 500, "y": 100, "in": [450, 100]},
        ]),
    );
    select(&mut s, id, json!([[0, 1]]));
    s.execute("object.nudge", &json!({"dx": 300, "dy": 0})).unwrap();
    // Both chords re-map their handle to the left of the anchor, which has no direction between
    // them: both handles move with the anchor, as with the preference off.
    let b = anchor(&s, id, 1);
    assert_eq!(b.p, Point::new(600.0, 100.0));
    assert_eq!(b.h_out, Point::new(650.0, 100.0));
    assert_eq!(b.h_in, Point::new(550.0, 100.0));
}

#[test]
fn a_smooth_anchor_between_two_re_mapped_segments_keeps_its_handles_in_line() {
    let mut s = session();
    interpolate(&mut s, true);
    let id = create(
        &mut s,
        json!([
            {"x": 100, "y": 100, "out": [150, 100]},
            {"x": 300, "y": 100, "in": [250, 100], "out": [350, 100], "smooth": true},
            {"x": 500, "y": 100, "in": [450, 100]},
        ]),
    );
    select(&mut s, id, json!([[0, 1]]));
    s.execute("object.nudge", &json!({"dx": 0, "dy": -100})).unwrap();
    let b = anchor(&s, id, 1);
    let (v_in, v_out) = (b.h_in - b.p, b.h_out - b.p);
    assert_eq!(b.p, Point::new(300.0, 0.0));
    // The two chords turn by the same angle in opposite directions, so the handles point along
    // the mean of the two: across, at the length each chord gives them.
    assert!(v_in.cross(v_out).abs() < 1e-9, "the handles stay in line: {:?} {:?}", b.h_in, b.h_out);
    assert!(v_in.dot(v_out) < 0.0, "on opposite sides of the anchor");
    assert!(v_out.y.abs() < 1e-9 && v_out.x > 0.0, "{:?}", b.h_out);
    assert!((v_out.hypot() - 25.0 * 5f64.sqrt()).abs() < 1e-9 && (v_in.hypot() - 25.0 * 5f64.sqrt()).abs() < 1e-9);
}
