//! Pen lends Direct Selection for one handle gesture; Space repositions that handle's owner.
// Integration tests may unwrap and panic (AGENTS.md, Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::{Session, ViewInfo};
use vectorcraft_geom::{Anchor, AnchorKind, PathData, Point};
use vectorcraft_tools::{Cursor, Mods, PointerEvent, PointerKind, ToolKey};

fn view() -> ViewInfo {
    ViewInfo {
        zoom: 1.0,
        smart_guides: false,
        guides: false,
        snap_to_grid: false,
        snap_to_pixel: false,
        snap_to_point: false,
        snap_to_glyph: false,
        pixel_preview: false,
        ..Default::default()
    }
}

fn command(space: bool) -> Mods {
    Mods { cmd: true, space, ..Default::default() }
}

fn pointer(s: &mut Session, kind: PointerKind, at: (f64, f64), mods: Mods) {
    s.pointer(&PointerEvent::new(kind, at.0, at.1).with_mods(mods), view()).unwrap();
}

fn path(s: &Session, id: NodeId) -> PathData {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().clone()
}

fn history(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn open_pen_path() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s.prefs.selection_tolerance = 8.0;
    s.prefs.use_precise_cursors = false;
    // Choose the tool the Pen will lend before beginning its path.
    s.select_tool("directSelection", view()).unwrap();
    s.select_tool("pen", view()).unwrap();
    let none = Mods::default();
    pointer(&mut s, PointerKind::Down, (100.0, 300.0), none);
    pointer(&mut s, PointerKind::Up, (100.0, 300.0), none);
    pointer(&mut s, PointerKind::Down, (200.0, 300.0), none);
    pointer(&mut s, PointerKind::Drag, (250.0, 300.0), none);
    pointer(&mut s, PointerKind::Up, (250.0, 300.0), none);
    let id = s.doc().unwrap().selection.objects[0];
    let p = path(&s, id);
    assert_eq!(p.subpaths.len(), 1);
    assert!(!p.subpaths[0].closed);
    assert_eq!(p.subpaths[0].anchors.len(), 2);
    let a = p.subpaths[0].anchors[1];
    assert_eq!((a.p, a.h_in, a.h_out, a.kind), (Point::new(200.0, 300.0), Point::new(150.0, 300.0), Point::new(250.0, 300.0), AnchorKind::Smooth));
    assert_eq!(s.tool_id(), "pen");
    assert_eq!(s.cursor(a.h_out, command(false), view()), Cursor::ArrowHollow);
    (s, id)
}

fn assert_anchor(s: &Session, id: NodeId, first: Anchor, owner: (f64, f64), inside: (f64, f64), outside: (f64, f64)) {
    let p = path(s, id);
    assert_eq!(p.subpaths.len(), 1);
    assert!(!p.subpaths[0].closed);
    assert_eq!(p.subpaths[0].anchors.len(), 2);
    assert_eq!(p.subpaths[0].anchors[0], first, "the other anchor and its handles stay unchanged");
    let a = p.subpaths[0].anchors[1];
    assert_eq!(a.kind, AnchorKind::Smooth);
    for (label, actual, expected) in [
        ("owner", a.p, Point::new(owner.0, owner.1)),
        ("in handle", a.h_in, Point::new(inside.0, inside.1)),
        ("out handle", a.h_out, Point::new(outside.0, outside.1)),
    ] {
        assert!(actual.distance(expected) < 1e-6, "{label}: expected {expected:?}, got {actual:?}");
    }
}

