//! A command run from outside the tool (a panel, a menu item, an automation client) while a tool's drag is
//! in progress is a step of its own, before the drag's: the drag's next preview, its release and
//! Escape keep the command's edit, and the release still draws what the drag drew, with Real-time
//! Drawing on and off.

use serde_json::json;
use vectorcraft_geom::Rect;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::*;

/// The view the gestures run in: page coordinates, with Smart Guides off so nothing snaps.
fn view() -> ViewInfo {
    ViewInfo { smart_guides: false, ..ViewInfo::default() }
}

/// Feed the pointer event `(kind, x, y)` to the active tool.
fn pointer(s: &mut Session, kind: PointerKind, x: f64, y: f64) {
    s.pointer(&PointerEvent::new(kind, x, y), view()).unwrap();
}

/// A new 400 × 400 pt document with Real-time Drawing `real_time`, holding one 20 × 20 pt rectangle
/// (returned), nothing selected, and the Rectangle tool chosen.
fn session(real_time: bool) -> (Session, NodeId) {
    let mut s = Session::new();
    s.prefs.real_time_drawing = real_time;
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let other = s.doc().unwrap().selection.objects[0];
    s.execute("select.none", &json!({})).unwrap();
    s.select_tool("rectangle", view()).unwrap();
    (s, other)
}

fn visible(s: &Session, id: NodeId) -> bool {
    s.doc().unwrap().doc.node(id).expect("the object is in the document").visible
}

/// The objects of the document's first layer.
fn objects(s: &Session) -> usize {
    s.doc().unwrap().doc.layers[0].children().map_or(0, Vec::len)
}

/// The commands journaled from entry `start` on.
fn journaled(s: &Session, start: usize) -> Vec<String> {
    s.journal.iter().skip(start).map(|(id, _)| id.clone()).collect()
}

#[test]
fn a_command_during_a_drag_is_a_step_of_its_own_before_the_drag() {
    for real_time in [true, false] {
        let (mut s, other) = session(real_time);
        let (steps, start) = (s.doc().unwrap().history.undo.len(), s.journal.len());
        pointer(&mut s, PointerKind::Down, 100.0, 100.0);
        pointer(&mut s, PointerKind::Drag, 150.0, 150.0);
        s.execute("object.hide", &json!({"ids": [other.0]})).unwrap();
        assert!(!visible(&s, other), "real time {real_time}: the command hides the object at once");
        pointer(&mut s, PointerKind::Drag, 200.0, 180.0);
        assert!(!visible(&s, other), "real time {real_time}: the drag's next preview keeps the command's edit");
        pointer(&mut s, PointerKind::Up, 200.0, 180.0);
        assert!(!visible(&s, other), "real time {real_time}: the release keeps the command's edit");
        assert_eq!(objects(&s), 2, "real time {real_time}: the release draws the rectangle");
        let drawn = s.doc().unwrap().selection.objects[0];
        assert_ne!(drawn, other);
        assert_eq!(s.doc().unwrap().doc.bounds_of(&[drawn], false), Some(Rect::new(100.0, 100.0, 200.0, 180.0)), "real time {real_time}");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps + 2, "real time {real_time}: the command and the drag are a step each");
        assert_eq!(journaled(&s, start), ["object.hide", "shape.rectangle"], "real time {real_time}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(objects(&s), 1, "real time {real_time}: Undo takes back the drawn rectangle");
        assert!(!visible(&s, other), "real time {real_time}: and leaves the command's edit");
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(visible(&s, other), "real time {real_time}: the next Undo takes back the command's edit");
    }
}

#[test]
fn escape_after_a_command_during_a_drag_keeps_the_command() {
    for real_time in [true, false] {
        let (mut s, other) = session(real_time);
        let steps = s.doc().unwrap().history.undo.len();
        pointer(&mut s, PointerKind::Down, 100.0, 100.0);
        pointer(&mut s, PointerKind::Drag, 150.0, 150.0);
        s.execute("object.hide", &json!({"ids": [other.0]})).unwrap();
        s.tool_key(ToolKey::Escape, Mods::default(), view()).unwrap();
        pointer(&mut s, PointerKind::Up, 150.0, 150.0);
        assert!(!visible(&s, other), "real time {real_time}: Escape takes back only the drag");
        assert_eq!(objects(&s), 1, "real time {real_time}: nothing is drawn");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "real time {real_time}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(visible(&s, other), "real time {real_time}");
    }
}

/// Every object of the first layer but `other`, as the document has it.
fn others(s: &Session, other: NodeId) -> Vec<vectorcraft_doc::Node> {
    let d = &s.doc().unwrap().doc;
    d.layers[0].children().map_or_else(Vec::new, |c| c.iter().filter(|n| n.id != other).map(|n| (**n).clone()).collect())
}

