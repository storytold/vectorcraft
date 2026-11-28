//! Temporary Cmd/Control symmetry while Direct Selection drags an existing handle.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeId};
use vectorcraft_engine::{Session, ViewInfo};
use vectorcraft_geom::{Anchor, AnchorKind, Point};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.execute(command, &params).unwrap()
}

fn view() -> ViewInfo {
    ViewInfo {
        smart_guides: false,
        guides: false,
        snap_to_point: false,
        snap_to_grid: false,
        snap_to_pixel: false,
        snap_to_glyph: false,
        corner_widgets: false,
        show_bbox: false,
        ..ViewInfo::default()
    }
}

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 900, "height": 700}));
    for (key, value) in [("constrainAngle", json!(0)), ("showHandlesMultipleAnchors", json!(true)), ("realTimeDrawing", json!(true))] {
        run(&mut s, "prefs.set", json!({"key": key, "value": value}));
    }
    s
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn curve(s: &mut Session, incoming: Point) -> NodeId {
    let id = run(
        s,
        "path.create",
        json!({"anchors": [
            {"x": 20, "y": 220},
            {"x": 100, "y": 100, "in": [incoming.x, incoming.y], "out": [180, 100]},
            {"x": 320, "y": 220}
        ]}),
    );
    let id = NodeId(id["id"].as_u64().unwrap());
    s.select_tool("directSelection", view()).unwrap();
    id
}

fn anchor(s: &Session, id: NodeId) -> Anchor {
    node(s, id).path_data().unwrap().subpaths[0].anchors[1]
}

fn pointer(s: &mut Session, kind: PointerKind, at: Point, mods: Mods) {
    s.pointer(&PointerEvent::new(kind, at.x, at.y).with_mods(mods), view()).unwrap();
}

fn press_handle(s: &mut Session, at: Point) {
    pointer(s, PointerKind::Down, at, Mods::default());
    assert_eq!(s.tool_id(), "directSelection");
    assert!(s.in_interaction(), "the visible handle press begins Reshape");
}

fn near(actual: Point, expected: Point) {
    assert!(actual.distance(expected) < 1e-6, "point {actual:?}, expected {expected:?}");
}

fn handles(a: Anchor, incoming: Point, outgoing: Point, kind: AnchorKind) {
    near(a.h_in, incoming);
    near(a.h_out, outgoing);
    assert_eq!(a.kind, kind);
}

fn assert_other_geometry(s: &Session, id: NodeId, before: &Node) {
    let after = node(s, id);
    let (old, new) = (before.path_data().unwrap(), after.path_data().unwrap());
    assert_eq!(new.subpaths.len(), old.subpaths.len());
    assert_eq!(new.subpaths[0].closed, old.subpaths[0].closed);
    assert_eq!(new.subpaths[0].anchors.len(), 3);
    assert_eq!(new.subpaths[0].anchors[0], old.subpaths[0].anchors[0]);
    assert_eq!(new.subpaths[0].anchors[2], old.subpaths[0].anchors[2]);
    assert_eq!(new.subpaths[0].anchors[1].p, old.subpaths[0].anchors[1].p);
    assert_eq!(after.appearance, before.appearance);
}

#[test]
fn direct_handle_baseline_ordinary_smooth_keeps_the_opposite_length() {
    let mut s = session();
    let id = curve(&mut s, Point::new(50.0, 100.0));
    let before = node(&s, id);
    assert_eq!(anchor(&s, id).kind, AnchorKind::Smooth);
    let selection = json!(s.doc().unwrap().selection);
    let undo = s.doc().unwrap().history.undo.len();
    press_handle(&mut s, Point::new(180.0, 100.0));
    let q = Point::new(180.0, 160.0);
    pointer(&mut s, PointerKind::Drag, q, Mods::default());
    handles(anchor(&s, id), Point::new(60.0, 70.0), q, AnchorKind::Smooth);
    assert_other_geometry(&s, id, &before);
    pointer(&mut s, PointerKind::Up, q, Mods::default());
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(json!(s.doc().unwrap().selection), selection);
}