/// Do not insert a zero-motion Space-on event: the first motion must use the previous pointer.
fn borrowed_space_drag(s: &mut Session, id: NodeId) {
    let first = path(s, id).subpaths[0].anchors[0];
    pointer(s, PointerKind::Down, (250.0, 300.0), command(false));
    assert_eq!(s.tool_id(), "directSelection");
    assert!(s.in_interaction());
    pointer(s, PointerKind::Drag, (260.0, 300.0), command(false));
    assert_anchor(s, id, first, (200.0, 300.0), (150.0, 300.0), (260.0, 300.0));
    pointer(s, PointerKind::Drag, (280.0, 310.0), command(true));
    assert_anchor(s, id, first, (220.0, 310.0), (170.0, 310.0), (280.0, 310.0));
    // Releasing Space while the button stays down neither jumps nor commits the gesture.
    pointer(s, PointerKind::Drag, (280.0, 310.0), command(false));
    assert_anchor(s, id, first, (220.0, 310.0), (170.0, 310.0), (280.0, 310.0));
    pointer(s, PointerKind::Drag, (300.0, 310.0), command(false));
    assert_anchor(s, id, first, (220.0, 310.0), (170.0, 310.0), (300.0, 310.0));
    pointer(s, PointerKind::Drag, (290.0, 325.0), command(true));
    assert_anchor(s, id, first, (210.0, 325.0), (160.0, 325.0), (290.0, 325.0));
    pointer(s, PointerKind::Drag, (290.0, 325.0), command(false));
    assert_anchor(s, id, first, (210.0, 325.0), (160.0, 325.0), (290.0, 325.0));
    pointer(s, PointerKind::Drag, (310.0, 325.0), command(false));
    assert_anchor(s, id, first, (210.0, 325.0), (160.0, 325.0), (310.0, 325.0));
    assert_eq!(s.tool_id(), "directSelection");
    assert!(s.in_interaction());
}

#[test]
fn pen_space_first_motion_repeated_toggle_and_pen_continuation() {
    let (mut s, id) = open_pen_path();
    let before = history(&s);
    borrowed_space_drag(&mut s, id);
    pointer(&mut s, PointerKind::Up, (310.0, 325.0), command(false));
    assert_eq!(s.tool_id(), "pen");
    assert!(!s.in_interaction());
    assert_eq!(history(&s), before + 1, "one completed handle gesture");
    let moved = path(&s, id).subpaths[0].anchors[1];
    pointer(&mut s, PointerKind::Down, (350.0, 380.0), Mods::default());
    pointer(&mut s, PointerKind::Up, (350.0, 380.0), Mods::default());
    let p = path(&s, id);
    assert_eq!(p.subpaths[0].anchors.len(), 3, "the Pen continues the same path after its loan");
    assert_eq!(p.subpaths[0].anchors[1], moved);
    assert_eq!(p.subpaths[0].anchors[2].p, Point::new(350.0, 380.0));
    assert_eq!(s.doc().unwrap().selection.objects, [id]);
    let mut count = 0;
    s.doc().unwrap().doc.walk(|node| count += usize::from(node.path_data().is_some()));
    assert_eq!(count, 1, "continuation creates no second path");
    assert_eq!(history(&s), before + 2, "the new Pen anchor is its own step");
}

#[test]
fn pen_space_no_space_handle_drag_preserves_owner() {
    let (mut s, id) = open_pen_path();
    let first = path(&s, id).subpaths[0].anchors[0];
    let before = history(&s);
    pointer(&mut s, PointerKind::Down, (250.0, 300.0), command(false));
    assert_eq!(s.tool_id(), "directSelection");
    pointer(&mut s, PointerKind::Drag, (260.0, 300.0), command(false));
    pointer(&mut s, PointerKind::Up, (260.0, 300.0), command(false));
    assert_anchor(&s, id, first, (200.0, 300.0), (150.0, 300.0), (260.0, 300.0));
    assert_eq!(s.tool_id(), "pen");
    assert!(!s.in_interaction());
    assert_eq!(history(&s), before + 1);
}