/// A new 400 × 400 pt document with Real-time Drawing `real_time`, holding a 40 × 40 pt square at
/// (100, 100) and a 20 × 20 pt one at (10, 10) → the session, the larger square (selected) and the
/// smaller one.
fn two_squares(real_time: bool) -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    s.prefs.real_time_drawing = real_time;
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let other = s.doc().unwrap().selection.objects[0];
    s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 40, "height": 40})).unwrap();
    let target = s.doc().unwrap().selection.objects[0];
    (s, target, other)
}

/// A press at the first point of `pts`, drags to the others and a release at the last, with
/// `mid` run after the first drag.
fn drag_with(s: &mut Session, pts: &[(f64, f64)], mid: impl FnOnce(&mut Session)) {
    let (&(x0, y0), rest) = pts.split_first().expect("a press");
    pointer(s, PointerKind::Down, x0, y0);
    let mut mid = Some(mid);
    for &(x, y) in rest {
        pointer(s, PointerKind::Drag, x, y);
        if let Some(f) = mid.take() {
            f(s);
        }
    }
    let &(x, y) = pts.last().expect("a release");
    pointer(s, PointerKind::Up, x, y);
}

/// Is `id` filled red?
fn red(s: &Session, id: NodeId) -> bool {
    s.doc().unwrap().doc.node(id).and_then(|n| n.appearance.fill_paint().color()).is_some_and(|c| c.to_hex() == "#ff0000")
}

/// The Selection tool's move, the Scale and Rotate tools and the Pen, each with a command that
/// fills another object red during the drag: the release keeps both, as two undo steps, and Undo
/// takes back the drag first.
#[test]
fn the_move_scale_rotate_and_pen_drags_keep_a_command_run_during_them() {
    let drags: [(&str, &[(f64, f64)]); 4] = [
        ("selection", &[(120.0, 120.0), (140.0, 130.0), (170.0, 150.0)]),
        ("scale", &[(150.0, 150.0), (170.0, 170.0), (190.0, 190.0)]),
        ("rotate", &[(160.0, 120.0), (150.0, 150.0), (120.0, 160.0)]),
        ("pen", &[(300.0, 300.0), (320.0, 300.0), (350.0, 320.0)]),
    ];
    for (tool, pts) in drags {
        for real_time in [true, false] {
            let at = format!("{tool}, real time {real_time}");
            let (mut s, _, other) = two_squares(real_time);
            if tool == "pen" {
                s.execute("select.none", &json!({})).unwrap();
            }
            s.select_tool(tool, view()).unwrap();
            let (before, steps) = (others(&s, other), s.doc().unwrap().history.undo.len());
            drag_with(&mut s, pts, |s| {
                s.execute("paint.setFill", &json!({"ids": [other.0], "color": "#ff0000"})).unwrap();
            });
            assert!(!s.in_interaction(), "{at}");
            assert!(red(&s, other), "{at}: the release keeps the command's edit");
            assert_ne!(others(&s, other), before, "{at}: the release keeps the drag's edit");
            assert_eq!(s.doc().unwrap().history.undo.len(), steps + 2, "{at}");
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(others(&s, other), before, "{at}: Undo takes back the drag");
            assert!(red(&s, other), "{at}: and leaves the command's edit");
            s.execute("edit.undo", &json!({})).unwrap();
            assert!(!red(&s, other), "{at}: the next Undo takes back the command's edit");
        }
    }
}

/// A command that changes the object being moved: the move applies to the changed object.
#[test]
fn a_move_drag_moves_the_object_as_a_command_during_it_changed_it() {
    for real_time in [true, false] {
        let (mut s, target, _) = two_squares(real_time);
        s.select_tool("selection", view()).unwrap();
        let fill = |s: &Session| s.doc().unwrap().doc.node(target).unwrap().appearance.fill_paint().color().map(|c| c.to_hex());
        let was = fill(&s);
        drag_with(&mut s, &[(120.0, 120.0), (140.0, 130.0), (170.0, 150.0)], |s| {
            s.execute("paint.setFill", &json!({"ids": [target.0], "color": "#ff0000"})).unwrap();
        });
        let moved = s.doc().unwrap().doc.bounds_of(&[target], false);
        assert_eq!(moved, Some(Rect::new(150.0, 130.0, 190.0, 170.0)), "real time {real_time}");
        assert_eq!(fill(&s).as_deref(), Some("#ff0000"), "real time {real_time}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc.bounds_of(&[target], false), Some(Rect::new(100.0, 100.0, 140.0, 140.0)), "real time {real_time}");
        assert_eq!(fill(&s).as_deref(), Some("#ff0000"), "real time {real_time}: the fill is a step of its own");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(fill(&s), was, "real time {real_time}");
    }
}

/// A command that deletes the object being moved: the drag goes on and ends without an edit of
/// its own, on release and on Escape, and the deletion stays.
#[test]
fn a_move_drag_of_an_object_a_command_deleted_ends_without_an_edit() {
    for (real_time, escape) in [(true, false), (false, false), (true, true), (false, true)] {
        let at = format!("real time {real_time}, Escape {escape}");
        let (mut s, target, other) = two_squares(real_time);
        s.select_tool("selection", view()).unwrap();
        let steps = s.doc().unwrap().history.undo.len();
        pointer(&mut s, PointerKind::Down, 120.0, 120.0);
        pointer(&mut s, PointerKind::Drag, 140.0, 130.0);
        s.execute("edit.clear", &json!({"ids": [target.0]})).unwrap();
        assert!(s.doc().unwrap().doc.node(target).is_none(), "{at}");
        pointer(&mut s, PointerKind::Drag, 170.0, 150.0);
        if escape {
            s.tool_key(ToolKey::Escape, Mods::default(), view()).unwrap();
        }
        pointer(&mut s, PointerKind::Up, 170.0, 150.0);
        assert!(!s.in_interaction(), "{at}");
        assert!(s.doc().unwrap().doc.node(target).is_none(), "{at}: the deletion stays");
        assert_eq!(others(&s, other), [], "{at}");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "{at}: only the deletion is a step");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc.bounds_of(&[target], false), Some(Rect::new(100.0, 100.0, 140.0, 140.0)), "{at}");
    }
}