#[test]
fn direct_handle_baseline_cmd_added_mid_drag_mirrors_the_opposite() {
    let mut s = session();
    let id = curve(&mut s, Point::new(50.0, 100.0));
    let before = node(&s, id);
    press_handle(&mut s, Point::new(180.0, 100.0));
    let q = Point::new(180.0, 160.0);
    pointer(&mut s, PointerKind::Drag, q, Mods::default());
    handles(anchor(&s, id), Point::new(60.0, 70.0), q, AnchorKind::Smooth);
    pointer(&mut s, PointerKind::Drag, q, Mods { cmd: true, ..Mods::default() });
    handles(anchor(&s, id), Point::new(20.0, 40.0), q, AnchorKind::Smooth);
    assert_other_geometry(&s, id, &before);
    pointer(&mut s, PointerKind::Up, q, Mods { cmd: true, ..Mods::default() });
}

#[test]
fn direct_handle_cmd_mirror_holds_through_a_space_translation() {
    let mut s = session();
    let id = curve(&mut s, Point::new(50.0, 100.0));
    press_handle(&mut s, Point::new(180.0, 100.0));
    let q = Point::new(180.0, 160.0);
    pointer(&mut s, PointerKind::Drag, q, cmd());
    handles(anchor(&s, id), Point::new(20.0, 40.0), q, AnchorKind::Smooth);
    // Space moves the anchor and both handles; the opposite handle stays mirrored.
    let moved = Point::new(200.0, 190.0);
    pointer(&mut s, PointerKind::Drag, moved, Mods { space: true, ..cmd() });
    let a = anchor(&s, id);
    near(a.p, Point::new(120.0, 130.0));
    handles(a, Point::new(40.0, 70.0), Point::new(200.0, 190.0), AnchorKind::Smooth);
    // Releasing Space without moving keeps the translated, mirrored handles.
    pointer(&mut s, PointerKind::Drag, moved, cmd());
    handles(anchor(&s, id), Point::new(40.0, 70.0), Point::new(200.0, 190.0), AnchorKind::Smooth);
    let next = Point::new(220.0, 190.0);
    pointer(&mut s, PointerKind::Drag, next, cmd());
    assert_mirrored(anchor(&s, id), next, AnchorKind::Smooth);
    pointer(&mut s, PointerKind::Up, next, cmd());
}

#[test]
fn direct_handle_modifiers_changed_during_a_space_translation_apply_at_once() {
    let mut s = session();
    let id = curve(&mut s, Point::new(50.0, 100.0));
    press_handle(&mut s, Point::new(180.0, 100.0));
    pointer(&mut s, PointerKind::Drag, Point::new(180.0, 160.0), cmd());
    let moved = Point::new(200.0, 190.0);
    pointer(&mut s, PointerKind::Drag, moved, Mods { space: true, ..cmd() });
    handles(anchor(&s, id), Point::new(40.0, 70.0), moved, AnchorKind::Smooth);
    // Releasing Cmd while Space is held gives the opposite handle its own length again.
    pointer(&mut s, PointerKind::Drag, moved, Mods { space: true, ..Mods::default() });
    let a = anchor(&s, id);
    near(a.p, Point::new(120.0, 130.0));
    handles(a, Point::new(80.0, 100.0), moved, AnchorKind::Smooth);
    // Alt while Space is held leaves the opposite handle where the translation put it.
    pointer(&mut s, PointerKind::Drag, moved, Mods { space: true, alt: true, ..cmd() });
    handles(anchor(&s, id), Point::new(70.0, 130.0), moved, AnchorKind::Corner);
    pointer(&mut s, PointerKind::Up, moved, Mods { alt: true, ..cmd() });
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    document: Value,
    selection: Value,
    undo: usize,
    redo: usize,
}

fn snapshot(s: &Session) -> Snapshot {
    let st = s.doc().unwrap();
    Snapshot { document: json!(st.doc), selection: json!(st.selection), undo: st.history.undo.len(), redo: st.history.redo.len() }
}

