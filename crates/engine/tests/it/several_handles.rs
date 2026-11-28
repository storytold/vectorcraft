//! Direct Selection picks several direction handles and drags them together (`path.moveHandles`).

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_engine::{Session, ViewInfo};
use vectorcraft_geom::{Anchor, AnchorKind, Point};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

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

/// A path selected whole with Direct Selection chosen: anchor 1 is smooth, its incoming handle at
/// (50, 100) and its outgoing one at (180, 100); anchor 2's incoming handle is at (300, 140).
fn session() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 900, "height": 700})).unwrap();
    for (key, value) in [("constrainAngle", json!(0)), ("showHandlesMultipleAnchors", json!(true))] {
        s.execute("prefs.set", &json!({"key": key, "value": value})).unwrap();
    }
    let id = s
        .execute(
            "path.create",
            &json!({"anchors": [
                {"x": 20, "y": 220},
                {"x": 100, "y": 100, "in": [50, 100], "out": [180, 100]},
                {"x": 320, "y": 220, "in": [300, 140]}
            ]}),
        )
        .unwrap();
    let id = NodeId(id["id"].as_u64().unwrap());
    s.select_tool("directSelection", view()).unwrap();
    (s, id)
}

fn anchor(s: &Session, id: NodeId, i: usize) -> Anchor {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0].anchors[i]
}

fn pointer(s: &mut Session, kind: PointerKind, at: Point, mods: Mods) {
    s.pointer(&PointerEvent::new(kind, at.x, at.y).with_mods(mods), view()).unwrap();
}

fn click(s: &mut Session, at: Point, mods: Mods) {
    pointer(s, PointerKind::Down, at, mods);
    pointer(s, PointerKind::Up, at, mods);
}

fn drag(s: &mut Session, from: Point, to: Point, mods: Mods) {
    pointer(s, PointerKind::Down, from, mods);
    pointer(s, PointerKind::Drag, Point::new((from.x + to.x) / 2.0, (from.y + to.y) / 2.0), mods);
    pointer(s, PointerKind::Drag, to, mods);
    pointer(s, PointerKind::Up, to, mods);
}

fn shift() -> Mods {
    Mods { shift: true, ..Mods::default() }
}