/// A command that hides or locks the object being moved: the object stays where it was and
/// unselected, and the drag ends without an edit of its own.
#[test]
fn a_move_drag_of_an_object_a_command_hid_or_locked_leaves_it_alone() {
    for command in ["object.hide", "object.lock"] {
        for real_time in [true, false] {
            let at = format!("{command}, real time {real_time}");
            let (mut s, target, _) = two_squares(real_time);
            s.select_tool("selection", view()).unwrap();
            let steps = s.doc().unwrap().history.undo.len();
            drag_with(&mut s, &[(120.0, 120.0), (140.0, 130.0), (170.0, 150.0)], |s| {
                s.execute(command, &json!({"ids": [target.0]})).unwrap();
            });
            assert!(!s.in_interaction(), "{at}");
            assert_eq!(s.doc().unwrap().doc.bounds_of(&[target], false), Some(Rect::new(100.0, 100.0, 140.0, 140.0)), "{at}: not moved");
            assert!(!s.doc().unwrap().selection.objects.contains(&target), "{at}: not selected");
            assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "{at}: only the command is a step");
        }
    }
}

/// The commands that end a drag first still do: Undo during a drag keeps the drag as a step and
/// takes it back, and `command.batch` and `swatch.renameGroup` keep it as a step before their own.
#[test]
fn undo_and_a_batch_during_a_drag_still_keep_the_drag_first() {
    for real_time in [true, false] {
        let (mut s, target, other) = two_squares(real_time);
        s.select_tool("selection", view()).unwrap();
        pointer(&mut s, PointerKind::Down, 120.0, 120.0);
        pointer(&mut s, PointerKind::Drag, 170.0, 150.0);
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(!s.in_interaction(), "real time {real_time}");
        assert_eq!(s.doc().unwrap().doc.bounds_of(&[target], false), Some(Rect::new(100.0, 100.0, 140.0, 140.0)), "real time {real_time}");
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc.bounds_of(&[target], false), Some(Rect::new(150.0, 130.0, 190.0, 170.0)), "real time {real_time}");
        pointer(&mut s, PointerKind::Up, 170.0, 150.0);

        let (mut s, target, other2) = two_squares(real_time);
        assert_eq!(other, other2);
        s.select_tool("selection", view()).unwrap();
        let steps = s.doc().unwrap().history.undo.len();
        pointer(&mut s, PointerKind::Down, 120.0, 120.0);
        pointer(&mut s, PointerKind::Drag, 170.0, 150.0);
        s.execute("command.batch", &json!({"commands": [{"command": "object.hide", "params": {"ids": [other.0]}}]})).unwrap();
        assert!(!s.in_interaction(), "real time {real_time}: the batch keeps the drag first");
        pointer(&mut s, PointerKind::Up, 170.0, 150.0);
        assert_eq!(s.doc().unwrap().history.undo.len(), steps + 2, "real time {real_time}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(visible(&s, other), "real time {real_time}: Undo takes back the batch first");
        assert_eq!(s.doc().unwrap().doc.bounds_of(&[target], false), Some(Rect::new(150.0, 130.0, 190.0, 170.0)), "real time {real_time}");

        // A color group renamed during the drag keeps the drag as a step before the rename.
        let (mut s, target, _) = two_squares(real_time);
        s.select_tool("selection", view()).unwrap();
        let steps = s.doc().unwrap().history.undo.len();
        pointer(&mut s, PointerKind::Down, 120.0, 120.0);
        pointer(&mut s, PointerKind::Drag, 170.0, 150.0);
        s.execute("swatch.renameGroup", &json!({"group": "Brights", "name": "Vivid"})).unwrap();
        assert!(!s.in_interaction(), "real time {real_time}: the rename keeps the drag first");
        pointer(&mut s, PointerKind::Up, 170.0, 150.0);
        assert_eq!(s.doc().unwrap().history.undo.len(), steps + 2, "real time {real_time}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(s.doc().unwrap().doc.swatch_groups.iter().any(|g| g.name == "Brights"), "real time {real_time}: Undo takes back the rename");
        assert_eq!(s.doc().unwrap().doc.bounds_of(&[target], false), Some(Rect::new(150.0, 130.0, 190.0, 170.0)), "real time {real_time}");
    }
}

/// A copy made with `object.transform` during an Alt-drag runs inside the drag, with Real-time
/// Drawing on and off: the release records one step that holds both.
#[test]
fn a_copy_during_an_alt_drag_is_part_of_the_drag() {
    for real_time in [true, false] {
        let (mut s, target, _) = two_squares(real_time);
        s.select_tool("selection", view()).unwrap();
        let steps = s.doc().unwrap().history.undo.len();
        let alt = Mods { alt: true, ..Mods::default() };
        for (kind, x, y) in [(PointerKind::Down, 120.0, 120.0), (PointerKind::Drag, 170.0, 150.0)] {
            s.pointer(&PointerEvent::new(kind, x, y).with_mods(alt), view()).unwrap();
        }
        s.execute("object.transform", &json!({"matrix": [1, 0, 0, 1, 0, 60], "copy": true, "ids": [target.0]})).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, 170.0, 150.0).with_mods(alt), view()).unwrap();
        assert!(!s.in_interaction(), "real time {real_time}");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "real time {real_time}: one step for the drag and the copy");

        // With Alt let go during the drag, the drag moves again, and the copy is a step of its own.
        let (mut s, target, _) = two_squares(real_time);
        s.select_tool("selection", view()).unwrap();
        let steps = s.doc().unwrap().history.undo.len();
        s.pointer(&PointerEvent::new(PointerKind::Down, 120.0, 120.0).with_mods(alt), view()).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Drag, 150.0, 140.0).with_mods(alt), view()).unwrap();
        pointer(&mut s, PointerKind::Drag, 170.0, 150.0);
        s.execute("object.transform", &json!({"matrix": [1, 0, 0, 1, 0, 60], "copy": true, "ids": [target.0]})).unwrap();
        pointer(&mut s, PointerKind::Up, 170.0, 150.0);
        assert_eq!(s.doc().unwrap().history.undo.len(), steps + 2, "real time {real_time}: the copy and the move");
    }
}