fn assert_content(s: &Session, expected: &Snapshot) {
    let got = snapshot(s);
    assert_eq!(got.document, expected.document);
    assert_eq!(got.selection, expected.selection);
}

fn cmd() -> Mods {
    Mods { cmd: true, ..Mods::default() }
}

fn reflected(a: Point, q: Point) -> Point {
    a - (q - a)
}

fn constrained(a: Point, q: Point) -> Point {
    a + vectorcraft_geom::constrain_angle_from(q - a, 45.0, 0.0)
}

fn assert_mirrored(a: Anchor, q: Point, kind: AnchorKind) {
    handles(a, reflected(a.p, q), q, kind);
}

#[test]
fn direct_handle_stationary_cmd_release_repress_commit_and_next_gesture() {
    let mut s = session();
    let id = curve(&mut s, Point::new(50.0, 100.0));
    let before = snapshot(&s);
    press_handle(&mut s, Point::new(180.0, 100.0));
    let q = Point::new(180.0, 160.0);
    for (mods, opposite) in [(cmd(), Point::new(20.0, 40.0)), (Mods::default(), Point::new(60.0, 70.0)), (cmd(), Point::new(20.0, 40.0))] {
        pointer(&mut s, PointerKind::Drag, q, mods);
        handles(anchor(&s, id), opposite, q, AnchorKind::Smooth);
        assert_eq!(s.doc().unwrap().history.undo.len(), before.undo, "previews are not separate edits");
    }
    pointer(&mut s, PointerKind::Up, q, cmd());
    let committed = snapshot(&s);
    assert_eq!(committed.undo, before.undo + 1);
    assert_eq!(committed.selection, before.selection);
    run(&mut s, "edit.undo", json!({}));
    assert_content(&s, &before);
    run(&mut s, "edit.redo", json!({}));
    assert_content(&s, &committed);
    // Cmd did not install a persistent coupling mode: a later ordinary Smooth drag keeps
    // the opposite handle's committed length, rather than matching the newly dragged length.
    press_handle(&mut s, q);
    let next = Point::new(180.0, 200.0);
    pointer(&mut s, PointerKind::Drag, next, Mods::default());
    let a = anchor(&s, id);
    near(a.h_out, next);
    assert_eq!(a.kind, AnchorKind::Smooth);
    assert!((a.h_in.distance(a.p) - 100.0).abs() < 1e-6);
    assert!((a.h_in.distance(a.p) - a.h_out.distance(a.p)).abs() > 20.0);
    assert!(((a.h_in - a.p).cross(a.h_out - a.p)).abs() < 1e-6);
    pointer(&mut s, PointerKind::Up, next, Mods::default());
    assert_eq!(s.doc().unwrap().history.undo.len(), before.undo + 2);
}

#[test]
fn direct_handle_shift_constrains_before_mirroring_and_alt_takes_priority() {
    let mut s = session();
    let id = curve(&mut s, Point::new(50.0, 100.0));
    let p = anchor(&s, id).p;
    press_handle(&mut s, Point::new(180.0, 100.0));
    let raw = Point::new(180.0, 150.0);
    let q = constrained(p, raw);
    let shifted = Mods { shift: true, ..cmd() };
    pointer(&mut s, PointerKind::Drag, raw, shifted);
    assert_mirrored(anchor(&s, id), q, AnchorKind::Smooth);
    pointer(&mut s, PointerKind::Drag, raw, Mods { alt: true, ..shifted });
    handles(anchor(&s, id), Point::new(50.0, 100.0), q, AnchorKind::Corner);
    // Removing Alt rebuilds from the original Smooth anchor, not the Corner preview.
    pointer(&mut s, PointerKind::Drag, raw, shifted);
    assert_mirrored(anchor(&s, id), q, AnchorKind::Smooth);
    pointer(&mut s, PointerKind::Drag, raw, Mods { alt: true, ..cmd() });
    handles(anchor(&s, id), Point::new(50.0, 100.0), raw, AnchorKind::Corner);
    pointer(&mut s, PointerKind::Up, raw, Mods { alt: true, ..cmd() });
}