#[test]
fn pen_space_handle_drag_undo_redo_is_one_step() {
    let (mut s, id) = open_pen_path();
    let original = path(&s, id);
    let before = history(&s);
    borrowed_space_drag(&mut s, id);
    pointer(&mut s, PointerKind::Up, (310.0, 325.0), command(false));
    let changed = path(&s, id);
    assert_eq!(history(&s), before + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(path(&s, id), original);
    assert_eq!(history(&s), before);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(path(&s, id), changed);
    assert_eq!(history(&s), before + 1);
}

#[test]
fn pen_space_escape_restores_entire_borrowed_gesture() {
    let (mut s, id) = open_pen_path();
    let original = path(&s, id);
    let before = history(&s);
    borrowed_space_drag(&mut s, id);
    s.tool_key(ToolKey::Escape, command(false), view()).unwrap();
    assert_eq!(path(&s, id), original, "cancel restores pre-borrow geometry, not just the latest Space phase");
    assert_eq!(history(&s), before);
    assert!(!s.in_interaction());
    // Up releases the borrowed selection tool; Escape itself need not return the lender.
    pointer(&mut s, PointerKind::Up, (310.0, 325.0), command(false));
    assert_eq!(s.tool_id(), "pen");
    assert_eq!(path(&s, id), original);
    assert_eq!(history(&s), before);
}

#[test]
fn pen_space_type_on_path_without_space_stays_text() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let source = s
        .execute("path.create", &json!({"anchors": [{"x": 100, "y": 300, "in": [50, 300], "out": [150, 300]}, {"x": 400, "y": 300}]}))
        .unwrap()["id"]
        .clone();
    let id = NodeId(
        s.execute("text.createInPath", &json!({"path": source, "mode": "onPath", "text": "Along", "size": 12})).unwrap()["id"].as_u64().unwrap(),
    );
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.select_tool("directSelection", view()).unwrap();
    let text_path = |s: &Session| match &s.doc().unwrap().doc.node(id).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(text) => {
            assert_eq!(text.plain_text(), "Along");
            text.type_path().expect("the object remains type on a path")
        }
        kind => panic!("the object must remain text, got {kind:?}"),
    };
    let original = text_path(&s);
    let before = history(&s);
    pointer(&mut s, PointerKind::Down, (150.0, 300.0), Mods::default());
    pointer(&mut s, PointerKind::Drag, (160.0, 280.0), Mods::default());
    pointer(&mut s, PointerKind::Up, (160.0, 280.0), Mods::default());
    let changed = text_path(&s);
    let a = changed.subpaths[0].anchors[0];
    assert_eq!(a.p, original.subpaths[0].anchors[0].p);
    assert_eq!(a.h_out, Point::new(160.0, 280.0));
    assert_eq!(a.kind, AnchorKind::Smooth);
    let original_anchor = original.subpaths[0].anchors[0];
    let opposite = a.p + (a.p - a.h_out) * (original_anchor.h_in.distance(original_anchor.p) / a.h_out.distance(a.p));
    assert!(a.h_in.distance(opposite) < 1e-6, "the ordinary smooth type-path handle keeps its opposite handle's length and direction: {a:?}");
    assert_eq!(changed.subpaths[0].anchors[1], original.subpaths[0].anchors[1]);
    assert_eq!(history(&s), before + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(text_path(&s), original);
}

#[test]
fn pen_space_after_alt_drag_keeps_corner_and_opposite_handle_offset() {
    let (mut s, id) = open_pen_path();
    let first = path(&s, id).subpaths[0].anchors[0];
    let before = history(&s);
    pointer(&mut s, PointerKind::Down, (250.0, 300.0), command(false));
    let alt = Mods { alt: true, ..command(false) };
    pointer(&mut s, PointerKind::Drag, (260.0, 280.0), alt);
    let a = path(&s, id).subpaths[0].anchors[1];
    assert_eq!((a.kind, a.p, a.h_in, a.h_out), (AnchorKind::Corner, Point::new(200.0, 300.0), Point::new(150.0, 300.0), Point::new(260.0, 280.0)));
    let space = Mods { space: true, ..alt };
    pointer(&mut s, PointerKind::Drag, (280.0, 290.0), space);
    let moved = path(&s, id).subpaths[0].anchors[1];
    assert_eq!(
        (moved.kind, moved.p, moved.h_in, moved.h_out),
        (AnchorKind::Corner, Point::new(220.0, 310.0), Point::new(170.0, 310.0), Point::new(280.0, 290.0))
    );
    pointer(&mut s, PointerKind::Drag, (280.0, 290.0), alt);
    assert_eq!(path(&s, id).subpaths[0].anchors[1], moved, "releasing Space does not restore Smooth or move the independent opposite handle");
    pointer(&mut s, PointerKind::Up, (280.0, 290.0), alt);
    assert_eq!(path(&s, id).subpaths[0].anchors[0], first);
    assert_eq!(s.tool_id(), "pen");
    assert_eq!(history(&s), before + 1);
}