fn near(actual: Point, expected: Point) {
    assert!(actual.distance(expected) < 1e-6, "point {actual:?}, expected {expected:?}");
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn document(s: &Session) -> Value {
    json!(s.doc().unwrap().doc)
}

/// A click picks a handle and a Shift-click adds another without moving it; a drag of either then
/// moves both by the same offset, and a smooth anchor turns its other handle in line, as one undo
/// step that Undo takes back whole.
#[test]
fn a_drag_of_one_of_two_picked_handles_moves_both_in_one_undo_step() {
    let (mut s, id) = session();
    let before = document(&s);
    click(&mut s, Point::new(180.0, 100.0), Mods::default());
    // A Shift-click with a small slip of the pointer picks the handle and leaves it where it is.
    pointer(&mut s, PointerKind::Down, Point::new(300.0, 140.0), shift());
    pointer(&mut s, PointerKind::Drag, Point::new(301.0, 140.0), shift());
    pointer(&mut s, PointerKind::Up, Point::new(301.0, 140.0), shift());
    near(anchor(&s, id, 2).h_in, Point::new(300.0, 140.0));
    let undo = undo_len(&s);
    drag(&mut s, Point::new(180.0, 100.0), Point::new(180.0, 130.0), Mods::default());
    let a = anchor(&s, id, 1);
    near(a.h_out, Point::new(180.0, 130.0));
    assert_eq!(a.kind, AnchorKind::Smooth);
    // The incoming handle keeps its length (50) and turns in line with the moved one.
    let dir = (a.p - a.h_out) / (a.p - a.h_out).hypot();
    near(a.h_in, a.p + dir * 50.0);
    near(anchor(&s, id, 2).h_in, Point::new(300.0, 170.0));
    assert_eq!(undo_len(&s), undo + 1, "one undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(document(&s), before);
}

/// The picked handles stay picked after their drag, and a second drag of the other one moves both
/// again.
#[test]
fn picked_handles_stay_picked_after_they_are_dragged() {
    let (mut s, id) = session();
    click(&mut s, Point::new(180.0, 100.0), Mods::default());
    click(&mut s, Point::new(300.0, 140.0), shift());
    drag(&mut s, Point::new(180.0, 100.0), Point::new(180.0, 130.0), Mods::default());
    drag(&mut s, Point::new(300.0, 170.0), Point::new(310.0, 170.0), Mods::default());
    near(anchor(&s, id, 1).h_out, Point::new(190.0, 130.0));
    near(anchor(&s, id, 2).h_in, Point::new(310.0, 170.0));
}

/// A Shift-click on a picked handle takes it away, and Escape lets every picked handle go: a drag
/// then moves only the handle pressed.
#[test]
fn shift_click_and_escape_let_picked_handles_go() {
    for escape in [false, true] {
        let (mut s, id) = session();
        click(&mut s, Point::new(180.0, 100.0), Mods::default());
        click(&mut s, Point::new(300.0, 140.0), shift());
        if escape {
            s.tool_key(ToolKey::Escape, Mods::default(), view()).unwrap();
        } else {
            click(&mut s, Point::new(180.0, 100.0), shift());
        }
        drag(&mut s, Point::new(300.0, 140.0), Point::new(300.0, 170.0), Mods::default());
        near(anchor(&s, id, 2).h_in, Point::new(300.0, 170.0));
        near(anchor(&s, id, 1).h_out, Point::new(180.0, 100.0));
    }
}

/// Both handles of a smooth anchor picked move together and the anchor becomes a corner; with Alt,
/// each picked handle's anchor becomes a corner and keeps its other handle.
#[test]
fn both_handles_of_an_anchor_or_alt_make_a_corner() {
    let (mut s, id) = session();
    click(&mut s, Point::new(180.0, 100.0), Mods::default());
    click(&mut s, Point::new(50.0, 100.0), shift());
    drag(&mut s, Point::new(180.0, 100.0), Point::new(180.0, 120.0), Mods::default());
    let a = anchor(&s, id, 1);
    near(a.h_out, Point::new(180.0, 120.0));
    near(a.h_in, Point::new(50.0, 120.0));
    assert_eq!(a.kind, AnchorKind::Corner);

    let (mut s, id) = session();
    let alt = Mods { alt: true, ..Mods::default() };
    click(&mut s, Point::new(180.0, 100.0), Mods::default());
    click(&mut s, Point::new(300.0, 140.0), shift());
    drag(&mut s, Point::new(180.0, 100.0), Point::new(180.0, 130.0), alt);
    let a = anchor(&s, id, 1);
    near(a.h_out, Point::new(180.0, 130.0));
    near(a.h_in, Point::new(50.0, 100.0));
    assert_eq!(a.kind, AnchorKind::Corner);
    near(anchor(&s, id, 2).h_in, Point::new(300.0, 170.0));
}

/// `path.moveHandles` refuses a list it cannot act on, moves a handle listed twice once, and
/// records no undo step for a move by nothing.
#[test]
fn move_handles_checks_its_params() {
    let (mut s, id) = session();
    let h = |anchor: u64, which: &str| json!({"id": id.0, "anchor": anchor, "which": which});
    let before = document(&s);
    let undo = undo_len(&s);
    for (params, message) in [
        (json!({"dx": 1, "dy": 1}), "missing handles"),
        (json!({"handles": [], "dx": 1, "dy": 1}), "`handles` is empty"),
        (json!({"handles": {}, "dx": 1, "dy": 1}), "`handles` must be an array"),
        (json!({"handles": [h(1, "out")], "dy": 1}), "missing number `dx`"),
        (json!({"handles": [h(1, "out")], "dx": 1, "dy": 1, "independent": "yes"}), "`independent` must be true or false"),
        (json!({"handles": [{"id": id.0, "anchor": 1}], "dx": 1, "dy": 1}), "`which` must be \"in\" or \"out\""),
        (json!({"handles": [h(1, "out"), h(9, "in")], "dx": 1, "dy": 1}), "has no anchor 9 in subpath 0"),
        (json!({"handles": [{"anchor": 1, "which": "in"}], "dx": 1, "dy": 1}), "missing id"),
    ] {
        let err = s.execute("path.moveHandles", &params).unwrap_err().to_string();
        assert!(err.contains(message), "{params}: {err}");
    }
    s.execute("path.moveHandles", &json!({"handles": [h(1, "out")], "dx": 0, "dy": 0})).unwrap();
    assert_eq!(document(&s), before);
    assert_eq!(undo_len(&s), undo, "no undo step for a refused call or a move by nothing");
    s.execute("path.moveHandles", &json!({"handles": [h(2, "in"), h(2, "in")], "dx": 5, "dy": 0})).unwrap();
    near(anchor(&s, id, 2).h_in, Point::new(305.0, 140.0));
    assert_eq!(undo_len(&s), undo + 1);
}