#[test]
fn direct_handle_corner_with_collapsed_partner_is_reversible_without_changing_kind() {
    let mut s = session();
    let id = curve(&mut s, Point::new(100.0, 100.0));
    let before = snapshot(&s);
    assert_eq!(anchor(&s, id).kind, AnchorKind::Corner);
    press_handle(&mut s, Point::new(180.0, 100.0));
    let q = Point::new(180.0, 160.0);
    pointer(&mut s, PointerKind::Drag, q, cmd());
    assert_mirrored(anchor(&s, id), q, AnchorKind::Corner);
    pointer(&mut s, PointerKind::Drag, q, Mods::default());
    handles(anchor(&s, id), Point::new(100.0, 100.0), q, AnchorKind::Corner);
    pointer(&mut s, PointerKind::Drag, q, cmd());
    assert_mirrored(anchor(&s, id), q, AnchorKind::Corner);
    s.tool_key(vectorcraft_tools::ToolKey::Escape, Mods::default(), view()).unwrap();
    assert_eq!(snapshot(&s), before);
}

#[test]
fn direct_handle_stationary_drag_still_mirrors_unequal_partner_but_equal_is_noop() {
    for (incoming, changes) in [(Point::new(50.0, 100.0), true), (Point::new(20.0, 100.0), false)] {
        let mut s = session();
        let id = curve(&mut s, incoming);
        let before = snapshot(&s);
        let q = Point::new(180.0, 100.0);
        press_handle(&mut s, q);
        pointer(&mut s, PointerKind::Drag, q, cmd());
        assert_mirrored(anchor(&s, id), q, AnchorKind::Smooth);
        pointer(&mut s, PointerKind::Up, q, cmd());
        if changes {
            assert_eq!(s.doc().unwrap().history.undo.len(), before.undo + 1);
            run(&mut s, "edit.undo", json!({}));
            assert_content(&s, &before);
        } else {
            assert_eq!(snapshot(&s), before, "an already reflected pair records no edit");
        }
    }
}

#[test]
fn direct_handle_escape_restores_document_selection_and_existing_redo() {
    let mut s = session();
    let id = curve(&mut s, Point::new(50.0, 100.0));
    run(&mut s, "shape.rectangle", json!({"x": 500, "y": 400, "width": 25, "height": 30}));
    let redo_document = snapshot(&s).document;
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let before = snapshot(&s);
    assert_eq!(before.redo, 1, "a real existing Redo is the cancellation control");
    press_handle(&mut s, Point::new(180.0, 100.0));
    let q = Point::new(180.0, 160.0);
    pointer(&mut s, PointerKind::Drag, q, cmd());
    assert_mirrored(anchor(&s, id), q, AnchorKind::Smooth);
    s.tool_key(vectorcraft_tools::ToolKey::Escape, Mods::default(), view()).unwrap();
    assert!(!s.in_interaction());
    assert_eq!(snapshot(&s), before);
    pointer(&mut s, PointerKind::Up, q, Mods::default());
    assert_eq!(snapshot(&s), before, "a later release does not commit the canceled preview");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(snapshot(&s).document, redo_document, "the prior Redo still restores its actual content");
}

#[test]
fn direct_handle_cmd_before_press_still_lends_selection_and_returns_direct() {
    let mut s = session();
    let id = curve(&mut s, Point::new(50.0, 100.0));
    let before = snapshot(&s);
    let a = anchor(&s, id);
    pointer(&mut s, PointerKind::Down, a.p, cmd());
    assert_eq!(s.tool_id(), "selection", "Cmd before Down keeps the existing Selection loan");
    pointer(&mut s, PointerKind::Up, a.p, cmd());
    assert_eq!(s.tool_id(), "directSelection");
    assert_eq!(anchor(&s, id), a);
    assert_eq!(snapshot(&s).document, before.document);
    assert_eq!(s.doc().unwrap().history.undo.len(), before.undo);
}