#[test]
fn pen_space_after_shift_uses_the_accepted_handle_without_a_release_jump() {
    let (mut s, id) = open_pen_path();
    pointer(&mut s, PointerKind::Down, (250.0, 300.0), command(false));
    let shift = Mods { shift: true, ..command(false) };
    pointer(&mut s, PointerKind::Drag, (260.0, 320.0), shift);
    let accepted = path(&s, id).subpaths[0].anchors[1];
    assert!(accepted.h_out.distance(Point::new(260.0, 320.0)) > 1.0, "Shift accepts a constrained handle, not the raw pointer");
    pointer(&mut s, PointerKind::Drag, (280.0, 330.0), Mods { space: true, ..shift });
    let delta = vectorcraft_geom::Vec2::new(20.0, 10.0);
    let moved = path(&s, id).subpaths[0].anchors[1];
    assert_eq!(moved.kind, accepted.kind);
    for (actual, expected) in [(moved.p, accepted.p + delta), (moved.h_in, accepted.h_in + delta), (moved.h_out, accepted.h_out + delta)] {
        assert!(actual.distance(expected) < 1e-6, "Space translates the accepted constrained handles: {moved:?}");
    }
    pointer(&mut s, PointerKind::Drag, (280.0, 330.0), shift);
    let released = path(&s, id).subpaths[0].anchors[1];
    assert!(
        released.p.distance(moved.p) < 1e-6 && released.h_in.distance(moved.h_in) < 1e-6 && released.h_out.distance(moved.h_out) < 1e-6,
        "same-position Space release preserves the accepted constrained handle: {moved:?} -> {released:?}"
    );
    pointer(&mut s, PointerKind::Up, (280.0, 330.0), shift);
    assert_eq!(s.tool_id(), "pen");
}

#[test]
fn pen_space_set_handle_at_keeps_corner_and_unrelated_fractional_geometry() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let id = NodeId(
        s.execute(
            "path.create",
            &json!({"anchors": [
                {"x": 100.25, "y": 100.5, "out": [130.75, 140.25]},
                {"x": 200, "y": 200, "in": [150, 200], "out": [250, 200]},
                {"x": 300.4, "y": 275.6, "in": [270.3, 280.2]}
            ]}),
        )
        .unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    // Both segments are curved; Align to Pixel Grid retains the unrelated fractional geometry.
    s.execute("object.setPixelAligned", &json!({"ids": [id.0], "on": true})).unwrap();
    s.execute("path.setHandle", &json!({"id": id.0, "anchor": 1, "which": "out", "x": 250, "y": 200, "independent": true})).unwrap();
    let original = path(&s, id);
    assert!(s.doc().unwrap().doc.node(id).unwrap().extras().pixel_aligned);
    assert_eq!(original.subpaths[0].anchors[1].kind, AnchorKind::Corner);
    assert_eq!(original.subpaths[0].anchors[0].p, Point::new(100.25, 100.5));
    let before = history(&s);
    // Omitted at preserves the existing no-op contract and the collinear Corner kind.
    s.execute("path.setHandle", &json!({"id": id.0, "anchor": 1, "which": "out", "x": 250, "y": 200})).unwrap();
    assert_eq!(path(&s, id), original);
    assert_eq!(history(&s), before);
    s.execute("path.setHandle", &json!({"id": id.0, "anchor": 1, "which": "out", "at": [210.25, 205.5], "x": 260.25, "y": 205.5})).unwrap();
    let changed = path(&s, id);
    let a = changed.subpaths[0].anchors[1];
    assert_eq!((a.kind, a.p, a.h_in, a.h_out), (AnchorKind::Corner, Point::new(210.25, 205.5), Point::new(160.25, 205.5), Point::new(260.25, 205.5)));
    assert_eq!(changed.subpaths[0].anchors[0], original.subpaths[0].anchors[0]);
    assert_eq!(changed.subpaths[0].anchors[2], original.subpaths[0].anchors[2]);
    assert!(s.doc().unwrap().doc.node(id).unwrap().extras().pixel_aligned);
    assert_eq!(history(&s), before + 1);
    let node = s.doc().unwrap().doc.node(id).unwrap().clone();
    let before_bad = history(&s);
    assert!(s.execute("path.setHandle", &json!({"id": id.0, "anchor": 1, "which": "out", "at": [220], "x": 270, "y": 210})).is_err());
    assert_eq!(s.doc().unwrap().doc.node(id).unwrap(), &node, "an invalid owner point does not apply the handle change first");
    assert_eq!(history(&s), before_bad);
}