/// A command that changes only the current layer during a drag keeps its change through Escape.
#[test]
fn escape_keeps_a_current_layer_set_during_a_drag() {
    for real_time in [true, false] {
        let (mut s, _) = session(real_time);
        let first = s.doc().unwrap().active_layer;
        let second = NodeId(s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap());
        let first = first.expect("a current layer");
        s.execute("layer.setCurrent", &json!({"id": first.0})).unwrap();
        pointer(&mut s, PointerKind::Down, 100.0, 100.0);
        pointer(&mut s, PointerKind::Drag, 150.0, 150.0);
        s.execute("layer.setCurrent", &json!({"id": second.0})).unwrap();
        s.tool_key(ToolKey::Escape, Mods::default(), view()).unwrap();
        pointer(&mut s, PointerKind::Up, 150.0, 150.0);
        assert_eq!(s.doc().unwrap().active_layer, Some(second), "real time {real_time}");
    }
}

/// With Real-time Drawing on, the drag's preview is on show again right after the command.
#[test]
fn the_drag_preview_shows_again_after_the_command() {
    let (mut s, other) = session(true);
    pointer(&mut s, PointerKind::Down, 100.0, 100.0);
    pointer(&mut s, PointerKind::Drag, 150.0, 150.0);
    assert_eq!(objects(&s), 2, "the preview draws the rectangle");
    s.execute("object.hide", &json!({"ids": [other.0]})).unwrap();
    assert_eq!(objects(&s), 2, "the preview is on show after the command");
    s.execute("object.showAll", &json!({})).unwrap();
    assert_eq!(objects(&s), 2, "and after a second one");
    assert!(visible(&s, other));
    assert!(s.in_interaction());
    pointer(&mut s, PointerKind::Up, 150.0, 150.0);
    assert_eq!(objects(&s), 2);
    assert!(visible(&s, other));
}