#[test]
fn direct_handle_cmd_that_lends_direct_selection_from_the_pen_does_not_mirror() {
    let mut s = session();
    let id = curve(&mut s, Point::new(50.0, 100.0));
    s.select_tool("pen", view()).unwrap();
    let before = node(&s, id);
    let undo = s.doc().unwrap().history.undo.len();
    pointer(&mut s, PointerKind::Down, Point::new(180.0, 100.0), cmd());
    assert_eq!(s.tool_id(), "directSelection", "Cmd lends the Pen the last selection tool");
    let q = Point::new(180.0, 160.0);
    pointer(&mut s, PointerKind::Drag, q, cmd());
    handles(anchor(&s, id), Point::new(60.0, 70.0), q, AnchorKind::Smooth);
    // Cmd let go and pressed again during the drag mirrors, as it does in Direct Selection itself.
    pointer(&mut s, PointerKind::Drag, q, Mods::default());
    handles(anchor(&s, id), Point::new(60.0, 70.0), q, AnchorKind::Smooth);
    pointer(&mut s, PointerKind::Drag, q, cmd());
    handles(anchor(&s, id), Point::new(20.0, 40.0), q, AnchorKind::Smooth);
    assert_other_geometry(&s, id, &before);
    pointer(&mut s, PointerKind::Up, q, cmd());
    assert_eq!(s.tool_id(), "pen");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
}

#[test]
fn direct_handle_cmd_held_through_a_pen_lent_drag_leaves_the_opposite_length() {
    let mut s = session();
    let id = curve(&mut s, Point::new(50.0, 100.0));
    s.select_tool("pen", view()).unwrap();
    pointer(&mut s, PointerKind::Down, Point::new(180.0, 100.0), cmd());
    let q = Point::new(180.0, 160.0);
    pointer(&mut s, PointerKind::Drag, q, cmd());
    pointer(&mut s, PointerKind::Up, q, cmd());
    assert_eq!(s.tool_id(), "pen");
    handles(anchor(&s, id), Point::new(60.0, 70.0), q, AnchorKind::Smooth);
}

fn text_anchor(s: &Session, id: NodeId) -> Anchor {
    let n = node(s, id);
    let vectorcraft_doc::NodeKind::Text(t) = n.kind else { panic!("text on a path") };
    t.type_path().unwrap().subpaths[0].anchors[1]
}

fn text_properties(s: &Session, id: NodeId) -> Value {
    let n = node(s, id);
    let vectorcraft_doc::NodeKind::Text(t) = n.kind else { panic!("text on a path") };
    let vectorcraft_doc::TextKind::OnPath { start, end, .. } = &t.kind else { panic!("path text") };
    json!({"runs": t.runs, "para": t.para, "paras": t.paras, "xf": t.xf, "start": start, "end": end,
        "effect": t.path_effect, "align": t.path_align, "spacing": t.path_spacing, "appearance": n.appearance})
}

fn path_text(s: &mut Session) -> NodeId {
    let path = curve(s, Point::new(50.0, 100.0));
    let made = run(
        s,
        "text.createInPath",
        json!({"path": path.0, "mode": "onPath", "text": "Along the original curve", "font": "Source Sans 3", "size": 12}),
    );
    let id = NodeId(made["id"].as_u64().unwrap());
    run(s, "type.pathOptions", json!({"id": id.0, "start": 0.15, "end": 0.85}));
    run(s, "object.transform", json!({"ids": [id.0], "matrix": [1.5, 0.4, 0.25, 0.9, 250, 80]}));
    s.select_tool("directSelection", view()).unwrap();
    id
}

#[test]
fn direct_handle_affine_path_text_mirrors_in_drawn_space_and_keeps_brackets() {
    let mut s = session();
    let id = path_text(&mut s);
    let before = snapshot(&s);
    let properties = text_properties(&s, id);
    assert_eq!(properties["start"], json!(0.15));
    assert_eq!(properties["end"], json!(0.85));
    let a = text_anchor(&s, id);
    assert_eq!(a.kind, AnchorKind::Smooth);
    near(a.p, Point::new(425.0, 210.0));
    near(a.h_out, Point::new(545.0, 242.0));
    assert!((a.h_in.distance(a.p) - a.h_out.distance(a.p)).abs() > 40.0);
    press_handle(&mut s, a.h_out);
    let raw = a.p + vectorcraft_geom::Vec2::new(80.0, 60.0);
    pointer(&mut s, PointerKind::Drag, raw, cmd());
    assert_mirrored(text_anchor(&s, id), raw, AnchorKind::Smooth);
    assert_eq!(text_properties(&s, id), properties);
    let shifted = Mods { shift: true, ..cmd() };
    pointer(&mut s, PointerKind::Drag, raw, shifted);
    assert_mirrored(text_anchor(&s, id), constrained(a.p, raw), AnchorKind::Smooth);
    assert_eq!(text_properties(&s, id), properties);
    pointer(&mut s, PointerKind::Up, raw, shifted);
    let after = snapshot(&s);
    assert_eq!(after.undo, before.undo + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_content(&s, &before);
    run(&mut s, "edit.redo", json!({}));
    assert_content(&s, &after);
}

fn blend_state(s: &Session, id: NodeId) -> (Anchor, Value, Vec<u32>) {
    let n = node(s, id);
    let vectorcraft_doc::NodeKind::Blend { children, spec } = n.kind else { panic!("blend") };
    assert!(spec.spine.is_some(), "the fixture uses an already pinned spine");
    let path = vectorcraft_doc::live::blend_spine(&children, &spec).unwrap().0;
    (path.subpaths[0].anchors[1], json!(children), spec.key_anchors.clone())
}

fn blend(s: &mut Session) -> NodeId {
    let a = run(s, "shape.rectangle", json!({"x": 15, "y": 95, "width": 10, "height": 10}))["id"].as_u64().unwrap();
    let b = run(s, "shape.rectangle", json!({"x": 295, "y": 95, "width": 10, "height": 10}))["id"].as_u64().unwrap();
    let made = run(s, "object.blend.make", json!({"ids": [a, b], "steps": 2}));
    let id = NodeId(made["id"].as_u64().unwrap());
    let added = run(s, "object.blend.spine.addAnchor", json!({"id": id.0, "x": 100, "y": 100}));
    assert_eq!(added["anchor"], json!(1));
    run(s, "object.blend.spine.convertAnchor", json!({"id": id.0, "anchor": 1, "to": "smooth", "x": 180, "y": 100}));
    run(s, "object.blend.spine.moveAnchor", json!({"id": id.0, "anchor": 1, "handle": "in", "x": 50, "y": 100}));
    run(s, "select.set", json!({"ids": [id.0]}));
    s.select_tool("directSelection", view()).unwrap();
    let (middle, _, keys) = blend_state(s, id);
    near(middle.p, Point::new(100.0, 100.0));
    handles(middle, Point::new(50.0, 100.0), Point::new(180.0, 100.0), AnchorKind::Smooth);
    assert_eq!(keys, [0, 2]);
    // An actual spine-point click makes its handles visible to Direct Selection.
    pointer(s, PointerKind::Down, middle.p, Mods::default());
    pointer(s, PointerKind::Up, middle.p, Mods::default());
    assert!(!s.in_interaction());
    id
}

#[test]
fn direct_handle_pinned_blend_spine_cmd_shift_preserves_keys_and_mapping() {
    let mut s = session();
    let id = blend(&mut s);
    let before = snapshot(&s);
    let (a, keys, mapping) = blend_state(&s, id);
    press_handle(&mut s, a.h_out);
    let q = Point::new(180.0, 160.0);
    pointer(&mut s, PointerKind::Drag, q, cmd());
    assert_mirrored(blend_state(&s, id).0, q, AnchorKind::Smooth);
    let raw = Point::new(180.0, 150.0);
    let shifted = Mods { shift: true, ..cmd() };
    pointer(&mut s, PointerKind::Drag, raw, shifted);
    assert_mirrored(blend_state(&s, id).0, constrained(a.p, raw), AnchorKind::Smooth);
    // The existing spine Shift-only path is deliberately unconstrained; Cmd's new rule
    // must not leak into that old branch after the modifier is released.
    pointer(&mut s, PointerKind::Drag, raw, Mods { shift: true, ..Mods::default() });
    near(blend_state(&s, id).0.h_out, raw);
    pointer(&mut s, PointerKind::Drag, raw, shifted);
    let (after_anchor, after_keys, after_mapping) = blend_state(&s, id);
    assert_mirrored(after_anchor, constrained(a.p, raw), AnchorKind::Smooth);
    assert_eq!(after_keys, keys, "handle edits leave the actual key art unchanged");
    assert_eq!(after_mapping, mapping);
    pointer(&mut s, PointerKind::Up, raw, shifted);
    let after = snapshot(&s);
    assert_eq!(after.undo, before.undo + 1);
    assert_eq!(after.selection, before.selection);
    run(&mut s, "edit.undo", json!({}));
    assert_content(&s, &before);
    run(&mut s, "edit.redo", json!({}));
    assert_content(&s, &after);
}

#[test]
fn direct_handle_incoming_drag_mirrors_outgoing_and_release_restores_ordinary_length() {
    let mut s = session();
    let id = curve(&mut s, Point::new(50.0, 100.0));
    let before = snapshot(&s);
    let original = node(&s, id);
    press_handle(&mut s, Point::new(50.0, 100.0));
    let q = Point::new(40.0, 20.0);
    pointer(&mut s, PointerKind::Drag, q, Mods::default());
    handles(anchor(&s, id), q, Point::new(148.0, 164.0), AnchorKind::Smooth);
    pointer(&mut s, PointerKind::Drag, q, cmd());
    handles(anchor(&s, id), q, Point::new(160.0, 180.0), AnchorKind::Smooth);
    pointer(&mut s, PointerKind::Drag, q, Mods::default());
    handles(anchor(&s, id), q, Point::new(148.0, 164.0), AnchorKind::Smooth);
    pointer(&mut s, PointerKind::Drag, q, cmd());
    assert_other_geometry(&s, id, &original);
    pointer(&mut s, PointerKind::Up, q, cmd());
    handles(anchor(&s, id), q, Point::new(160.0, 180.0), AnchorKind::Smooth);
    assert_eq!(s.doc().unwrap().history.undo.len(), before.undo + 1);
    assert_eq!(snapshot(&s).selection, before.selection);
    run(&mut s, "edit.undo", json!({}));
    assert_content(&s, &before);
}

/// `command` with `params` is refused with an error containing `message`, and the document,
/// selection and undo history stay as they were.
fn refused(s: &mut Session, command: &str, params: Value, message: &str) {
    let before = snapshot(s);
    let err = s.execute(command, &params).unwrap_err().to_string();
    assert!(err.contains(message), "{command} {params}: {err}");
    assert_eq!(snapshot(s), before);
}

#[test]
fn path_set_handle_symmetric_mirrors_refuses_bad_values_and_records_no_step_when_in_place() {
    let mut s = session();
    let id = curve(&mut s, Point::new(50.0, 100.0));
    let at = |x: f64, y: f64| json!({"id": id.0, "anchor": 1, "which": "out", "x": x, "y": y});
    for bad in [json!("yes"), json!(1)] {
        let mut p = at(180.0, 160.0);
        p["symmetric"] = bad;
        refused(&mut s, "path.setHandle", p, "`symmetric` must be true or false");
    }
    let mut both = at(180.0, 160.0);
    both["symmetric"] = json!(true);
    both["independent"] = json!(true);
    refused(&mut s, "path.setHandle", both, "`independent` and `symmetric` cannot both be true");

    let undo = s.doc().unwrap().history.undo.len();
    let mut mirror = at(180.0, 160.0);
    mirror["symmetric"] = json!(true);
    run(&mut s, "path.setHandle", mirror.clone());
    handles(anchor(&s, id), Point::new(20.0, 40.0), Point::new(180.0, 160.0), AnchorKind::Smooth);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    let placed = snapshot(&s);
    run(&mut s, "path.setHandle", mirror);
    assert_eq!(snapshot(&s), placed, "both handles already in place record no undo step");
}

#[test]
fn symmetric_leaves_the_other_handle_retracted_at_an_open_end() {
    let mut s = session();
    let id = curve(&mut s, Point::new(50.0, 100.0));
    let first = |s: &Session| node(s, id).path_data().unwrap().subpaths[0].anchors[0];
    let p = first(&s).p;
    run(&mut s, "path.setHandle", json!({"id": id.0, "anchor": 0, "which": "out", "x": 60, "y": 180, "symmetric": true}));
    handles(first(&s), p, Point::new(60.0, 180.0), AnchorKind::Corner);

    let mut s = session();
    let blend_id = blend(&mut s);
    let end = |s: &Session| {
        let n = node(s, blend_id);
        let vectorcraft_doc::NodeKind::Blend { children, spec } = n.kind else { panic!("blend") };
        vectorcraft_doc::live::blend_spine(&children, &spec).unwrap().0.subpaths[0].anchors[0]
    };
    let start = end(&s);
    run(&mut s, "object.blend.spine.moveAnchor", json!({"id": blend_id.0, "anchor": 0, "handle": "out", "x": 60, "y": 60, "symmetric": true}));
    near(end(&s).h_out, Point::new(60.0, 60.0));
    near(end(&s).h_in, start.h_in);
    near(end(&s).h_in, start.p);
}

#[test]
fn spine_and_type_path_handles_refuse_bad_symmetric_values() {
    let mut s = session();
    let id = blend(&mut s);
    let spine = |extra: Value| {
        let mut p = json!({"id": id.0, "anchor": 1, "handle": "out", "x": 180, "y": 160});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        p
    };
    refused(&mut s, "object.blend.spine.moveAnchor", spine(json!({"symmetric": "yes"})), "`symmetric` must be true or false");
    refused(
        &mut s,
        "object.blend.spine.moveAnchor",
        spine(json!({"symmetric": true, "independent": true})),
        "`independent` and `symmetric` cannot both be true",
    );
    refused(
        &mut s,
        "object.blend.spine.moveAnchor",
        json!({"id": id.0, "anchor": 1, "x": 180, "y": 160, "symmetric": true}),
        "`symmetric` places a handle",
    );

    let text = path_text(&mut s);
    let a = text_anchor(&s, text);
    let at = json!({"id": text.0, "anchor": 1, "which": "out", "x": a.h_out.x, "y": a.h_out.y + 20.0});
    let mut bad = at.clone();
    bad["symmetric"] = json!(0);
    refused(&mut s, "text.setPathHandle", bad, "`symmetric` must be true or false");
    let mut both = at;
    both["symmetric"] = json!(true);
    both["independent"] = json!(true);
    refused(&mut s, "text.setPathHandle", both, "`independent` and `symmetric` cannot both be true");
}

#[test]
fn direct_handle_cmd_that_lends_direct_selection_from_the_pen_does_not_mirror_type_on_a_path() {
    let mut s = session();
    let id = path_text(&mut s);
    let a = text_anchor(&s, id);
    let length = a.h_in.distance(a.p);
    s.select_tool("pen", view()).unwrap();
    pointer(&mut s, PointerKind::Down, a.h_out, cmd());
    assert_eq!(s.tool_id(), "directSelection");
    let q = a.p + vectorcraft_geom::Vec2::new(80.0, 60.0);
    pointer(&mut s, PointerKind::Drag, q, cmd());
    let held = text_anchor(&s, id);
    near(held.h_out, q);
    assert!((held.h_in.distance(held.p) - length).abs() < 1e-6, "the other handle keeps its length");
    pointer(&mut s, PointerKind::Drag, q, Mods::default());
    pointer(&mut s, PointerKind::Drag, q, cmd());
    assert_mirrored(text_anchor(&s, id), q, AnchorKind::Smooth);
    pointer(&mut s, PointerKind::Up, q, cmd());
    assert_eq!(s.tool_id(), "pen");
}