/// A command that changes the selection during a move drag (Select All, Deselect, a duplicate)
/// leaves the drag moving the object it began with.
#[test]
fn a_selection_made_during_a_move_drag_leaves_the_drag_moving_its_object() {
    for command in ["select.all", "select.none", "edit.duplicate"] {
        for real_time in [true, false] {
            let at = format!("{command}, real time {real_time}");
            let (mut s, target, other) = two_squares(real_time);
            s.select_tool("selection", view()).unwrap();
            drag_with(&mut s, &[(120.0, 120.0), (140.0, 130.0), (170.0, 150.0)], |s| {
                s.execute(command, &json!({})).unwrap();
            });
            let d = &s.doc().unwrap().doc;
            assert_eq!(d.bounds_of(&[target], false), Some(Rect::new(150.0, 130.0, 190.0, 170.0)), "{at}: the drag moves its object");
            assert_eq!(d.bounds_of(&[other], false), Some(Rect::new(10.0, 10.0, 30.0, 30.0)), "{at}: and nothing else");
            if command == "edit.duplicate" {
                let copies = others(&s, other).into_iter().filter(|n| n.id != target).map(|n| n.id).collect::<Vec<_>>();
                assert_eq!(copies.len(), 1, "{at}");
                assert_eq!(d.bounds_of(&copies, false), Some(Rect::new(100.0, 100.0, 140.0, 140.0)), "{at}: the copy stays where it was made");
            }
        }
    }
}

/// A command that selects another object during a move drag: the drag's object stays selected,
/// and the fill and stroke proxies keep showing its paints.
#[test]
fn a_selection_made_during_a_move_drag_leaves_the_paints_of_the_drag_object() {
    for real_time in [true, false] {
        let at = format!("real time {real_time}");
        let (mut s, target, other) = two_squares(real_time);
        s.execute("paint.setFill", &json!({"ids": [target.0], "color": "#ff0000"})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [other.0], "color": "#0000ff"})).unwrap();
        s.execute("select.none", &json!({})).unwrap();
        s.execute("select.set", &json!({"ids": [target.0]})).unwrap();
        s.select_tool("selection", view()).unwrap();
        let fill = |s: &Session| s.paint.fill.color().map(|c| c.to_hex());
        assert_eq!(fill(&s).as_deref(), Some("#ff0000"), "{at}");
        pointer(&mut s, PointerKind::Down, 120.0, 120.0);
        pointer(&mut s, PointerKind::Drag, 140.0, 130.0);
        s.execute("select.set", &json!({"ids": [other.0]})).unwrap();
        assert_eq!(s.doc().unwrap().selection.objects, [target], "{at}");
        assert_eq!(fill(&s).as_deref(), Some("#ff0000"), "{at}: after the command");
        pointer(&mut s, PointerKind::Drag, 170.0, 150.0);
        pointer(&mut s, PointerKind::Up, 170.0, 150.0);
        assert_eq!(s.doc().unwrap().selection.objects, [target], "{at}");
        assert_eq!(fill(&s).as_deref(), Some("#ff0000"), "{at}: after the release");
    }
}

/// Replay the journal of `s` from `start` on `replayed`.
fn replay_from(s: &Session, start: usize, replayed: &mut Session) {
    for (id, p) in s.journal.iter().skip(start) {
        replayed.replay(id, p).unwrap();
    }
}

/// The journal of a move drag with a selection command run during it replays as the drag went:
/// the drag moves its own object, not the one the command selected.
#[test]
fn a_move_drag_after_a_selection_command_replays_on_its_own_object() {
    for real_time in [true, false] {
        let at = format!("real time {real_time}");
        let (mut s, target, other) = two_squares(real_time);
        s.select_tool("selection", view()).unwrap();
        let start = s.journal.len();
        drag_with(&mut s, &[(120.0, 120.0), (140.0, 130.0), (170.0, 150.0)], |s| {
            s.execute("select.set", &json!({"ids": [other.0]})).unwrap();
        });
        let bounds = |s: &Session| {
            let d = &s.doc().unwrap().doc;
            (d.bounds_of(&[target], false), d.bounds_of(&[other], false))
        };
        assert_eq!(bounds(&s), (Some(Rect::new(150.0, 130.0, 190.0, 170.0)), Some(Rect::new(10.0, 10.0, 30.0, 30.0))), "{at}");
        let (mut replayed, _, _) = two_squares(real_time);
        replay_from(&s, start, &mut replayed);
        assert_eq!(bounds(&replayed), bounds(&s), "{at}: {:?}", journaled(&s, start));
        assert_eq!(replayed.doc().unwrap().selection.objects, s.doc().unwrap().selection.objects, "{at}");
        assert_eq!(replayed.doc().unwrap().active_layer, s.doc().unwrap().active_layer, "{at}");
    }
}

/// A square on a second layer selected with a ruler guide, and a square on the first layer →
/// the session (with Real-time Drawing `real_time`) and the square on the first layer.
fn square_and_guide(real_time: bool) -> (Session, NodeId) {
    let mut s = Session::new();
    s.prefs.real_time_drawing = real_time;
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let other = s.doc().unwrap().selection.objects[0];
    s.execute("layer.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 40, "height": 40})).unwrap();
    let target = s.doc().unwrap().selection.objects[0];
    s.execute("guide.add", &json!({"vertical": true, "pos": 300})).unwrap();
    s.execute("select.set", &json!({"ids": [target.0]})).unwrap();
    s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
    (s, other)
}

/// The journal of a move drag of a square and a ruler guide, with a command during it that
/// selects an object on another layer, replays with the drag's selection, guide included, with
/// the layer the command made current and with the Layers panel rows the command left highlighted.
#[test]
fn a_move_drag_of_a_guide_after_a_selection_command_replays_its_selection_and_layer() {
    for real_time in [true, false] {
        let at = format!("real time {real_time}");
        let (mut s, other) = square_and_guide(real_time);
        s.select_tool("selection", view()).unwrap();
        let start = s.journal.len();
        drag_with(&mut s, &[(120.0, 120.0), (140.0, 130.0), (170.0, 150.0)], |s| {
            s.execute("select.set", &json!({"ids": [other.0]})).unwrap();
        });
        let (mut replayed, _) = square_and_guide(real_time);
        replay_from(&s, start, &mut replayed);
        let state = |s: &mut Session| {
            let d = s.doc().unwrap();
            let state = (
                d.selection.objects.clone(),
                d.selection.guides.clone(),
                d.active_layer,
                d.highlighted_rows(),
                d.doc.bounds_of(&d.selection.objects, false),
            );
            (state, s.execute("guide.list", &json!({})).unwrap())
        };
        assert_eq!(state(&mut replayed), state(&mut s), "{at}: {:?}", journaled(&s, start));
    }
}

/// The journal of a move drag of a targeted square, with a command during it that selects another
/// square, replays with the square targeted, as the drag leaves it.
#[test]
fn a_move_drag_of_a_targeted_object_after_a_selection_command_replays_its_target() {
    for real_time in [true, false] {
        let at = format!("real time {real_time}");
        let setup = || {
            let (mut s, target, other) = two_squares(real_time);
            s.execute("layer.target", &json!({"id": target.0})).unwrap();
            s.select_tool("selection", view()).unwrap();
            (s, other)
        };
        let (mut s, other) = setup();
        let start = s.journal.len();
        drag_with(&mut s, &[(120.0, 120.0), (140.0, 130.0), (170.0, 150.0)], |s| {
            s.execute("select.set", &json!({"ids": [other.0]})).unwrap();
        });
        let (mut replayed, _) = setup();
        replay_from(&s, start, &mut replayed);
        let state = |s: &Session| {
            let d = s.doc().unwrap();
            (d.selection.objects.clone(), d.selection.target, d.doc.bounds_of(&d.selection.objects, false))
        };
        assert_eq!(state(&replayed), state(&s), "{at}: {:?}", journaled(&s, start));
    }
}

/// The journal of a move drag of a targeted square and a ruler guide, with a command during it
/// that selects another square, replays with the square targeted and the guide selected.
#[test]
fn a_move_drag_of_a_targeted_object_and_a_guide_after_a_selection_command_replays_both() {
    for real_time in [true, false] {
        let at = format!("real time {real_time}");
        let setup = || {
            let (mut s, target, other) = two_squares(real_time);
            s.execute("guide.add", &json!({"vertical": true, "pos": 300})).unwrap();
            s.execute("layer.target", &json!({"id": target.0})).unwrap();
            s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
            s.select_tool("selection", view()).unwrap();
            (s, target, other)
        };
        let (mut s, target, other) = setup();
        let start = s.journal.len();
        drag_with(&mut s, &[(120.0, 120.0), (140.0, 130.0), (170.0, 150.0)], |s| {
            s.execute("select.set", &json!({"ids": [other.0]})).unwrap();
        });
        let state = |s: &mut Session| {
            let d = s.doc().unwrap();
            let state = (d.selection.objects.clone(), d.selection.target, d.selection.guides.clone(), d.doc.bounds_of(&d.selection.objects, false));
            (state, s.execute("guide.list", &json!({})).unwrap())
        };
        let live = state(&mut s);
        assert_eq!((live.0.1, live.0.2.len()), (Some(target), 1), "{at}: the drag leaves the square targeted and the guide selected");
        let (mut replayed, _, _) = setup();
        replay_from(&s, start, &mut replayed);
        assert_eq!(state(&mut replayed), live, "{at}: {:?}", journaled(&s, start));
    }
}

/// The journal of a move drag of a path with every anchor selected as anchor points, with a
/// command during it that selects another square, replays with those anchors selected.
#[test]
fn a_move_drag_of_every_anchor_of_a_path_after_a_selection_command_replays_its_anchors() {
    for real_time in [true, false] {
        let at = format!("real time {real_time}");
        let setup = || {
            let (mut s, target, other) = two_squares(real_time);
            s.execute("select.set", &json!({"ids": [target.0]})).unwrap();
            s.execute("select.anchors", &json!({"id": target.0, "anchors": [[0, 0]], "mode": "add"})).unwrap();
            s.select_tool("selection", view()).unwrap();
            (s, target, other)
        };
        let (mut s, target, other) = setup();
        let every = s.doc().unwrap().selection.anchors.get(&target).cloned();
        assert!(every.as_ref().is_some_and(|a| a.len() > 1), "{at}: {every:?}");
        let start = s.journal.len();
        drag_with(&mut s, &[(120.0, 120.0), (140.0, 130.0), (170.0, 150.0)], |s| {
            s.execute("select.set", &json!({"ids": [other.0]})).unwrap();
        });
        let state = |s: &Session| {
            let d = s.doc().unwrap();
            (d.selection.objects.clone(), d.selection.anchors.clone(), d.doc.bounds_of(&d.selection.objects, false))
        };
        assert_eq!(state(&s).1.get(&target), every.as_ref(), "{at}: the drag leaves every anchor selected as anchors");
        let (mut replayed, _, _) = setup();
        replay_from(&s, start, &mut replayed);
        assert_eq!(state(&replayed), state(&s), "{at}: {:?}", journaled(&s, start));
    }
}

/// The journal of a move drag of a targeted layer's art, with a duplicate made on that layer
/// during it, replays with the drag moving its own objects, not the duplicates.
#[test]
fn a_move_drag_of_a_targeted_layer_after_a_duplicate_replays_its_objects() {
    for real_time in [true, false] {
        let at = format!("real time {real_time}");
        let setup = || {
            let (mut s, _, _) = two_squares(real_time);
            let layer = s.doc().unwrap().doc.layers[0].id;
            s.execute("layer.target", &json!({"id": layer.0})).unwrap();
            s.select_tool("selection", view()).unwrap();
            s
        };
        let mut s = setup();
        let start = s.journal.len();
        drag_with(&mut s, &[(120.0, 120.0), (140.0, 130.0), (170.0, 150.0)], |s| {
            s.execute("edit.duplicate", &json!({})).unwrap();
        });
        let mut replayed = setup();
        replay_from(&s, start, &mut replayed);
        let state = |s: &Session| {
            let d = s.doc().unwrap();
            (d.selection.objects.clone(), d.doc.bounds_of(&d.selection.objects, false), objects(s))
        };
        assert_eq!(state(&replayed), state(&s), "{at}: {:?}", journaled(&s, start));
    }
}

/// A selected user slice and a square → the session (with Real-time Drawing `real_time`), the
/// slice and the square.
fn slice_and_square(real_time: bool) -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    s.prefs.real_time_drawing = real_time;
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let other = s.doc().unwrap().selection.objects[0];
    let slice = s.execute("object.slice.create", &json!({"x": 200, "y": 200, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    s.execute("object.slice.select", &json!({"slices": [slice]})).unwrap();
    (s, NodeId(slice), other)
}

/// The journal of a Slice Selection drag, with a command during it that selects an object,
/// replays with the slice selected, as the drag leaves it.
#[test]
fn a_slice_drag_after_a_selection_command_replays_its_slice_selection() {
    for real_time in [true, false] {
        let at = format!("real time {real_time}");
        let (mut s, slice, other) = slice_and_square(real_time);
        s.select_tool("sliceSelection", view()).unwrap();
        let start = s.journal.len();
        drag_with(&mut s, &[(220.0, 220.0), (230.0, 230.0), (240.0, 240.0)], |s| {
            s.execute("select.set", &json!({"ids": [other.0]})).unwrap();
        });
        assert_eq!(s.doc().unwrap().selection.slices, [slice], "{at}");
        let (mut replayed, _, _) = slice_and_square(real_time);
        replay_from(&s, start, &mut replayed);
        let state = |s: &Session| {
            let sel = &s.doc().unwrap().selection;
            (sel.objects.clone(), sel.slices.clone())
        };
        assert_eq!(state(&replayed), state(&s), "{at}: {:?}", journaled(&s, start));
    }
}

/// A Warp stroke over two selected squares, with a command that deletes one of them during the
/// stroke: the release still warps the other square, and the stroke's journal entry names it.
#[test]
fn a_warp_stroke_over_an_object_a_command_deleted_still_warps_the_others() {
    for real_time in [true, false] {
        let at = format!("real time {real_time}");
        let (mut s, target, other) = two_squares(real_time);
        s.execute("select.all", &json!({})).unwrap();
        s.select_tool("warp", view()).unwrap();
        let before = s.doc().unwrap().doc.node(target).cloned();
        let steps = s.doc().unwrap().history.undo.len();
        pointer(&mut s, PointerKind::Down, 120.0, 120.0);
        pointer(&mut s, PointerKind::Drag, 135.0, 125.0);
        let r = s.execute("edit.clear", &json!({"ids": [other.0]})).unwrap();
        assert!(r.get("warning").is_none(), "{at}: {r}");
        pointer(&mut s, PointerKind::Up, 135.0, 125.0);
        assert!(!s.in_interaction(), "{at}");
        assert!(s.doc().unwrap().doc.node(other).is_none(), "{at}: the deletion stays");
        assert_ne!(s.doc().unwrap().doc.node(target).cloned(), before, "{at}: the release warps the other square");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps + 2, "{at}: the deletion and the stroke are a step each");
        let (id, p) = s.journal.last().unwrap();
        assert_eq!((id.as_str(), &p["ids"]), ("object.liquify", &json!([target.0])), "{at}");
    }
}

/// The Scale and Rotate tools and the Pen adding to a path, with a command that deletes the
/// object they act on during the drag: the drag ends without an edit of its own and the deletion
/// stays.
#[test]
fn a_scale_rotate_or_pen_drag_of_an_object_a_command_deleted_ends_without_an_edit() {
    let drags: [(&str, &[(f64, f64)]); 3] = [
        ("scale", &[(150.0, 150.0), (170.0, 170.0), (190.0, 190.0)]),
        ("rotate", &[(160.0, 120.0), (150.0, 150.0), (120.0, 160.0)]),
        ("pen", &[(350.0, 350.0), (370.0, 360.0), (380.0, 380.0)]),
    ];
    for (tool, pts) in drags {
        for real_time in [true, false] {
            let at = format!("{tool}, real time {real_time}");
            let (mut s, target, _) = two_squares(real_time);
            s.select_tool(tool, view()).unwrap();
            let target = if tool == "pen" {
                s.execute("select.none", &json!({})).unwrap();
                for (x, y) in [(300.0, 300.0), (350.0, 300.0)] {
                    pointer(&mut s, PointerKind::Down, x, y);
                    pointer(&mut s, PointerKind::Up, x, y);
                }
                s.doc().unwrap().selection.objects[0]
            } else {
                target
            };
            let before = s.doc().unwrap().doc.clone();
            let steps = s.doc().unwrap().history.undo.len();
            drag_with(&mut s, pts, |s| {
                s.execute("edit.clear", &json!({"ids": [target.0]})).unwrap();
            });
            assert!(!s.in_interaction(), "{at}");
            assert!(s.doc().unwrap().doc.node(target).is_none(), "{at}: the deletion stays");
            assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "{at}: only the deletion is a step");
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(*s.doc().unwrap().doc, *before, "{at}: Undo brings the object back as it was");
        }
    }
}

/// With the Live Paint Bucket's Selection tool (Cmd+drag), a fill chosen during the drag stays the
/// new-art fill after the release and after Escape, though the drag puts back the bucket's colors
/// it began with.
#[test]
fn a_fill_chosen_during_a_bucket_drag_stays() {
    for (real_time, escape) in [(true, false), (false, false), (true, true), (false, true)] {
        let at = format!("real time {real_time}, Escape {escape}");
        let (mut s, _, _) = two_squares(real_time);
        s.select_tool("livePaintBucket", view()).unwrap();
        let cmd = Mods { cmd: true, ..Mods::default() };
        for (kind, x, y) in [(PointerKind::Down, 120.0, 120.0), (PointerKind::Drag, 140.0, 130.0)] {
            s.pointer(&PointerEvent::new(kind, x, y).with_mods(cmd), view()).unwrap();
        }
        assert!(s.in_interaction(), "{at}: the lent Selection tool drags the square");
        s.execute("paint.setFill", &json!({"color": "#00ff00", "ids": []})).unwrap();
        let green = s.paint.fill.clone();
        s.pointer(&PointerEvent::new(PointerKind::Drag, 170.0, 150.0).with_mods(cmd), view()).unwrap();
        if escape {
            s.tool_key(ToolKey::Escape, Mods::default(), view()).unwrap();
        }
        s.pointer(&PointerEvent::new(PointerKind::Up, 170.0, 150.0).with_mods(cmd), view()).unwrap();
        assert_eq!(s.paint.fill, green, "{at}");
    }
}

/// A command run during a Quick Pen press still runs inside the press, so it records no undo
/// step of its own while the press is held.
#[test]
fn a_command_during_a_quick_pen_press_runs_inside_it() {
    for real_time in [true, false] {
        let at = format!("real time {real_time}");
        let (mut s, other) = session(real_time);
        s.select_tool(vectorcraft_tools::quickpen::ID, view()).unwrap();
        pointer(&mut s, PointerKind::Down, 50.0, 50.0);
        pointer(&mut s, PointerKind::Up, 50.0, 50.0);
        pointer(&mut s, PointerKind::Down, 100.0, 100.0);
        pointer(&mut s, PointerKind::Drag, 150.0, 120.0);
        assert!(s.in_interaction(), "{at}: the press drags a piece");
        let steps = s.doc().unwrap().history.undo.len();
        s.execute("object.hide", &json!({"ids": [other.0]})).unwrap();
        assert!(s.in_interaction(), "{at}");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps, "{at}: no step of its own during the press");
    }
}
