//! Transform & utility tools driven through `Session::pointer`, and their commands.

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Node};
use vectorcraft_geom::{Point, Rect, Vec2, shapes};
use vectorcraft_tools::distort::perspective::{self as persp, Plane};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::tests_large_edits::{soon, undo_gives_back};
use crate::tests_persp_select::{bounds_of, grid, persp_session, plane_pts};
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    // Separately created fixtures must compare equal across clock ticks.
    s.execute("file.new", &json!({"width": 800, "height": 600, "created": null})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

fn gesture(s: &mut Session, tool: &str, pts: &[(f64, f64)], mods: Mods) -> Vec<UiRequest> {
    let v = ViewInfo::default();
    s.select_tool(tool, v).unwrap();
    let mut out = vec![];
    for (i, (x, y)) in pts.iter().enumerate() {
        let kind = if i == 0 {
            PointerKind::Down
        } else if i == pts.len() - 1 {
            PointerKind::Up
        } else {
            PointerKind::Drag
        };
        out.extend(s.pointer(&PointerEvent::new(kind, *x, *y).with_mods(mods), v).unwrap());
    }
    out
}

fn close(a: Rect, b: Rect) -> bool {
    (a.x0 - b.x0).abs() < 1e-6 && (a.y0 - b.y0).abs() < 1e-6 && (a.x1 - b.x1).abs() < 1e-6 && (a.y1 - b.y1).abs() < 1e-6
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn rotate_tool_drag_rotates_about_center_one_undo() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 50.0);
    let n = undo_len(&s);
    gesture(&mut s, "rotate", &[(250.0, 125.0), (200.0, 175.0), (150.0, 225.0), (150.0, 225.0)], Mods::default());
    // 90° about (150,125): 100×50 becomes 50×100.
    assert!(close(bounds(&s, a), Rect::new(125.0, 75.0, 175.0, 175.0)), "{:?}", bounds(&s, a));
    assert_eq!(undo_len(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(close(bounds(&s, a), Rect::new(100.0, 100.0, 200.0, 150.0)));
}

#[test]
fn rotate_tool_alt_release_copies() {
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 100.0, 50.0);
    let alt = Mods { alt: true, ..Default::default() };
    let v = ViewInfo::default();
    s.select_tool("rotate", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 250.0, 125.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 150.0, 225.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 150.0, 225.0).with_mods(alt), v).unwrap();
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 2);
}

/// Returns a session with a group of two rectangles and a rectangle to its right, both selected.
fn group_and_rect() -> Session {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 40.0, 40.0);
    let b = rect(&mut s, 160.0, 100.0, 40.0, 40.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    rect(&mut s, 220.0, 100.0, 40.0, 40.0);
    s.execute("select.all", &json!({})).unwrap();
    s
}

/// Sends `kind` at `(x, y)` to the active tool, with Alt held when `alt` is set.
fn drag_at(s: &mut Session, kind: PointerKind, (x, y): (f64, f64), alt: bool) {
    let mods = Mods { alt, ..Mods::default() };
    s.pointer(&PointerEvent::new(kind, x, y).with_mods(mods), ViewInfo::default()).unwrap();
}

/// Returns the command and params of the interaction's last preview.
fn previewed(s: &Session) -> (String, Value) {
    s.doc().unwrap().interaction.as_ref().and_then(|it| it.preview.clone()).unwrap()
}

/// Returns the document and the selected objects that `cmd` with `params` gives on a new
/// [`group_and_rect`] session, after `first` when it is given.
fn on_a_new_session(first: Option<(&str, &Value)>, cmd: &str, params: &Value) -> (Document, Vec<NodeId>) {
    let mut t = group_and_rect();
    if let Some((c, p)) = first {
        t.execute(c, p).unwrap();
    }
    t.execute(cmd, params).unwrap();
    let st = t.doc().unwrap();
    (Document::clone(&st.doc), st.selection.objects.clone())
}

/// Returns true when the active document and its selected objects are `want`.
fn shows(s: &Session, want: &(Document, Vec<NodeId>)) -> bool {
    let st = s.doc().unwrap();
    *st.doc == want.0 && st.selection.objects == want.1
}

/// An Alt-drag with the Selection tool or the Rotate tool copies the selection on its first move,
/// and the moves after it transform those copies: the interaction keeps them, made once, until
/// the drag ends. The Move dialog's Preview with Copy previews `object.move` with `copy` in the
/// same way.
#[test]
fn an_alt_drag_copies_the_selection_once() {
    let kept = |s: &Session| s.doc().unwrap().interaction.as_ref().and_then(|it| it.copies.clone());
    for (tool, [press, moves @ ..]) in [
        ("selection", [(120.0, 120.0), (140.0, 120.0), (160.0, 120.0), (180.0, 120.0)]),
        ("rotate", [(300.0, 120.0), (300.0, 140.0), (300.0, 160.0), (300.0, 180.0)]),
    ] {
        let mut s = group_and_rect();
        s.select_tool(tool, ViewInfo::default()).unwrap();
        drag_at(&mut s, PointerKind::Down, press, true);
        let mut first = None;
        for at in moves {
            drag_at(&mut s, PointerKind::Drag, at, true);
            let k = kept(&s).unwrap_or_else(|| panic!("{tool}: the move to {at:?} keeps its copies"));
            assert!(Arc::ptr_eq(first.get_or_insert_with(|| k.clone()), &k), "{tool}: the move to {at:?} transforms the first move's copies");
        }
        drag_at(&mut s, PointerKind::Up, moves[2], true);
        assert!(s.doc().unwrap().interaction.is_none());
        assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 4, "{tool}: the objects and their copies");
    }
    let mut s = group_and_rect();
    s.begin_interaction("Move").unwrap();
    s.preview("object.move", &json!({"dx": 10, "dy": 0, "copy": true})).unwrap();
    let first = kept(&s).expect("the first preview keeps its copies");
    s.preview("object.move", &json!({"dx": 20, "dy": 0, "copy": true})).unwrap();
    assert!(kept(&s).is_some_and(|k| Arc::ptr_eq(&k, &first)), "the second preview transforms the first one's copies");
}

/// Each move of an Alt-drag with the Selection tool leaves the document and the selected copies
/// that one `object.transform` with `copy` and that move's matrix gives, with the same ids on every
/// move; the release keeps the last move as one undo step named Copy.
#[test]
fn each_move_of_an_alt_drag_gives_what_one_transform_with_copy_gives() {
    let mut s = group_and_rect();
    let (before, undo) = (s.doc().unwrap().doc.clone(), undo_len(&s));
    s.select_tool("selection", ViewInfo::default()).unwrap();
    drag_at(&mut s, PointerKind::Down, (120.0, 120.0), true);
    let mut copies = None;
    for x in [140.0, 160.0, 125.0, 190.0] {
        drag_at(&mut s, PointerKind::Drag, (x, 120.0), true);
        let (cmd, params) = previewed(&s);
        assert!(params["copy"] == json!(true), "{params}");
        assert!(shows(&s, &on_a_new_session(None, &cmd, &params)), "the move to {x}");
        let ids = s.doc().unwrap().selection.objects.clone();
        let first = copies.get_or_insert_with(|| ids.clone());
        assert_eq!(*first, ids, "the copies keep their ids");
    }
    drag_at(&mut s, PointerKind::Up, (190.0, 120.0), true);
    let (cmd, params) = s.journal.last().cloned().unwrap();
    assert!(shows(&s, &on_a_new_session(None, &cmd, &params)));
    assert_eq!(undo_len(&s), undo + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().map(|e| e.label.as_str()), Some("Copy"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(*s.doc().unwrap().doc == *before, "Undo gives back the document");
}

/// While Alt is released during a drag, the previews move the selected objects themselves; once Alt
/// is pressed again, they copy the objects again, with the ids of the first copies. The release
/// names the undo step for the last move.
#[test]
fn releasing_alt_during_a_drag_moves_the_objects_themselves() {
    let mut s = group_and_rect();
    let originals = s.doc().unwrap().selection.objects.clone();
    s.select_tool("selection", ViewInfo::default()).unwrap();
    drag_at(&mut s, PointerKind::Down, (120.0, 120.0), true);
    drag_at(&mut s, PointerKind::Drag, (140.0, 120.0), true);
    let copies = s.doc().unwrap().selection.objects.clone();
    for (x, alt) in [(160.0, false), (180.0, true), (200.0, false)] {
        drag_at(&mut s, PointerKind::Drag, (x, 120.0), alt);
        let (cmd, params) = previewed(&s);
        assert!(params["copy"] == json!(alt), "{params}");
        assert!(shows(&s, &on_a_new_session(None, &cmd, &params)), "the move to {x}, Alt: {alt}");
        let selected = if alt { &copies } else { &originals };
        assert_eq!(&s.doc().unwrap().selection.objects, selected);
    }
    drag_at(&mut s, PointerKind::Up, (200.0, 120.0), false);
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 2, "no copies");
    assert_eq!(s.doc().unwrap().history.undo.last().map(|e| e.label.as_str()), Some("Move"));
}

/// A transform with `copy` of the objects selected when an Alt-drag started, run while the drag is
/// open after its first move, copies the document as that move left it: the move's copies stay,
/// and new copies go directly above the objects.
#[test]
fn a_copy_run_during_an_alt_drag_copies_the_document_as_the_move_left_it() {
    let mut s = group_and_rect();
    let ids: Vec<u64> = s.doc().unwrap().selection.objects.iter().map(|i| i.0).collect();
    s.select_tool("selection", ViewInfo::default()).unwrap();
    drag_at(&mut s, PointerKind::Down, (120.0, 120.0), true);
    drag_at(&mut s, PointerKind::Drag, (140.0, 120.0), true);
    let (cmd, params) = previewed(&s);
    let again = json!({"matrix": [1, 0, 0, 1, 0, 60], "copy": true, "ids": ids});
    s.execute("object.transform", &again).unwrap();
    assert!(shows(&s, &on_a_new_session(Some((&cmd, &params)), "object.transform", &again)));
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 6, "the objects and two copies of each");
}

/// A command that changes the drag's snapshot during an Alt-drag (showing the perspective grid)
/// keeps its change in the moves after it, which copy the changed snapshot.
#[test]
fn a_grid_shown_during_an_alt_drag_stays_shown_in_the_moves_after_it() {
    let mut s = group_and_rect();
    s.select_tool("selection", ViewInfo::default()).unwrap();
    drag_at(&mut s, PointerKind::Down, (120.0, 120.0), true);
    drag_at(&mut s, PointerKind::Drag, (140.0, 120.0), true);
    let show = json!({"visible": true});
    s.execute("perspective.grid.show", &show).unwrap();
    drag_at(&mut s, PointerKind::Drag, (160.0, 120.0), true);
    let (cmd, params) = previewed(&s);
    assert!(shows(&s, &on_a_new_session(Some(("perspective.grid.show", &show)), &cmd, &params)));
}

/// Canceling an Alt-drag after a few moves puts back the document and the selection it started
/// from, with no undo step, and it ends the interaction, which held the copies.
#[test]
fn canceling_an_alt_drag_leaves_the_document_as_it_was() {
    let mut s = group_and_rect();
    let (before, selection, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().selection.clone(), undo_len(&s));
    s.select_tool("selection", ViewInfo::default()).unwrap();
    drag_at(&mut s, PointerKind::Down, (120.0, 120.0), true);
    for x in [140.0, 160.0, 180.0] {
        drag_at(&mut s, PointerKind::Drag, (x, 120.0), true);
    }
    s.cancel_interaction().unwrap();
    let st = s.doc().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &before), "the document it started from");
    assert_eq!(st.selection, selection);
    assert_eq!(undo_len(&s), undo);
    assert!(s.doc().unwrap().interaction.is_none());
}

/// Returns a session with the grid and the square of [`persp_session`], the grid shown with its
/// right plane active, and a group of two squares attached to the right plane beside the square,
/// with the square and the group selected; and the square's id.
fn square_and_group_in_perspective() -> (Session, NodeId) {
    let (mut s, square) = persp_session();
    let a = rect(&mut s, 525.0, 390.0, 20.0, 20.0);
    let b = rect(&mut s, 550.0, 390.0, 20.0, 20.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let group = s.execute("object.group", &json!({})).unwrap()["id"].clone();
    s.execute("perspective.attach", &json!({"ids": [group], "plane": "right"})).unwrap();
    s.execute("perspective.grid.show", &json!({"visible": true})).unwrap();
    s.execute("perspective.plane.set", &json!({"plane": "right"})).unwrap();
    s.execute("select.set", &json!({"ids": [square.0, group]})).unwrap();
    (s, square)
}

/// Returns the press and the three moves of an Alt-drag on [`square_and_group_in_perspective`]: from
/// the middle of `square`, 10 pt apart along the right plane, or with `widget`, from the right
/// plane's widget, 4 pt apart to the left.
fn persp_drag(s: &Session, square: NodeId, widget: bool) -> [(f64, f64); 4] {
    let g = grid(s);
    let xy = |p: Point| (p.x, p.y);
    if widget {
        let (_, w) = g.plane_widgets().into_iter().find(|(p, _)| *p == Plane::Right).unwrap();
        [0.0, 4.0, 8.0, 12.0].map(|dx| xy(w + Vec2::new(-dx, 0.0)))
    } else {
        let c = bounds_of(&plane_pts(s, square, Plane::Right, 0.0)).center();
        let h = g.homography(Plane::Right).unwrap();
        [0.0, 10.0, 20.0, 30.0].map(|du| xy(h.apply(c + Vec2::new(du, 0.0)).unwrap()))
    }
}

/// An Alt-drag of objects in perspective with the Perspective Selection tool, and an Alt-drag of a
/// plane widget with that tool or the Perspective Grid tool, copy the objects on the first move, and
/// the moves after it map those copies: the interaction keeps them, made once, until the drag ends.
#[test]
fn an_alt_drag_in_perspective_copies_the_objects_once() {
    let kept = |s: &Session| s.doc().unwrap().interaction.as_ref().and_then(|it| it.copies.clone());
    for (tool, widget) in [("perspectiveSelection", false), ("perspectiveSelection", true), ("perspectiveGrid", true)] {
        let what = if widget { "plane widget" } else { "objects" };
        let (mut s, square) = square_and_group_in_perspective();
        let [press, moves @ ..] = persp_drag(&s, square, widget);
        s.select_tool(tool, ViewInfo::default()).unwrap();
        drag_at(&mut s, PointerKind::Down, press, true);
        let mut first = None;
        for at in moves {
            drag_at(&mut s, PointerKind::Drag, at, true);
            let k = kept(&s).unwrap_or_else(|| panic!("{tool}, {what}: the move to {at:?} keeps its copies"));
            assert!(Arc::ptr_eq(first.get_or_insert_with(|| k.clone()), &k), "{tool}, {what}: the move to {at:?} maps the first move's copies");
        }
        drag_at(&mut s, PointerKind::Up, moves[2], true);
        assert!(s.doc().unwrap().interaction.is_none());
        assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 4, "{tool}, {what}: the objects and their copies");
    }
}

/// Each move of an Alt-drag with the Perspective Selection tool leaves the document and the selected
/// objects that one command with that move's params gives: `perspective.move` for objects and
/// `perspective.plane.move` for a plane widget. A move with Alt released moves the objects (or the
/// plane alone), and the next move with Alt copies them again with the first copies' ids. The
/// release keeps the last move as one undo step, Undo gives back the document, and a drag canceled
/// after its moves leaves the document as it was.
#[test]
fn each_move_of_an_alt_drag_in_perspective_gives_what_one_command_gives() {
    let by_one_command = |cmd: &str, params: &Value| {
        let (mut t, _) = square_and_group_in_perspective();
        t.execute(cmd, params).unwrap();
        let st = t.doc().unwrap();
        (Document::clone(&st.doc), st.selection.objects.clone())
    };
    for (widget, label) in [(false, "Copy in Perspective"), (true, "Move Plane")] {
        let (mut s, square) = square_and_group_in_perspective();
        let (before, originals, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().selection.objects.clone(), undo_len(&s));
        let [press, moves @ ..] = persp_drag(&s, square, widget);
        s.select_tool("perspectiveSelection", ViewInfo::default()).unwrap();
        drag_at(&mut s, PointerKind::Down, press, true);
        let mut copies = None;
        for (at, alt) in moves.into_iter().zip([true, false, true]) {
            drag_at(&mut s, PointerKind::Drag, at, alt);
            let (cmd, params) = previewed(&s);
            assert!(shows(&s, &by_one_command(&cmd, &params)), "{label}: the move to {at:?}, Alt: {alt}");
            let selected = s.doc().unwrap().selection.objects.clone();
            if alt {
                assert_eq!(copies.get_or_insert_with(|| selected.clone()), &selected, "{label}: the copies keep their ids");
            } else {
                assert_eq!(selected, originals, "{label}: the objects themselves");
            }
        }
        drag_at(&mut s, PointerKind::Up, moves[2], true);
        let (cmd, params) = s.journal.last().cloned().unwrap();
        assert!(shows(&s, &by_one_command(&cmd, &params)), "{label}: the release");
        assert_eq!(undo_len(&s), undo + 1);
        assert_eq!(s.doc().unwrap().history.undo.last().map(|e| e.label.as_str()), Some(label));
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(*s.doc().unwrap().doc == *before, "{label}: Undo gives back the document");
        let doc = s.doc().unwrap().doc.clone();
        drag_at(&mut s, PointerKind::Down, press, true);
        for at in moves {
            drag_at(&mut s, PointerKind::Drag, at, true);
        }
        s.cancel_interaction().unwrap();
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &doc), "{label}: Cancel puts back the document");
        assert_eq!(s.doc().unwrap().selection.objects, originals);
        assert_eq!(undo_len(&s), undo);
    }
}

/// On a grid as files from before attachments were kept on the objects store it (the grid lists the
/// attachments, and the objects have none of their own), each move of an Alt-drag with the
/// Perspective Selection tool, of the objects or of a plane widget, leaves what one command with
/// that move's params gives, with the copies attached to the right plane.
#[test]
fn an_alt_drag_in_perspective_on_a_grid_from_an_older_file_attaches_the_copies() {
    let older = || {
        let (s, square) = square_and_group_in_perspective();
        let ids = s.doc().unwrap().selection.objects.clone();
        let mut d = Document::clone(&s.doc().unwrap().doc);
        let mut attached = serde_json::Map::new();
        for id in &ids {
            persp::release(d.node_mut(*id).unwrap());
            attached.insert(id.0.to_string(), json!("right"));
        }
        d.unknown.get_mut(persp::DOC_KEY).unwrap()["attached"] = Value::Object(attached);
        let mut t = Session::new();
        t.add_document(d, None);
        t.execute("select.set", &json!({"ids": ids})).unwrap();
        (t, square)
    };
    for (what, widget) in [("objects", false), ("plane widget", true)] {
        let (mut s, square) = older();
        let [press, moves @ ..] = persp_drag(&s, square, widget);
        s.select_tool("perspectiveSelection", ViewInfo::default()).unwrap();
        drag_at(&mut s, PointerKind::Down, press, true);
        for at in moves {
            drag_at(&mut s, PointerKind::Drag, at, true);
            let (cmd, params) = previewed(&s);
            let (mut t, _) = older();
            t.execute(&cmd, &params).unwrap();
            let st = t.doc().unwrap();
            assert!(shows(&s, &(Document::clone(&st.doc), st.selection.objects.clone())), "{what}: the move to {at:?}");
            let d = &s.doc().unwrap().doc;
            for copy in &s.doc().unwrap().selection.objects {
                assert_eq!(persp::attachment(d.node(*copy).unwrap()).map(|a| a.0), Some(Plane::Right), "{what}: the copy {copy:?} is attached");
            }
        }
    }
}

/// With Alt held, a Perspective Selection tool press on one of 30,000 squares on the left plane,
/// four moves of 20 pt and the release copy the squares once: each move maps the copies that the
/// first one kept, and the drag leaves what one `perspective.move` with copy leaves, as one undo
/// step.
#[test]
fn alt_drag_in_perspective_of_30000_objects_takes_few_tree_walks() {
    // The squares are 6 pt wide and 10 pt apart, in rows of 150, between the left vanishing point
    // of the two-point grid and its origin.
    let on_the_left = |d: &mut Document, k: usize| {
        let (x, y) = (300.0 + (k % 150) as f64 * 10.0, 100.0 + (k / 150) as f64 * 10.0);
        Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, y, x + 6.0, y + 6.0)), Appearance::default_art())
    };
    let in_perspective = || {
        let mut s = crate::tests_large_edits::session(30_000, on_the_left);
        s.execute("perspective.grid.preset", &json!({"kind": 2})).unwrap();
        s.execute("perspective.attach", &json!({"plane": "left"})).unwrap();
        s
    };
    let s = in_perspective();
    let before = s.doc().unwrap().doc.clone();
    // The press lands on a square in the middle of the art.
    let grab = bounds(&s, s.doc().unwrap().selection.objects[15_075]).center();
    let (mut s, kept) = soon(s, "the Alt-drag in perspective", move |s| {
        let (v, alt) = (ViewInfo::default(), Mods { alt: true, ..Mods::default() });
        s.select_tool("perspectiveSelection", v)?;
        let at = |kind, dx: f64| PointerEvent::new(kind, grab.x + dx, grab.y).with_mods(alt);
        s.pointer(&at(PointerKind::Down, 0.0), v)?;
        let mut kept = vec![];
        for dx in [20.0, 40.0, 60.0, 80.0] {
            s.pointer(&at(PointerKind::Drag, dx), v)?;
            kept.push(s.doc()?.interaction.as_ref().and_then(|it| it.copies.clone()));
        }
        s.pointer(&at(PointerKind::Up, 80.0), v)?;
        Ok::<_, EngineError>(kept)
    });
    let kept = kept.unwrap();
    let first = kept[0].clone().expect("the first move keeps its copies");
    assert!(kept.iter().all(|k| k.as_ref().is_some_and(|k| Arc::ptr_eq(k, &first))), "each move maps the copies the first one kept");
    let (cmd, params) = s.journal.last().cloned().unwrap();
    let mut want = in_perspective();
    want.execute(&cmd, &params).unwrap();
    assert!(*s.doc().unwrap().doc == *want.doc().unwrap().doc, "the document one perspective.move with copy leaves");
    assert!(s.doc().unwrap().selection.objects == want.doc().unwrap().selection.objects, "the copies are selected");
    assert_eq!(s.doc().unwrap().history.undo.last().map(|e| e.label.as_str()), Some("Copy in Perspective"));
    undo_gives_back(&mut s, &before);
}

#[test]
fn rotate_tool_alt_click_requests_dialog() {
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 100.0, 50.0);
    let alt = Mods { alt: true, ..Default::default() };
    let ui = gesture(&mut s, "rotate", &[(10.0, 20.0), (10.0, 20.0)], alt);
    assert_eq!(ui, vec![UiRequest::Dialog("rotate".into(), json!({"angle": 0, "origin": [10.0, 20.0]}))]);
}

#[test]
fn scale_tool_click_origin_then_drag() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    gesture(&mut s, "scale", &[(100.0, 100.0), (100.0, 100.0)], Mods::default());
    gesture(&mut s, "scale", &[(200.0, 200.0), (250.0, 250.0), (300.0, 300.0), (300.0, 300.0)], Mods::default());
    assert!(close(bounds(&s, a), Rect::new(100.0, 100.0, 300.0, 300.0)), "{:?}", bounds(&s, a));
}

#[test]
fn reflect_and_shear_tools() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 50.0);
    // Reflect across a vertical axis through the centre (150,125): bounds unchanged, geometry mirrored.
    gesture(&mut s, "reflect", &[(150.0, 50.0), (150.0, 40.0), (150.0, 20.0), (150.0, 20.0)], Mods::default());
    assert!(close(bounds(&s, a), Rect::new(100.0, 100.0, 200.0, 150.0)));
    // Shear horizontally: top edge shifts right by 25 relative to the centre row.
    gesture(&mut s, "shear", &[(150.0, 100.0), (160.0, 100.0), (175.0, 100.0), (175.0, 100.0)], Mods::default());
    let b = bounds(&s, a);
    assert!((b.width() - 150.0).abs() < 1e-6, "{b:?}");
}

/// Two clicks with the Reflect tool reflect the selection across the line through both points as
/// one undo step; with Alt on the second click, a copy is reflected and Undo removes it.
#[test]
fn two_reflect_clicks_reflect_across_their_line_as_one_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 50.0);
    s.select_tool("reflect", ViewInfo::default()).unwrap();
    let click = |s: &mut Session, x: f64, y: f64, alt: bool| {
        drag_at(s, PointerKind::Down, (x, y), alt);
        drag_at(s, PointerKind::Up, (x, y), alt);
    };
    let n = undo_len(&s);
    click(&mut s, 300.0, 50.0, false);
    assert_eq!(undo_len(&s), n);
    assert!(close(bounds(&s, a), Rect::new(100.0, 100.0, 200.0, 150.0)));
    click(&mut s, 300.0, 250.0, false);
    assert!(close(bounds(&s, a), Rect::new(400.0, 100.0, 500.0, 150.0)), "{:?}", bounds(&s, a));
    assert_eq!(undo_len(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(close(bounds(&s, a), Rect::new(100.0, 100.0, 200.0, 150.0)));

    let children = |s: &Session| s.doc().unwrap().doc.layers[0].children().unwrap().len();
    click(&mut s, 50.0, 300.0, false);
    click(&mut s, 350.0, 300.0, true);
    assert_eq!(children(&s), 2);
    assert!(close(bounds(&s, a), Rect::new(100.0, 100.0, 200.0, 150.0)));
    // The copy is selected and reflected across y = 300.
    let copy = s.doc().unwrap().selection.objects.clone();
    assert!(copy.len() == 1 && copy[0] != a, "{copy:?}");
    assert!(close(bounds(&s, copy[0]), Rect::new(100.0, 450.0, 200.0, 500.0)), "{:?}", bounds(&s, copy[0]));
    assert_eq!(undo_len(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(children(&s), 1);
}

/// Reflect and Shear return an error for an `axis` they don't take, naming it (both also take an
/// angle, Shear's from -359° to 359°): nothing moves and no undo step is recorded. A null `axis` counts as not given: Reflect
/// then reflects across the vertical axis and Shear shears along the horizontal one.
#[test]
fn reflect_and_shear_reject_unknown_axes_without_changing_state() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 20.0, 30.0, 10.0);
    let n = undo_len(&s);
    let reflect = "axis must be \"vertical\", \"horizontal\" or an angle in degrees";
    let shear = "axis must be \"horizontal\", \"vertical\" or an angle in degrees from -359 to 359";
    for (cmd, p, msg) in [
        ("object.reflect", json!({"axis": "diagonal", "origin": [0, 0]}), format!("{reflect}, not `diagonal`")),
        ("object.reflect", json!({"axis": "45", "origin": [0, 0]}), format!("{reflect}, not `45`")),
        ("object.reflect", json!({"axis": true, "origin": [0, 0]}), format!("{reflect}, not `true`")),
        ("object.shear", json!({"angle": 30, "axis": "diagonal"}), format!("{shear}, not `diagonal`")),
        ("object.shear", json!({"angle": 30, "axis": "90"}), format!("{shear}, not `90`")),
        ("object.shear", json!({"angle": 30, "axis": 360}), format!("{shear}, not `360`")),
        ("object.shear", json!({"angle": 30, "axis": -359.5}), format!("{shear}, not `-359.5`")),
        ("object.shear", json!({"angle": 30, "axis": [45]}), format!("{shear}, not `[45]`")),
        ("object.shear", json!({"angle": 30, "axis": false}), format!("{shear}, not `false`")),
    ] {
        let e = s.execute(cmd, &p).expect_err(&format!("{cmd} {p}"));
        assert_eq!(e.to_string(), format!("invalid parameters for `{cmd}`: {msg}"));
    }
    assert!(close(bounds(&s, a), Rect::new(10.0, 20.0, 40.0, 30.0)), "nothing moved: {:?}", bounds(&s, a));
    assert_eq!(undo_len(&s), n, "no undo step");
    s.execute("object.reflect", &json!({"axis": null, "origin": [0, 0]})).unwrap();
    assert!(close(bounds(&s, a), Rect::new(-40.0, 20.0, -10.0, 30.0)), "across the vertical axis: {:?}", bounds(&s, a));
    s.execute("object.reflect", &json!({"axis": 0, "origin": [0, 0]})).unwrap();
    assert!(close(bounds(&s, a), Rect::new(-40.0, -30.0, -10.0, -20.0)), "an angle still reflects: {:?}", bounds(&s, a));
    s.execute("object.shear", &json!({"angle": 45, "axis": null, "origin": [0, 0]})).unwrap();
    let b = bounds(&s, a);
    assert!((b.y0 + 30.0).abs() < 1e-6 && (b.y1 + 20.0).abs() < 1e-6 && b.width() > 30.0, "along the horizontal axis: {b:?}");
}

/// Shear takes an angle from -359 to 359 degrees and shears by its tangent: 60 and -60 slant
/// opposite ways, and 120 and 300 slant as -60 does.
#[test]
fn shear_angles_slant_by_their_tangent() {
    let run = 100.0 * 60f64.to_radians().tan();
    for (angle, want) in [
        (60.0, Rect::new(-run, 0.0, 100.0, 100.0)),
        (-60.0, Rect::new(0.0, 0.0, 100.0 + run, 100.0)),
        (120.0, Rect::new(0.0, 0.0, 100.0 + run, 100.0)),
        (300.0, Rect::new(0.0, 0.0, 100.0 + run, 100.0)),
        (-359.0, Rect::new(-100.0 * 1f64.to_radians().tan(), 0.0, 100.0, 100.0)),
    ] {
        let mut s = session();
        let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
        s.execute("object.shear", &json!({"angle": angle, "origin": [0, 0]})).unwrap();
        assert!(close(bounds(&s, a), want), "{angle}: {:?}, want {want:?}", bounds(&s, a));
    }
}

/// Shear returns an error for an angle outside -359 to 359 and for ±90 and ±270, which have no
/// finite shear (within a billionth of a degree too); nothing moves and no undo step is recorded.
#[test]
fn shear_rejects_angles_with_no_shear_or_out_of_range() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 20.0, 30.0, 10.0);
    let n = undo_len(&s);
    let none = |v: f64| format!("invalid parameters for `object.shear`: `angle` cannot be {v}: ±90 and ±270 degrees have no shear");
    let range = |v: f64| format!("invalid parameters for `object.shear`: `angle` must be from -359 to 359 degrees, not {v}");
    for (angle, msg) in [
        (90.0, none(90.0)),
        (-90.0, none(-90.0)),
        (270.0, none(270.0)),
        (-270.0, none(-270.0)),
        (90.0 + 1e-12, none(90.0 + 1e-12)),
        (360.0, range(360.0)),
        (-359.5, range(-359.5)),
        (1e300, range(1e300)),
    ] {
        let e = s.execute("object.shear", &json!({"angle": angle})).expect_err(&angle.to_string());
        assert_eq!(e.to_string(), msg);
    }
    assert!(s.execute("object.shear", &json!({"angle": "45"})).is_err(), "a string is not an angle");
    assert!(close(bounds(&s, a), Rect::new(10.0, 20.0, 40.0, 30.0)), "nothing moved: {:?}", bounds(&s, a));
    assert_eq!(undo_len(&s), n, "no undo step");
}

/// Shear by 0, 180 or -180 degrees, along either axis, has no slant: the art keeps its exact
/// coordinates and no undo step is recorded.
#[test]
fn shear_by_0_and_180_changes_nothing() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 20.0, 30.0, 10.0);
    let before = bounds(&s, a);
    let n = undo_len(&s);
    for angle in [180.0, -180.0, 0.0, 180.0 + 1e-12, -180.0 - 1e-12] {
        for axis in ["horizontal", "vertical"] {
            s.execute("object.shear", &json!({"angle": angle, "axis": axis})).unwrap();
            assert_eq!(bounds(&s, a), before, "{angle} {axis}");
            assert_eq!(undo_len(&s), n, "{angle} {axis}: no undo step");
        }
    }
}

/// Shear along an axis at an angle, counter-clockwise from horizontal on the page: each point
/// slides along the axis by the tangent of the shear angle times its distance across the axis
/// from the origin. -359° and 359° are taken, the shear is one undo step and Transform Again
/// repeats it; 0° and 90° shear as the named axes do, and a copy leaves the original in place.
#[test]
fn shear_along_an_axis_at_an_angle() {
    let anchors = |s: &Session, id: NodeId| -> Vec<Point> {
        s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().anchors().map(|(_, _, an)| an.p).collect()
    };
    // The axis's direction and the direction a quarter turn counter-clockwise from it, with y down.
    let sheared = |p: Point, o: Point, axis: f64, angle: f64| {
        let (a, k) = (axis.to_radians(), angle.to_radians().tan());
        let (along, across) = (Vec2::new(a.cos(), -a.sin()), Vec2::new(-a.sin(), -a.cos()));
        p + along * (k * (p - o).dot(across))
    };
    let near = |a: &[Point], b: &[Point]| a.len() == b.len() && a.iter().zip(b).all(|(p, q)| p.distance(*q) < 1e-9);
    let o = Point::new(20.0, 30.0);
    for (axis, angle) in [(45.0, 30.0), (-120.5, 20.0), (359.0, 30.0), (-359.0, -30.0), (30.0, 30.0)] {
        let mut s = session();
        let a = rect(&mut s, 100.0, 100.0, 60.0, 20.0);
        let before = anchors(&s, a);
        let n = undo_len(&s);
        s.execute("object.shear", &json!({"angle": angle, "axis": axis, "origin": [o.x, o.y]})).unwrap();
        let want: Vec<Point> = before.iter().map(|p| sheared(*p, o, axis, angle)).collect();
        assert!(near(&anchors(&s, a), &want), "{axis}° {angle}°: {:?}, not {want:?}", anchors(&s, a));
        assert_eq!(undo_len(&s), n + 1, "one undo step");
        s.execute("object.transformAgain", &json!({})).unwrap();
        let again: Vec<Point> = want.iter().map(|p| sheared(*p, o, axis, angle)).collect();
        assert!(near(&anchors(&s, a), &again), "Transform Again at {axis}°: {:?}", anchors(&s, a));
    }
    for (named, axis) in [("horizontal", 0.0), ("vertical", 90.0), ("vertical", -270.0)] {
        let mut s = session();
        let a = rect(&mut s, 100.0, 100.0, 60.0, 20.0);
        s.execute("object.shear", &json!({"angle": 30, "axis": named, "origin": [o.x, o.y]})).unwrap();
        let by_name = anchors(&s, a);
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("object.shear", &json!({"angle": 30, "axis": axis, "origin": [o.x, o.y]})).unwrap();
        assert!(near(&anchors(&s, a), &by_name), "{axis}° shears as {named}: {:?}, not {by_name:?}", anchors(&s, a));
    }
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 60.0, 20.0);
    let before = anchors(&s, a);
    s.execute("object.shear", &json!({"angle": 30, "axis": 45, "origin": [o.x, o.y], "copy": true})).unwrap();
    assert!(near(&anchors(&s, a), &before), "the original stays: {:?}", anchors(&s, a));
    let copy = *s.doc().unwrap().selection.objects.first().unwrap();
    assert_ne!(copy, a, "the copy is selected");
    let want: Vec<Point> = before.iter().map(|p| sheared(*p, o, 45.0, 30.0)).collect();
    assert!(near(&anchors(&s, copy), &want), "the copy is sheared: {:?}", anchors(&s, copy));
}

#[test]
fn distort_command_maps_corners() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("object.distort", &json!({"corners": [[10, 0], [90, 0], [100, 100], [0, 100]]})).unwrap();
    let d = &s.doc().unwrap().doc;
    let pts: Vec<Point> = d.node(a).unwrap().path_data().unwrap().anchors().map(|(_, _, an)| an.p).collect();
    for want in [Point::new(10.0, 0.0), Point::new(90.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)] {
        assert!(pts.iter().any(|p| p.distance(want) < 1e-6), "{pts:?} missing {want:?}");
    }
    assert!(s.execute("object.distort", &json!({"corners": [[0, 0]]})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(close(bounds(&s, a), Rect::new(0.0, 0.0, 100.0, 100.0)));
}

/// Free Distort warps the art of a linked opacity mask with the object it masks, and one undo puts
/// both back; an unlinked mask stays where it is.
#[test]
fn distort_command_warps_a_linked_opacity_mask_with_its_object() {
    for linked in [true, false] {
        let mut s = session();
        let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
        let b = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
        s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
        s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
        if !linked {
            s.execute("transparency.unlinkOpacityMask", &json!({})).unwrap();
        }
        let mask_path = |s: &Session| s.doc().unwrap().doc.node(a).unwrap().extras().mask.as_ref().unwrap().art.path_data().cloned();
        let path = |s: &Session| s.doc().unwrap().doc.node(a).unwrap().path_data().cloned();
        let before = (path(&s), mask_path(&s));
        s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
        s.execute("object.distort", &json!({"corners": [[10, 0], [90, 0], [100, 100], [0, 100]]})).unwrap();
        assert!(path(&s) != before.0);
        let want = if linked { path(&s) } else { before.1.clone() };
        assert_eq!(mask_path(&s), want, "linked: {linked}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!((path(&s), mask_path(&s)), before);
    }
}

/// Free Distort of a group warps the linked opacity mask of a path inside it with the path.
#[test]
fn distort_command_warps_the_linked_mask_of_a_path_in_a_group() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    let c = rect(&mut s, 150.0, 0.0, 50.0, 50.0);
    s.execute("select.set", &json!({"ids": [a.0, c.0]})).unwrap();
    let g = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    let before = s.doc().unwrap().doc.node(a).unwrap().path_data().cloned();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    s.execute("object.distort", &json!({"corners": [[10, 0], [190, 0], [200, 100], [0, 100]]})).unwrap();
    let a = s.doc().unwrap().doc.node(a).unwrap();
    assert!(a.path_data().cloned() != before, "the path is warped");
    assert_eq!(a.extras().mask.as_ref().unwrap().art.path_data(), a.path_data(), "and so is its mask");
}

/// Corners where the box already stands change nothing: no undo step, the document as it was. A
/// corner moved by a hundredth of a point is a warp.
#[test]
fn unchanged_distort_records_no_history() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    let b = s.doc().unwrap().doc.bounds_of(&[a], false).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let n = undo_len(&s);
    let at = |x1: f64| json!([[b.x0, b.y0], [x1, b.y0], [b.x1, b.y1], [b.x0, b.y1]]);
    assert_eq!(s.execute("object.distort", &json!({"corners": at(b.x1)})).unwrap()["ids"], json!([a.0]));
    assert_eq!(undo_len(&s), n);
    assert!(s.doc().unwrap().doc == doc, "the document is unchanged");
    s.execute("object.distort", &json!({"corners": at(b.x1 + 0.01)})).unwrap();
    assert_eq!(undo_len(&s), n + 1);
    assert!(s.doc().unwrap().doc != doc);
}

#[test]
fn distort_perspective_midpoint_is_projective() {
    // Distort a tiny probe at the centre of a 100×100 source box into a trapezoid: with a real
    // perspective warp the centre lands where the quad's diagonals cross (y = 100/6), not at y = 50.
    let mut s = session();
    let probe = rect(&mut s, 49.999, 49.999, 0.002, 0.002);
    s.execute("object.distort", &json!({"corners": [[40, 0], [60, 0], [100, 100], [0, 100]], "from": [0, 0, 100, 100], "ids": [probe.0]})).unwrap();
    let c = bounds(&s, probe).center();
    assert!((c.x - 50.0).abs() < 1e-6);
    assert!((c.y - 100.0 / 6.0).abs() < 1e-3, "{c:?}");
}

/// A 2 × 2 px image placed at (0, 0)–(100, 100), selected.
fn placed_image(s: &mut Session) -> NodeId {
    s.edit("Place", |d, sel| {
        d.images.insert("pic".into(), vectorcraft_doc::ImageBlob::png(vec![]));
        let id = d.alloc_id();
        let im = vectorcraft_doc::ImageObject {
            key: "pic".into(),
            width: 2,
            height: 2,
            xf: vectorcraft_geom::Affine::scale(50.0),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        d.insert(d.default_layer(), 0, Node::new(id, vectorcraft_doc::NodeKind::Image(Box::new(im))))?;
        sel.set([id]);
        Ok(id)
    })
    .unwrap()
}

fn corners(s: &Session, id: NodeId) -> [Point; 4] {
    s.doc().unwrap().doc.node(id).unwrap().image_corners().unwrap()
}

fn assert_corners(got: [Point; 4], want: [(f64, f64); 4]) {
    for (g, (x, y)) in got.iter().zip(want) {
        assert!(g.distance(Point::new(x, y)) < 1e-9, "{got:?} against {want:?}");
    }
}

/// Perspective Distort and Free Distort put an image's corners exactly on the handles, in one
/// undo step; the image is bounded and clicked by its corners, and a move takes them along.
#[test]
fn distort_puts_an_images_corners_on_the_handles() {
    for want in [[(40.0, 0.0), (60.0, 0.0), (100.0, 100.0), (0.0, 100.0)], [(10.0, 5.0), (90.0, 0.0), (120.0, 110.0), (0.0, 100.0)]] {
        let mut s = session();
        let img = placed_image(&mut s);
        let undo = undo_len(&s);
        let corners_json: Vec<[f64; 2]> = want.iter().map(|(x, y)| [*x, *y]).collect();
        s.execute("object.distort", &json!({ "corners": corners_json })).unwrap();
        assert_eq!(undo_len(&s), undo + 1);
        assert_corners(corners(&s, img), want);
        assert!(s.doc().unwrap().doc.node(img).unwrap().is_distorted_image());
        let b = bounds(&s, img);
        let xs = want.map(|c| c.0);
        let ys = want.map(|c| c.1);
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(near(b.x0, xs.iter().copied().fold(f64::MAX, f64::min)) && near(b.x1, xs.iter().copied().fold(f64::MIN, f64::max)), "{b:?}");
        assert!(near(b.y0, ys.iter().copied().fold(f64::MAX, f64::min)) && near(b.y1, ys.iter().copied().fold(f64::MIN, f64::max)), "{b:?}");
        s.execute("object.move", &json!({"dx": 10, "dy": 20})).unwrap();
        assert_corners(corners(&s, img), want.map(|(x, y)| (x + 10.0, y + 20.0)));
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        assert_corners(corners(&s, img), [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        assert!(!s.doc().unwrap().doc.node(img).unwrap().is_distorted_image());
    }
    // Clicks: inside the trapezoid hits the image; inside its old square but outside the
    // trapezoid does not.
    let mut s = session();
    let img = placed_image(&mut s);
    s.execute("object.distort", &json!({ "corners": [[40, 0], [60, 0], [100, 100], [0, 100]] })).unwrap();
    let doc = &s.doc().unwrap().doc;
    let opt = vectorcraft_doc::hit::HitOptions::default();
    assert_eq!(vectorcraft_doc::hit::hit_test(doc, Point::new(50.0, 50.0), opt).map(|h| h.leaf), Some(img));
    assert_eq!(vectorcraft_doc::hit::hit_test(doc, Point::new(5.0, 10.0), opt).map(|h| h.leaf), None);
}

/// Distorted together with a path, the image's corners and the path's anchors follow one warp.
#[test]
fn distort_moves_an_image_and_a_path_as_one() {
    let mut s = session();
    let img = placed_image(&mut s);
    let path = rect(&mut s, 100.0, 0.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [img.0, path.0]})).unwrap();
    s.execute("object.distort", &json!({"corners": [[0, 0], [200, 0], [200, 100], [50, 100]]})).unwrap();
    let d = &s.doc().unwrap().doc;
    let anchors: Vec<Point> = d.node(path).unwrap().path_data().unwrap().anchors().map(|(_, _, a)| a.p).collect();
    let q = corners(&s, img);
    // The image's right edge and the path's left edge were one line, and stay one where the warp
    // moves it.
    assert!(q[2].distance(Point::new(100.0, 100.0)) > 1.0, "{q:?}");
    for p in [q[1], q[2]] {
        assert!(anchors.iter().any(|a| a.distance(p) < 1e-9), "{p:?} not in {anchors:?}");
    }
}

/// Corners that would fold an image over are refused: nothing changes and no undo step is kept.
/// A distorted image keeps its distortion where commands that cannot keep it are asked to work.
#[test]
fn distort_refuses_a_folded_image_and_crop_and_trace_refuse_a_distorted_one() {
    let mut s = session();
    let img = placed_image(&mut s);
    let (before, undo) = (s.doc().unwrap().doc.clone(), undo_len(&s));
    let e = s.execute("object.distort", &json!({"corners": [[0, 0], [100, 0], [0, 100], [100, 100]]})).unwrap_err().to_string();
    assert!(e.contains("convex"), "{e}");
    assert_eq!(undo_len(&s), undo);
    assert_eq!(s.doc().unwrap().doc, before);
    // Three corners on one line give no perspective map at all.
    let e = s.execute("object.distort", &json!({"corners": [[0, 0], [200, 0], [100, 50], [0, 100]]})).unwrap_err().to_string();
    assert!(e.contains("convex"), "{e}");
    assert_eq!(s.doc().unwrap().doc, before);
    // A convex shape too small for a perspective map: the corners could not reach the handles.
    let e = s.execute("object.distort", &json!({"corners": [[0, 0], [1e-7, 0], [8e-8, 1e-7], [0, 1e-7]]})).unwrap_err().to_string();
    assert!(e.contains("too close together"), "{e}");
    // The same shape far from the origin: its distance from it does not hide the missed corner.
    let x = 1_000_000.0;
    let e = s.execute("object.distort", &json!({"corners": [[x, 0], [x + 1e-7, 0], [x + 8e-8, 1e-7], [x, 1e-7]]})).unwrap_err().to_string();
    assert!(e.contains("too close together"), "{e}");
    // A long, thin shape with one side all but gone.
    let e = s.execute("object.distort", &json!({"corners": [[0, 0], [1_000_000, 0], [1_000_000, 1e-19], [0, 1]]})).unwrap_err().to_string();
    assert!(e.contains("too close together"), "{e}");
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(undo_len(&s), undo);
    // A small shape near the edge of the canvas is still distorted.
    let o = vectorcraft_doc::MAX_COORD - 2.0;
    let far = [[o, o], [o + 1.0, o], [o + 0.2130712, o + 1.0], [o, o + 1.0]];
    s.execute("object.distort", &json!({ "from": [0, 0, 100, 100], "corners": far })).unwrap();
    assert!(s.doc().unwrap().doc.node(img).unwrap().is_distorted_image());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(undo_len(&s), undo);
    s.execute("object.distort", &json!({ "corners": [[40, 0], [60, 0], [100, 100], [0, 100]] })).unwrap();
    for (cmd, params) in [
        ("object.cropImage", json!({})),
        ("imageTrace.make", json!({})),
        ("links.relink", json!({"path": "/nowhere/pic.png"})),
        // Refused before the folder is searched, so a file missing there does not hide it.
        ("links.relink", json!({"folder": "/nowhere"})),
        ("links.embed", json!({"ids": [img.0]})),
        ("links.unembed", json!({})),
        // Refused before any file is written.
        ("links.unembed", json!({"folder": "/nowhere"})),
        ("object.createObjectMosaic", json!({})),
        ("object.mesh.create", json!({})),
        (
            "file.place",
            json!({"replace": true, "name": "b.png", "dataBase64": vectorcraft_format::base64_encode(&crate::tests_links::png(2, 2, [0, 0, 255]))}),
        ),
    ] {
        let e = s.execute(cmd, &params).unwrap_err().to_string();
        // Every command gives the one shared refusal, which the complete-message catalogs translate.
        assert!(e.contains("is an image changed by Perspective Distort or Free Distort, which this command does not work on"), "{cmd}: {e}");
    }
    assert!(s.doc().unwrap().doc.node(img).unwrap().is_distorted_image());
    assert_eq!(undo_len(&s), undo + 1);
    // Type replaces a distorted image by its bounds, which the corners give: not refused.
    let text = json!({"replace": true, "name": "note.txt", "dataBase64": vectorcraft_format::base64_encode(b"Hello")});
    let out = s.execute("file.place", &text).unwrap();
    let id = NodeId(out["ids"][0].as_u64().unwrap());
    assert!(matches!(s.doc().unwrap().doc.node(id).unwrap().kind, vectorcraft_doc::NodeKind::Text(_)));
}

/// Create Gradient Mesh on a path and a distorted image converts the path and names the image it
/// left as it is, with the shared refusal.
#[test]
fn mesh_create_reports_the_distorted_image_it_skips() {
    let mut s = session();
    let img = placed_image(&mut s);
    s.execute("object.distort", &json!({ "corners": [[40, 0], [60, 0], [100, 100], [0, 100]] })).unwrap();
    let path = rect(&mut s, 200.0, 0.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [img.0, path.0]})).unwrap();
    let out = s.execute("object.mesh.create", &json!({"rows": 1, "cols": 1})).unwrap();
    assert_eq!(out["ids"].as_array().map(Vec::len), Some(1), "{out}");
    let skipped = out["skipped"].as_array().expect("the skipped image");
    assert_eq!(skipped.len(), 1, "{out}");
    assert_eq!(skipped[0]["id"], json!(img.0));
    assert!(skipped[0]["reason"].as_str().unwrap().contains("Perspective Distort or Free Distort"), "{out}");
    assert!(s.doc().unwrap().doc.node(img).unwrap().is_distorted_image());
    // Without a distorted image, no `skipped`.
    let other = rect(&mut s, 400.0, 0.0, 100.0, 100.0);
    let out = s.execute("object.mesh.create", &json!({"ids": [other.0], "rows": 1, "cols": 1})).unwrap();
    assert!(out.get("skipped").is_none(), "{out}");
}

/// A blend takes the warp's affine approximation whole, so an image inside it does not stop a
/// distort whose corners an image could not follow.
#[test]
fn distort_of_a_blend_holding_an_image_is_not_refused() {
    let mut s = session();
    let img = placed_image(&mut s);
    let r = rect(&mut s, 200.0, 0.0, 100.0, 100.0);
    let out = s.execute("object.blend.make", &json!({"ids": [img.0, r.0]})).unwrap();
    let blend = NodeId(out["id"].as_u64().unwrap());
    let undo = undo_len(&s);
    s.execute("object.distort", &json!({"ids": [blend.0], "corners": [[0, 0], [300, 0], [100, 50], [0, 100]]})).unwrap();
    assert_eq!(undo_len(&s), undo + 1);
    assert!(!s.doc().unwrap().doc.node(img).unwrap().is_distorted_image());
}

/// The Control bar's Mask clips a distorted image to the shape between its corners.
#[test]
fn mask_on_a_distorted_image_follows_its_corners() {
    let mut s = session();
    let img = placed_image(&mut s);
    let want = [(40.0, 0.0), (60.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
    s.execute("object.distort", &json!({ "corners": [[40, 0], [60, 0], [100, 100], [0, 100]] })).unwrap();
    let out = s.execute("object.maskImage", &json!({})).unwrap();
    let path = NodeId(out["path"].as_u64().unwrap());
    let d = &s.doc().unwrap().doc;
    let anchors: Vec<Point> = d.node(path).unwrap().path_data().unwrap().anchors().map(|(_, _, a)| a.p).collect();
    assert_eq!(anchors.len(), 4, "{anchors:?}");
    for (x, y) in want {
        assert!(anchors.iter().any(|a| a.distance(Point::new(x, y)) < 1e-9), "{x},{y} not in {anchors:?}");
    }
    assert!(d.node(img).unwrap().is_distorted_image());
}

/// A Transform effect moves a distorted image's corners with it: expanded, the image stands
/// where the effect drew it.
#[test]
fn a_transform_effect_moves_a_distorted_images_corners() {
    let mut s = session();
    let img = placed_image(&mut s);
    s.execute("object.distort", &json!({ "corners": [[40, 0], [60, 0], [100, 100], [0, 100]] })).unwrap();
    s.execute("effect.apply", &json!({"ids": [img.0], "effect": "distort.transform", "params": {"moveH": 200}})).unwrap();
    s.execute("select.set", &json!({"ids": [img.0]})).unwrap();
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    let mut found = vec![];
    for layer in &s.doc().unwrap().doc.layers {
        layer.walk(&mut |n| {
            if let Some(q) = n.image_corners() {
                found.push(q);
            }
        });
    }
    let want = [(240.0, 0.0), (260.0, 0.0), (300.0, 100.0), (200.0, 100.0)];
    assert!(found.iter().any(|q| q.iter().zip(want).all(|(p, (x, y))| p.distance(Point::new(x, y)) < 1e-6)), "{found:?}");
}

/// A Transform effect after a Warp effect moves a distorted image's warped pixels once: the art
/// stands where the Warp alone draws it, moved by the Transform.
#[test]
fn a_transform_after_a_warp_moves_a_distorted_image_once() {
    let expanded_bounds = |transform: bool| {
        let mut s = session();
        let img = placed_image(&mut s);
        s.execute("object.distort", &json!({ "corners": [[40, 0], [60, 0], [100, 100], [0, 100]] })).unwrap();
        s.execute("effect.apply", &json!({"ids": [img.0], "effect": "warp.bulge", "params": {"bend": 50}})).unwrap();
        if transform {
            s.execute("effect.apply", &json!({"ids": [img.0], "effect": "distort.transform", "params": {"moveH": 200}})).unwrap();
        }
        s.execute("select.set", &json!({"ids": [img.0]})).unwrap();
        s.execute("effect.expandAppearance", &json!({})).unwrap();
        let mut images = None;
        for layer in &s.doc().unwrap().doc.layers {
            layer.walk(&mut |n| {
                if let vectorcraft_doc::NodeKind::Image(im) = &n.kind {
                    let r = im.xf.transform_rect_bbox(vectorcraft_geom::Rect::new(0.0, 0.0, f64::from(im.width), f64::from(im.height)));
                    images = Some(images.map_or(r, |b: vectorcraft_geom::Rect| b.union(r)));
                }
            });
        }
        images.expect("the warped pieces")
    };
    let (alone, moved) = (expanded_bounds(false), expanded_bounds(true));
    assert!((moved.x0 - alone.x0 - 200.0).abs() < 1e-6 && (moved.x1 - alone.x1 - 200.0).abs() < 1e-6, "{alone:?} {moved:?}");
    assert!((moved.y0 - alone.y0).abs() < 1e-6 && (moved.y1 - alone.y1).abs() < 1e-6, "{alone:?} {moved:?}");
}

/// A raster effect's baked pixels already show the distortion: the image that replaces a
/// distorted one is placed by its own transform.
#[test]
fn baked_raster_effect_pixels_replace_a_distortion() {
    let mut s = session();
    let img = placed_image(&mut s);
    s.execute("object.distort", &json!({ "corners": [[40, 0], [60, 0], [100, 100], [0, 100]] })).unwrap();
    let v = s.doc().unwrap().doc.node(img).unwrap().clone();
    let baked = Node::new(NodeId(99), v.kind.clone());
    let out = crate::cmd::rasterfx::replace_with_image(v, baked);
    assert!(!out.is_distorted_image());
    assert!(out.extras().image_quad.is_none());
}

/// A distorted image is saved with its corners and reads back the same.
#[test]
fn a_distorted_image_saves_and_reads_back() {
    let mut s = session();
    let img = placed_image(&mut s);
    s.execute("object.distort", &json!({ "corners": [[40, 0], [60, 0], [100, 100], [0, 100]] })).unwrap();
    let doc = &s.doc().unwrap().doc;
    let bytes = vectorcraft_format::save(doc, false);
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert_eq!(back.node(img).unwrap().image_corners(), doc.node(img).unwrap().image_corners());
    assert!(back.node(img).unwrap().is_distorted_image());
    assert!(String::from_utf8_lossy(&bytes).contains("image_quad"));
    // An image without a distortion is saved without the field, and opens as it was.
    let mut plain = session();
    let img = placed_image(&mut plain);
    let doc = &plain.doc().unwrap().doc;
    let bytes = vectorcraft_format::save(doc, false);
    assert!(!String::from_utf8_lossy(&bytes).contains("image_quad"));
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert_eq!(back.node(img), doc.node(img));
    // Corners in a file that fold the image over leave it placed by its transform.
    let mut bad = (**doc).clone();
    bad.node_mut(img).unwrap().extras_mut().image_quad =
        Some(Box::new([(0.0, 0.0), (100.0, 0.0), (0.0, 100.0), (100.0, 100.0)].map(|(x, y)| Point::new(x, y))));
    let back = vectorcraft_format::load(&vectorcraft_format::save(&bad, false)).unwrap();
    assert!(!back.node(img).unwrap().is_distorted_image());
    assert_corners(back.node(img).unwrap().image_corners().unwrap(), [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
}

/// A distort whose corners cross the warp's horizon at the image is refused with that reason, and
/// a distort that leaves a distorted image where it is records nothing.
#[test]
fn distort_names_the_horizon_and_an_unchanged_distort_of_a_distorted_image_records_nothing() {
    let mut s = session();
    let img = placed_image(&mut s);
    let (before, undo) = (s.doc().unwrap().doc.clone(), undo_len(&s));
    // The box (0, 0)-(10, 10) widens toward its bottom so fast that the warp's horizon lies at
    // y = 15, inside the image.
    let e = s.execute("object.distort", &json!({"from": [0, 0, 10, 10], "corners": [[0, 0], [10, 0], [20, 10], [-10, 10]]})).unwrap_err().to_string();
    assert!(e.contains("horizon"), "{e}");
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(undo_len(&s), undo);
    s.execute("object.distort", &json!({ "corners": [[40, 0], [60, 0], [100, 100], [0, 100]] })).unwrap();
    let (before, undo) = (s.doc().unwrap().doc.clone(), undo_len(&s));
    let b = bounds(&s, img);
    let at = [[b.x0, b.y0], [b.x1, b.y0], [b.x1, b.y1], [b.x0, b.y1]];
    s.execute("object.distort", &json!({ "from": [b.x0, b.y0, b.x1, b.y1], "corners": at })).unwrap();
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(undo_len(&s), undo);
}

/// Image info gives a distorted image's resolution at its middle, not the resolution it had
/// before the distort.
#[test]
fn image_info_measures_a_distorted_image_at_its_middle() {
    let mut s = session();
    placed_image(&mut s);
    // 2 pixels across 100 pt: 1.44 ppi.
    let ppi = |s: &mut Session| {
        let out = s.execute("image.info", &json!({})).unwrap();
        [out["ppi"][0].as_f64().unwrap(), out["ppi"][1].as_f64().unwrap()]
    };
    assert!((ppi(&mut s)[0] - 1.44).abs() < 1e-9);
    // The top squeezed to half the width: the middle of the pixels spans less than 100 pt across,
    // so the resolution across there is higher.
    s.execute("object.distort", &json!({ "corners": [[25, 0], [75, 0], [100, 100], [0, 100]] })).unwrap();
    let [x, _] = ppi(&mut s);
    assert!(x > 1.44 * 1.1, "{x}");
}

/// Rasterize draws a distorted image's pixels between its corners, and the image it makes is a
/// plain one.
#[test]
fn rasterize_draws_a_distorted_image_between_its_corners() {
    let mut s = session();
    let img = placed_image(&mut s);
    let mut png = std::io::Cursor::new(vec![]);
    image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255])).write_to(&mut png, image::ImageFormat::Png).unwrap();
    s.edit("Pixels", |d, _| {
        d.images.insert("pic".into(), vectorcraft_doc::ImageBlob::png(png.into_inner()));
        Ok(())
    })
    .unwrap();
    s.execute("object.distort", &json!({ "corners": [[40, 0], [60, 0], [100, 100], [0, 100]] })).unwrap();
    s.execute("select.set", &json!({ "ids": [img.0] })).unwrap();
    let out = s.execute("object.rasterize", &json!({ "ppi": 72, "background": "transparent", "addAround": 0 })).unwrap();
    let d = &s.doc().unwrap().doc;
    let id = NodeId(out["id"].as_u64().unwrap());
    let n = d.node(id).unwrap();
    assert!(!n.is_distorted_image());
    let vectorcraft_doc::NodeKind::Image(im) = &n.kind else { panic!("an image") };
    // The raster covers the corners' bounds.
    let b = n.geometric_bounds().unwrap();
    assert!([b.x0, b.y0, b.x1 - 100.0, b.y1 - 100.0].iter().all(|v| v.abs() <= 1.0), "{b:?}");
    let px = craft_image_codecs::decode(&d.images[&im.key].bytes).unwrap();
    let (w, h) = px.dimensions();
    let rgba = px.to_rgba8();
    let alpha = |x: f64, y: f64| {
        let p = im.xf.inverse() * Point::new(x, y);
        assert!(p.x >= 0.0 && p.y >= 0.0 && p.x < f64::from(w) && p.y < f64::from(h), "({x}, {y}) is outside the raster: {p:?}");
        let (i, j) = (p.x as u32, p.y as u32);
        rgba[((j * w + i) * 4 + 3) as usize]
    };
    assert_eq!(alpha(50.0, 90.0), 255, "inside the corners");
    assert_eq!(alpha(10.0, 10.0), 0, "inside the old rectangle, outside the corners");
}

/// A Free Transform drag in Free Distort mode on an image puts the dragged corner where the drag
/// ends, in one undo step, also when a position on the way would have folded the image.
#[test]
fn free_transform_distort_drag_on_an_image_is_one_undo_step() {
    let mut s = session();
    let img = placed_image(&mut s);
    let undo = undo_len(&s);
    let v = ViewInfo::default();
    s.select_tool("freeTransform", v).unwrap();
    s.set_tool_option("mode", &json!("distort"));
    s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 100.0), v).unwrap();
    // Past the top left corner: the image would fold over.
    s.pointer(&PointerEvent::new(PointerKind::Drag, -50.0, -50.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 130.0, 120.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 130.0, 120.0), v).unwrap();
    assert_corners(corners(&s, img), [(0.0, 0.0), (100.0, 0.0), (130.0, 120.0), (0.0, 100.0)]);
    assert!(s.doc().unwrap().doc.node(img).unwrap().is_distorted_image());
    assert_eq!(undo_len(&s), undo + 1);
}

#[test]
fn free_transform_distort_mode_via_pointer() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let v = ViewInfo::default();
    s.select_tool("freeTransform", v).unwrap();
    s.set_tool_option("mode", &json!("distort"));
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 200.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 250.0, 260.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 250.0, 260.0), v).unwrap();
    let d = &s.doc().unwrap().doc;
    let pts: Vec<Point> = d.node(a).unwrap().path_data().unwrap().anchors().map(|(_, _, an)| an.p).collect();
    assert!(pts.iter().any(|p| p.distance(Point::new(250.0, 260.0)) < 1e-6), "{pts:?}");
    assert!(pts.iter().any(|p| p.distance(Point::new(100.0, 100.0)) < 1e-6));
    assert_eq!(s.journal.last().map(|j| j.0.as_str()), Some("object.distort"));
}

/// With Constrain on, a Free Distort corner keeps to the diagonal through the opposite corner, in
/// one undo step that undoes and redoes whole.
#[test]
fn free_transform_constrained_distort_keeps_the_corner_on_its_diagonal() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    let v = ViewInfo::default();
    s.select_tool("freeTransform", v).unwrap();
    s.set_tool_option("mode", &json!("distort"));
    s.set_tool_option("constrain", &json!(true));
    let before = undo_len(&s);
    s.pointer(&PointerEvent::new(PointerKind::Down, 300.0, 200.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 320.0, 230.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 340.0, 260.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 340.0, 260.0), v).unwrap();
    let pts = |s: &Session| -> Vec<Point> { s.doc().unwrap().doc.node(a).unwrap().path_data().unwrap().anchors().map(|(_, _, an)| an.p).collect() };
    let has_all = |pts: &[Point], want: [(f64, f64); 4]| want.iter().all(|&(x, y)| pts.iter().any(|q| q.distance(Point::new(x, y)) < 1e-6));
    // (40, 60) along the diagonal (200, 100) is 0.28 of it, (56, 28).
    let distorted = [(100.0, 100.0), (300.0, 100.0), (356.0, 228.0), (100.0, 200.0)];
    let original = [(100.0, 100.0), (300.0, 100.0), (300.0, 200.0), (100.0, 200.0)];
    assert!(has_all(&pts(&s), distorted), "{:?}", pts(&s));
    assert_eq!(undo_len(&s), before + 1);
    assert_eq!(s.journal.last().map(|j| j.0.as_str()), Some("object.distort"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(has_all(&pts(&s), original), "{:?}", pts(&s));
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(has_all(&pts(&s), distorted), "{:?}", pts(&s));
}

#[test]
fn free_transform_cmd_held_once_a_corner_drag_started_distorts_in_one_step() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let v = ViewInfo::default();
    s.select_tool("freeTransform", v).unwrap();
    let before = undo_len(&s);
    let cmd = Mods { cmd: true, ..Default::default() };
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 200.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 210.0, 210.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 250.0, 260.0).with_mods(cmd), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 250.0, 260.0).with_mods(cmd), v).unwrap();
    let d = &s.doc().unwrap().doc;
    let pts: Vec<Point> = d.node(a).unwrap().path_data().unwrap().anchors().map(|(_, _, an)| an.p).collect();
    for p in [Point::new(250.0, 260.0), Point::new(100.0, 100.0), Point::new(200.0, 100.0), Point::new(100.0, 200.0)] {
        assert!(pts.iter().any(|q| q.distance(p) < 1e-6), "{p:?} not in {pts:?}");
    }
    assert_eq!(undo_len(&s), before + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().map(|e| e.label.as_str()), Some("Distort"));
}

#[test]
fn free_transform_side_handle_scales() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    gesture(&mut s, "freeTransform", &[(200.0, 150.0), (250.0, 150.0), (300.0, 150.0), (300.0, 150.0)], Mods::default());
    assert!(close(bounds(&s, a), Rect::new(100.0, 100.0, 300.0, 200.0)));
}

#[test]
fn eyedropper_copies_appearance_to_selection_and_defaults() {
    let mut s = session();
    let src = rect(&mut s, 300.0, 300.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 4})).unwrap();
    s.execute("object.setProps", &json!({"opacity": 50})).unwrap();
    let dst = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.default", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [dst.0]})).unwrap();
    gesture(&mut s, "eyedropper", &[(325.0, 325.0), (325.0, 325.0)], Mods::default());
    let d = &s.doc().unwrap().doc;
    let n = d.node(dst).unwrap();
    assert_eq!(n.appearance.fill_paint(), Paint::solid(Color::from_hex("#ff0000").unwrap()));
    assert_eq!(n.appearance.stroke_width(), 4.0);
    assert!((n.opacity - 0.5).abs() < 1e-6);
    assert_eq!(s.paint.fill, Paint::solid(Color::from_hex("#ff0000").unwrap()));
    assert_eq!(s.paint.stroke_width, 4.0);
    let _ = src;
}

#[test]
fn eyedropper_shift_samples_color_into_active_proxy() {
    let mut s = session();
    rect(&mut s, 300.0, 300.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"color": "#00ff00"})).unwrap();
    let dst = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.default", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [dst.0]})).unwrap();
    s.execute("paint.toggleActive", &json!({})).unwrap(); // stroke active
    gesture(&mut s, "eyedropper", &[(325.0, 325.0), (325.0, 325.0)], Mods { shift: true, ..Default::default() });
    let n = s.doc().unwrap().doc.node(dst).unwrap().clone();
    assert_eq!(n.appearance.stroke_paint(), Paint::solid(Color::from_hex("#00ff00").unwrap()));
    assert_eq!(n.appearance.fill_paint(), Paint::solid(Color::WHITE));
}

#[test]
fn gradient_tool_sets_vector_on_solid_fill() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    gesture(&mut s, "gradient", &[(100.0, 150.0), (150.0, 150.0), (200.0, 100.0), (200.0, 100.0)], Mods::default());
    let n = s.doc().unwrap().doc.node(a).unwrap().clone();
    let Paint::Gradient(g) = n.appearance.fill_paint() else { panic!("expected gradient") };
    let geom = g.geom.unwrap();
    assert_eq!(geom.start, Point::new(100.0, 150.0));
    assert_eq!(geom.end, Point::new(200.0, 100.0));
    assert_eq!(undo_len(&s), 2);
    // Changing an existing gradient keeps its stops.
    s.execute("paint.setGradientGeom", &json!({"start": [0, 0], "end": [10, 0]})).unwrap();
    let Paint::Gradient(g2) = s.doc().unwrap().doc.node(a).unwrap().appearance.fill_paint() else { panic!() };
    assert_eq!(g2.gradient, g.gradient);
    assert!(s.execute("paint.setGradientGeom", &json!({"start": [0, 0]})).is_err());
}

#[test]
fn artboard_tool_moves_with_art_creates_and_deletes() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    let outside = rect(&mut s, 900.0, 100.0, 50.0, 50.0);
    s.execute("select.none", &json!({})).unwrap();
    gesture(&mut s, "artboard", &[(400.0, 300.0), (420.0, 310.0), (450.0, 330.0), (450.0, 330.0)], Mods::default());
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.artboards[0].rect, Rect::new(50.0, 30.0, 850.0, 630.0));
    assert!(close(bounds(&s, a), Rect::new(150.0, 130.0, 200.0, 180.0)));
    assert!(close(bounds(&s, outside), Rect::new(900.0, 100.0, 950.0, 150.0)));
    // Draw a new artboard on the pasteboard.
    gesture(&mut s, "artboard", &[(1000.0, 0.0), (1100.0, 50.0), (1200.0, 100.0), (1200.0, 100.0)], Mods::default());
    assert_eq!(s.doc().unwrap().doc.artboards.len(), 2);
    assert_eq!(s.tool_options()["active"], 1);
    assert_eq!(s.doc().unwrap().doc.artboards[1].rect, Rect::new(1000.0, 0.0, 1200.0, 100.0));
    // Delete it: the tool takes the key ahead of the Clear shortcut (the UI only hands Delete to a
    // tool that claims it).
    assert!(s.tool_claims_key(ToolKey::Delete, ViewInfo::default()));
    s.tool_key(ToolKey::Delete, Mods::default(), ViewInfo::default()).unwrap();
    assert_eq!(s.doc().unwrap().doc.artboards.len(), 1);
    // The last artboard can't be deleted, but neither key may fall through to Clear.
    for key in [ToolKey::Delete, ToolKey::Backspace] {
        assert!(s.tool_claims_key(key, ViewInfo::default()));
        s.tool_key(key, Mods::default(), ViewInfo::default()).unwrap();
        assert_eq!(s.doc().unwrap().doc.artboards.len(), 1);
    }
}

#[test]
fn artboard_drag_keeps_a_flush_background_with_roundoff_and_multiple_texts() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let (x, y) = (-35.23064566724116, -107.85371637852609);
    s.execute("artboard.setProps", &json!({"index": 0, "x": x, "y": y})).unwrap();
    // The background's top is 2.84e-14 pt above the artboard's top.
    let background = rect(&mut s, x, -107.85371637852612, 600.0, 400.0);
    let mut ids = vec![background];
    for i in 0..3 {
        let text = s.execute("text.create", &json!({"x": x + 100.0, "y": y + 80.0 + f64::from(i) * 60.0, "text": "Test", "size": 12})).unwrap();
        ids.push(NodeId(text["id"].as_u64().unwrap()));
    }
    s.execute("select.none", &json!({})).unwrap();
    let before: Vec<Rect> = ids.iter().map(|id| bounds(&s, *id)).collect();
    let board = s.doc().unwrap().doc.artboards[0].rect;
    let history = undo_len(&s);
    gesture(&mut s, "artboard", &[(x + 300.0, y + 200.0), (x + 400.0, y + 240.0), (x + 400.0, y + 240.0)], Mods::default());
    let moved_board = s.doc().unwrap().doc.artboards[0].rect;
    let delta = moved_board.origin() - board.origin();
    assert!(delta.hypot() > 1.0, "the artboard actually moved");
    for (id, was) in ids.iter().zip(&before) {
        assert!(close(bounds(&s, *id), *was + delta), "{id:?} must move with the artboard");
    }
    assert_eq!(undo_len(&s), history + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect, board);
    for (id, was) in ids.iter().zip(&before) {
        assert!(close(bounds(&s, *id), *was), "Undo restores {id:?}");
    }
    s.execute("edit.redo", &json!({})).unwrap();
    for (id, was) in ids.iter().zip(&before) {
        assert!(close(bounds(&s, *id), *was + delta), "Redo moves {id:?}");
    }
}

#[test]
fn artboard_resize_via_handle_and_move_command() {
    let mut s = session();
    gesture(&mut s, "artboard", &[(800.0, 300.0), (850.0, 300.0), (900.0, 300.0), (900.0, 300.0)], Mods::default());
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect, Rect::new(0.0, 0.0, 900.0, 600.0));
    s.execute("artboard.move", &json!({"index": 0, "dx": -10, "dy": 5})).unwrap();
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect, Rect::new(-10.0, 5.0, 890.0, 605.0));
    assert!(s.execute("artboard.move", &json!({"index": 7, "dx": 1})).is_err());
}

/// Alt-dragging an artboard with the Artboard tool leaves it and its art in place and moves
/// copies of both.
#[test]
fn artboard_alt_drag_duplicates_it_with_its_art() {
    let mut s = session();
    let r = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("select.none", &json!({})).unwrap();
    let alt = Mods { alt: true, ..Mods::default() };
    gesture(&mut s, "artboard", &[(400.0, 300.0), (600.0, 300.0), (1300.0, 300.0), (1300.0, 300.0)], alt);
    let st = s.doc().unwrap();
    let d = &st.doc;
    assert_eq!(d.artboards.len(), 2);
    assert_eq!(d.artboards[0].rect, Rect::new(0.0, 0.0, 800.0, 600.0), "the original stays");
    assert_eq!((d.artboards[1].rect, d.artboards[1].name.as_str()), (Rect::new(900.0, 0.0, 1700.0, 600.0), "Artboard 1 copy"));
    assert_ne!(d.artboards[1].id, d.artboards[0].id);
    let kids = d.layers[0].children().unwrap();
    assert_eq!(kids.len(), 2, "the rectangle and its copy");
    assert_eq!(d.node(r).unwrap().geometric_bounds().unwrap().x0, 100.0);
    let copy = kids.iter().find(|n| n.id != r).unwrap();
    assert_eq!(copy.geometric_bounds().unwrap().x0, 1000.0);
    assert_eq!(st.history.undo.last().unwrap().label, "Duplicate Artboard");
    // One undo takes the copies away.
    s.execute("edit.undo", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!((d.artboards.len(), d.layers[0].children().unwrap().len()), (1, 1));
}

/// `artboard.move` with `copy` reports the copies it moved, and copies only the artboard when Move/Copy
/// Artwork with Artboard is off.
#[test]
fn artboard_move_copy_reports_the_copies_and_follows_move_art() {
    let mut s = session();
    let r = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    let out = s.execute("artboard.move", &json!({"index": 0, "dx": 900, "dy": 0, "copy": true, "moveArt": true})).unwrap();
    assert_eq!(out["index"], 1);
    let copies: Vec<NodeId> = out["moved"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect();
    assert_eq!(copies.len(), 1);
    assert_ne!(copies[0], r);
    assert_eq!(s.doc().unwrap().doc.node(copies[0]).unwrap().geometric_bounds().unwrap().x0, 1000.0);
    let out = s.execute("artboard.move", &json!({"index": 0, "dx": 0, "dy": 900, "copy": true})).unwrap();
    assert_eq!(out["moved"], json!([]));
    let d = &s.doc().unwrap().doc;
    assert_eq!((d.artboards.len(), d.layers[0].children().unwrap().len()), (3, 2), "an artboard alone");
    assert_eq!(d.artboards[2].name, "Artboard 1 copy 2", "a name of its own");
    let ids: std::collections::HashSet<u32> = d.artboards.iter().map(|a| a.id).collect();
    assert_eq!(ids.len(), 3, "every artboard has an id of its own");
}

fn stroke_width(s: &Session, id: NodeId) -> f64 {
    s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().unwrap().width
}

fn guide_positions(s: &Session) -> Vec<f64> {
    s.doc().unwrap().doc.ruler_guides().iter().map(|g| g.pos).collect()
}

/// Scale Artwork with Artboard (#602): `artboard.setProps {scaleArt}` takes the art fully inside the
/// artboard and its guides from the old rectangle onto the new one, each side by its own ratio, in
/// one undo step; art partly outside and canvas guides stay.
#[test]
fn artboard_set_props_scales_the_art_and_guides_with_the_artboard() {
    let mut s = session();
    let inside = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("stroke.set", &json!({"weight": 4})).unwrap();
    let partly = rect(&mut s, 700.0, 500.0, 200.0, 200.0);
    for (vertical, pos, artboard) in [(true, 400.0, json!(0)), (false, 300.0, json!(0)), (true, 400.0, json!(null))] {
        s.execute("guide.add", &json!({"vertical": vertical, "pos": pos, "artboard": artboard})).unwrap();
    }
    let before = s.doc().unwrap().doc.clone();
    // Half the size: half the art, half the stroke (Scale Strokes & Effects).
    let out = s.execute("artboard.setProps", &json!({"index": 0, "width": 400, "height": 300, "scaleArt": true, "strokes": true})).unwrap();
    assert_eq!(out["scaled"], json!([inside.0]));
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect, Rect::new(0.0, 0.0, 400.0, 300.0));
    assert_eq!(bounds(&s, inside), Rect::new(50.0, 50.0, 150.0, 100.0));
    assert_eq!(stroke_width(&s, inside), 2.0);
    assert_eq!(bounds(&s, partly), Rect::new(700.0, 500.0, 900.0, 700.0), "art partly outside stays");
    assert_eq!(guide_positions(&s), vec![200.0, 150.0, 400.0], "the artboard's guides scale, the canvas guide stays");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *before, "one undo step");
    // Moved and stretched (left edge to -100, twice as wide), strokes kept: the art follows.
    s.execute("artboard.setProps", &json!({"index": 0, "x": -100, "width": 1600, "scaleArt": true, "strokes": false})).unwrap();
    assert_eq!(bounds(&s, inside), Rect::new(100.0, 100.0, 500.0, 200.0));
    assert_eq!(stroke_width(&s, inside), 4.0);
    assert_eq!(guide_positions(&s), vec![700.0, 300.0, 400.0]);
}

/// Without `scaleArt` a resize leaves the art and guides alone (and returns nothing, as before);
/// with it, a pure move scales nothing (guides move along, as they always have).
#[test]
fn artboard_set_props_scales_art_only_when_asked_and_resized() {
    let mut s = session();
    let r = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("guide.add", &json!({"vertical": true, "pos": 400, "artboard": 0})).unwrap();
    assert_eq!(s.execute("artboard.setProps", &json!({"index": 0, "width": 400})).unwrap(), Value::Null);
    assert_eq!(bounds(&s, r), Rect::new(100.0, 100.0, 300.0, 200.0));
    assert_eq!(guide_positions(&s), vec![400.0]);
    let out = s.execute("artboard.setProps", &json!({"index": 0, "x": 50, "scaleArt": true})).unwrap();
    assert_eq!(out["scaled"], json!([]));
    assert_eq!(bounds(&s, r), Rect::new(100.0, 100.0, 300.0, 200.0));
    assert_eq!(guide_positions(&s), vec![450.0]);
    assert!(s.execute("artboard.setProps", &json!({"index": 9, "width": 10, "scaleArt": true})).is_err());
}

/// #1482: `artboard.setProps {artboards}` resizes several artboards in one undo step, and with
/// `scaleArt` each object scales once, with the artboard it was inside before the change: art that
/// the first artboard's new size moves into the second stays as the first one put it, and art
/// inside two overlapping artboards goes with the first one listed. An empty list, an artboard
/// listed twice, an entry without an index and an index that names no artboard are refused.
#[test]
fn artboard_set_props_resizes_several_artboards_and_scales_each_object_once() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100, "artboards": 2, "created": null})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 0, "x": 0, "y": 0})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "x": 120, "y": 0})).unwrap();
    let art = rect(&mut s, 70.0, 10.0, 10.0, 10.0);
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let out = s
        .execute(
            "artboard.setProps",
            &json!({"artboards": [{"index": 0, "width": 200}, {"index": 1, "width": 200, "name": " Back\t"}], "scaleArt": true}),
        )
        .unwrap();
    assert_eq!(out["scaled"], json!([art.0]));
    let boards = &s.doc().unwrap().doc.artboards;
    assert_eq!((boards[0].rect, boards[1].rect), (Rect::new(0.0, 0.0, 200.0, 100.0), Rect::new(120.0, 0.0, 320.0, 100.0)));
    assert_eq!(boards[1].name, "Back");
    assert_eq!(bounds(&s, art), Rect::new(140.0, 10.0, 160.0, 20.0), "scaled once, with Artboard 1");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *before);
    // Artboard 2 over Artboard 1's right half: the art inside both goes with Artboard 2, listed first.
    s.execute("artboard.setProps", &json!({"index": 1, "x": 50})).unwrap();
    s.execute("artboard.setProps", &json!({"artboards": [{"index": 1, "width": 200}, {"index": 0, "width": 200}], "scaleArt": true})).unwrap();
    assert_eq!(bounds(&s, art), Rect::new(90.0, 10.0, 110.0, 20.0));
    let was = s.doc().unwrap().doc.clone();
    for bad in [
        json!({"artboards": []}),
        json!({"artboards": [{"index": 0, "width": 50}, {"index": 0, "height": 50}]}),
        json!({"artboards": [{"index": 0, "width": 50}, {"height": 50}]}),
        json!({"artboards": [{"index": 0, "width": 50}, {"index": 2, "height": 50}]}),
    ] {
        assert!(s.execute("artboard.setProps", &bad).is_err(), "{bad}");
        assert_eq!(*s.doc().unwrap().doc, *was, "{bad}");
    }
}

/// Move Artwork is independent of scaling and takes only fully contained art and board guides.
#[test]
fn coordinate_moves_honor_move_art_and_keep_resize_behavior() {
    let mut s = session();
    let inside = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("stroke.set", &json!({"weight": 4})).unwrap();
    let partly = rect(&mut s, 700.0, 500.0, 200.0, 200.0);
    s.execute("guide.add", &json!({"vertical": true, "pos": 400, "artboard": 0})).unwrap();
    s.execute("guide.add", &json!({"vertical": true, "pos": 400})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    for scale in [false, true] {
        let out = s.execute("artboard.setProps", &json!({"index": 0, "x": 30, "y": -10, "moveArt": true, "scaleArt": scale})).unwrap();
        assert_eq!(bounds(&s, inside), Rect::new(130.0, 90.0, 330.0, 190.0));
        assert_eq!(bounds(&s, partly), Rect::new(700.0, 500.0, 900.0, 700.0));
        assert_eq!(stroke_width(&s, inside), 4.0);
        assert_eq!(guide_positions(&s), vec![430.0, 400.0]);
        assert_eq!(out, if scale { json!({"scaled": []}) } else { Value::Null });
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(*s.doc().unwrap().doc, *before);
    }
    for p in [json!({"index": 0, "x": 30, "moveArt": false}), json!({"index": 0, "x": 30, "width": 400, "moveArt": true})] {
        s.execute("artboard.setProps", &p).unwrap();
        assert_eq!(bounds(&s, inside), Rect::new(100.0, 100.0, 300.0, 200.0));
        s.execute("edit.undo", &json!({})).unwrap();
    }
}

#[test]
fn coordinate_moves_journal_the_locked_art_preference() {
    let mut s = session();
    let locked = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("object.lock", &json!({})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 0, "x": 30, "moveArt": true})).unwrap();
    assert_eq!(bounds(&s, locked), Rect::new(100.0, 100.0, 300.0, 200.0));
    s.execute("edit.undo", &json!({})).unwrap();
    s.prefs.move_locked_with_artboard = true;
    s.execute("artboard.setProps", &json!({"index": 0, "x": 30, "moveArt": true})).unwrap();
    assert_eq!(bounds(&s, locked), Rect::new(130.0, 100.0, 330.0, 200.0));
    let (id, p) = s.journal.last().unwrap().clone();
    assert_eq!(p["lockedAndHidden"], true);
    s.execute("edit.undo", &json!({})).unwrap();
    s.prefs.move_locked_with_artboard = false;
    s.execute(&id, &p).unwrap();
    assert_eq!(bounds(&s, locked), Rect::new(130.0, 100.0, 330.0, 200.0));
    s.execute("edit.undo", &json!({})).unwrap();
    // A null reads as a key left out, and journals the value it resolved to.
    s.prefs.move_locked_with_artboard = true;
    s.execute("artboard.setProps", &json!({"index": 0, "x": 30, "moveArt": true, "lockedAndHidden": null})).unwrap();
    assert_eq!(s.journal.last().unwrap().1["lockedAndHidden"], true);
}

/// A drag with the Artboard tool journals whether locked art went along, so a replay under the
/// other preference moves the same art.
#[test]
fn artboard_tool_drag_journals_the_locked_art_preference() {
    let mut s = session();
    let locked = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("object.lock", &json!({})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    s.prefs.move_locked_with_artboard = true;
    gesture(&mut s, "artboard", &[(400.0, 300.0), (420.0, 310.0), (450.0, 330.0), (450.0, 330.0)], Mods::default());
    let moved = Rect::new(150.0, 130.0, 200.0, 180.0);
    assert!(close(bounds(&s, locked), moved));
    let (id, p) = s.journal.last().unwrap().clone();
    assert_eq!((id.as_str(), &p["lockedAndHidden"]), ("artboard.move", &json!(true)));
    s.execute("edit.undo", &json!({})).unwrap();
    s.prefs.move_locked_with_artboard = false;
    s.execute(&id, &p).unwrap();
    assert!(close(bounds(&s, locked), moved));
}

/// Locked and hidden art scales only with Move Locked and Hidden Artwork with Artboard, as it
/// moves only with it.
#[test]
fn scale_artwork_with_artboard_leaves_locked_art_unless_the_preference_says() {
    let mut s = session();
    let locked = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("object.lock", &json!({})).unwrap();
    let scaled = |s: &mut Session| {
        let out = s.execute("artboard.setProps", &json!({"index": 0, "width": 400, "height": 300, "scaleArt": true})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        out["scaled"].clone()
    };
    assert_eq!(scaled(&mut s), json!([]));
    s.execute("prefs.set", &json!({"key": "moveLockedWithArtboard", "value": true})).unwrap();
    assert_eq!(scaled(&mut s), json!([locked.0]));
    // The choice is journaled: replayed with the preference off, the same art scales.
    s.execute("artboard.setProps", &json!({"index": 0, "width": 400, "height": 300, "scaleArt": true})).unwrap();
    assert_eq!(s.journal.last().unwrap().1["lockedAndHidden"], true);
    let (id, p) = s.journal.last().unwrap().clone();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("prefs.set", &json!({"key": "moveLockedWithArtboard", "value": false})).unwrap();
    assert_eq!(s.execute(&id, &p).unwrap()["scaled"], json!([locked.0]));
}

/// The Artboard tool with its `scaleArt` option on resizes proportionally and the art scales with
/// the artboard, in one undo step.
#[test]
fn artboard_tool_scales_art_with_the_artboard() {
    let mut s = session();
    let r = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    let before = s.doc().unwrap().doc.clone();
    s.set_tool_options(Some("artboard"), json!({"scaleArt": true}).as_object().unwrap());
    // The right handle 100 pt out: 9/8 as wide, and as tall about the middle of the left side.
    gesture(&mut s, "artboard", &[(800.0, 300.0), (850.0, 300.0), (900.0, 300.0), (900.0, 300.0)], Mods::default());
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect, Rect::new(0.0, -37.5, 900.0, 637.5));
    assert_eq!(bounds(&s, r), Rect::new(112.5, 75.0, 337.5, 187.5));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *before, "one undo step");
    // Off again (the option is remembered like Move Artwork with Artboard): the art stays.
    s.set_tool_options(Some("artboard"), json!({"scaleArt": false}).as_object().unwrap());
    gesture(&mut s, "artboard", &[(800.0, 300.0), (850.0, 300.0), (900.0, 300.0), (900.0, 300.0)], Mods::default());
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect, Rect::new(0.0, 0.0, 900.0, 600.0));
    assert_eq!(bounds(&s, r), Rect::new(100.0, 100.0, 300.0, 200.0));
}

#[test]
fn magic_wand_selects_same_fill() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    let b = rect(&mut s, 50.0, 10.0, 20.0, 20.0);
    let c = rect(&mut s, 90.0, 10.0, 20.0, 20.0);
    s.execute("paint.setFill", &json!({"color": "#ff0000", "ids": [c.0]})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    gesture(&mut s, "magicWand", &[(20.0, 20.0), (20.0, 20.0)], Mods::default());
    assert_eq!(s.doc().unwrap().selection.objects, vec![a, b]);
    gesture(&mut s, "magicWand", &[(100.0, 20.0), (100.0, 20.0)], Mods { shift: true, ..Default::default() });
    assert_eq!(s.doc().unwrap().selection.len(), 3);
}

#[test]
fn lasso_selects_anchor_subset() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("select.none", &json!({})).unwrap();
    gesture(&mut s, "lasso", &[(90.0, 90.0), (210.0, 90.0), (210.0, 110.0), (90.0, 110.0), (90.0, 110.0)], Mods::default());
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects, vec![a]);
    assert_eq!(st.selection.partial(a).map(|p| p.len()), Some(2));
}

/// #5351: a Lasso loop round a gradient mesh's points keeps the mesh selected and picks the
/// points inside it, so a solid fill colour recolours just those points.
#[test]
fn lasso_picks_gradient_mesh_points() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("object.mesh.create", &json!({"rows": 2, "cols": 2})).unwrap();
    // Round the top row of points (100,100), (150,100) and (200,100).
    gesture(&mut s, "lasso", &[(90.0, 90.0), (210.0, 90.0), (210.0, 110.0), (90.0, 110.0), (90.0, 110.0)], Mods::default());
    let st = s.doc().unwrap();
    let Some(NodeKind::Mesh(m)) = st.doc.node(a).map(|n| &n.kind) else { panic!("still a gradient mesh") };
    assert_eq!(m.points.len(), 9);
    assert_eq!(st.selection.objects, vec![a], "the mesh stays selected");
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let Some(NodeKind::Mesh(m)) = s.doc().unwrap().doc.node(a).map(|n| &n.kind) else { panic!("still a gradient mesh") };
    let red: Vec<usize> = (0..m.points.len()).filter(|i| m.points[*i].color.to_hex() == "#ff0000").collect();
    let top: Vec<usize> = (0..3).map(|c| m.idx(0, c)).collect();
    assert_eq!(red, top, "only the points inside the loop are recoloured");
}

/// With gradient mesh points picked, the Fill proxy (and the Color panel that edits it) shows
/// the color a solid fill gives them, not the mesh object's own fill: "?" when the picked
/// points' colors differ, the object's fill again once no point is picked.
#[test]
fn the_fill_proxy_shows_the_picked_mesh_points_color() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff8500"})).unwrap();
    s.execute("object.mesh.create", &json!({"rows": 2, "cols": 2})).unwrap();
    let object_fill = s.proxy_paints().0;
    // Round the top row of points.
    gesture(&mut s, "lasso", &[(90.0, 90.0), (210.0, 90.0), (210.0, 110.0), (90.0, 110.0), (90.0, 110.0)], Mods::default());
    s.execute("paint.setFill", &json!({"color": "#3366ff"})).unwrap();
    let shown = s.execute("paint.proxies", &json!({})).unwrap();
    assert_eq!(s.proxy_paints().0.color().map(|c| c.to_hex()), Some("#3366ff".into()), "the picked points' color: {shown}");
    assert_eq!(shown["fillMixed"], json!(false));
    assert_eq!(shown["fill"], json!(s.proxy_paints().0));
    // One picked point takes another color: the points differ.
    let Some(NodeKind::Mesh(m)) = s.doc().unwrap().doc.node(a).map(|n| &n.kind) else { panic!("still a gradient mesh") };
    let first = m.idx(0, 0);
    s.execute("object.mesh.setPointColor", &json!({"id": a.0, "index": first, "color": "#00ff00"})).unwrap();
    assert!(s.proxy_mixed().0, "the picked points' colors differ");
    // Picking none (the Selection tool) shows the object's fill again.
    s.select_tool("selection", ViewInfo::default()).unwrap();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    assert_eq!(s.proxy_paints().0, object_fill);
    assert!(!s.proxy_mixed().0);
}

/// With the Appearance panel's stroke row active, a solid fill still recolors the picked mesh
/// points, so the Fill proxy shows them; with its fill row active, the fill goes to that row, and
/// the proxy shows the row.
#[test]
fn the_fill_proxy_follows_where_the_fill_goes_with_an_appearance_row_active() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("object.mesh.create", &json!({"rows": 2, "cols": 2})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff8500"})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#000000"})).unwrap();
    let items = &s.doc().unwrap().doc.node(a).unwrap().appearance.items;
    let fill_row = items.iter().position(|it| it.is_fill()).expect("a fill row");
    let stroke_row = items.iter().position(|it| !it.is_fill()).expect("a stroke row");
    gesture(&mut s, "lasso", &[(90.0, 90.0), (210.0, 90.0), (210.0, 110.0), (90.0, 110.0), (90.0, 110.0)], Mods::default());
    assert_eq!(s.fill_mesh_points().first().map(|(_, v)| v.len()), Some(3), "the top row is picked");
    let point_color = |s: &Session| {
        let Some(NodeKind::Mesh(m)) = s.doc().unwrap().doc.node(a).map(|n| &n.kind) else { panic!("still a gradient mesh") };
        m.points[m.idx(0, 0)].color.to_hex()
    };

    s.execute("appearance.setActiveItem", &json!({"index": stroke_row})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#3366ff"})).unwrap();
    assert_eq!(point_color(&s), "#3366ff", "the stroke row leaves the fill on the picked points");
    assert_eq!(s.proxy_paints().0.color().map(|c| c.to_hex()), Some("#3366ff".into()));
    assert!(!s.proxy_mixed().0);

    s.execute("appearance.setActiveItem", &json!({"index": fill_row})).unwrap();
    assert!(s.fill_mesh_points().is_empty());
    s.execute("paint.setFill", &json!({"color": "#00ff00"})).unwrap();
    assert_eq!(point_color(&s), "#3366ff", "the fill row takes the fill");
    assert_eq!(s.proxy_paints().0.color().map(|c| c.to_hex()), Some("#00ff00".into()));
}

/// A gradient mesh with points picked selected with another object: a solid fill recolors the
/// points and fills the other object, and the Fill proxy and its "?" count the points' colors.
#[test]
fn the_fill_proxy_counts_picked_mesh_points_beside_other_objects() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let b = rect(&mut s, 300.0, 300.0, 50.0, 50.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("object.mesh.create", &json!({"rows": 2, "cols": 2})).unwrap();
    gesture(&mut s, "lasso", &[(90.0, 90.0), (210.0, 90.0), (210.0, 110.0), (90.0, 110.0), (90.0, 110.0)], Mods::default());
    s.execute("select.add", &json!({"ids": [b.0]})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, vec![a, b]);
    assert_eq!(s.fill_mesh_points().first().map(|(id, v)| (*id, v.len())), Some((a, 3)), "the top row is still picked");
    s.execute("paint.setFill", &json!({"color": "#3366ff"})).unwrap();
    let Some(NodeKind::Mesh(m)) = s.doc().unwrap().doc.node(a).map(|n| &n.kind) else { panic!("still a gradient mesh") };
    assert_eq!(m.points[m.idx(0, 0)].color.to_hex(), "#3366ff");
    assert_eq!(s.proxy_paints().0.color().map(|c| c.to_hex()), Some("#3366ff".into()));
    assert!(!s.proxy_mixed().0, "the points and the other object are the same color");
    s.execute("paint.setFill", &json!({"ids": [b.0], "color": "#00ff00"})).unwrap();
    assert!(s.proxy_mixed().0, "the picked points and the other object's fill differ");
}

/// Shift-drag a marquee with the Selection tool (#483): the selected objects it reaches are
/// deselected and the others selected, the rest of the selection staying as it was.
#[test]
fn a_selection_tool_click_gives_new_art_the_objects_fill() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"color": "#00ff00"})).unwrap();
    rect(&mut s, 200.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    gesture(&mut s, "selection", &[(125.0, 125.0), (125.0, 125.0)], Mods::default());
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    assert_eq!(s.paint.fill, s.doc().unwrap().doc.node(a).unwrap().appearance.fill_paint());
}

#[test]
fn selection_shift_marquee_toggles_objects() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    let b = rect(&mut s, 200.0, 100.0, 50.0, 50.0);
    let c = rect(&mut s, 300.0, 100.0, 50.0, 50.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let shift = Mods { shift: true, ..Default::default() };
    gesture(&mut s, "selection", &[(180.0, 80.0), (300.0, 200.0), (380.0, 200.0)], shift);
    assert_eq!(s.doc().unwrap().selection.objects, vec![a, c]);
    // Again over all three: a leaves, b joins, c leaves.
    gesture(&mut s, "selection", &[(80.0, 80.0), (300.0, 200.0), (380.0, 200.0)], shift);
    assert_eq!(s.doc().unwrap().selection.objects, vec![b]);
    // Without Shift the marquee replaces the selection.
    gesture(&mut s, "selection", &[(80.0, 80.0), (300.0, 200.0), (260.0, 200.0)], Mods::default());
    assert_eq!(s.doc().unwrap().selection.objects, vec![a, b]);
}

/// Shift-drag a marquee with Direct Selection (#483): the anchors inside toggle; a path left with
/// none of them leaves the selection, one with all of them is selected whole again.
#[test]
fn direct_selection_shift_marquee_toggles_anchors() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let b = rect(&mut s, 300.0, 100.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let shift = Mods { shift: true, ..Default::default() };
    // Around a's top edge (two anchors, selected) and b's top-left anchor (not selected).
    let top = [(90.0, 90.0), (310.0, 110.0), (310.0, 110.0)];
    gesture(&mut s, "directSelection", &top, shift);
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects, vec![a, b]);
    assert_eq!(st.selection.partial(a).map(|p| p.len()), Some(2), "a's bottom anchors stay");
    assert_eq!(st.selection.partial(b).map(|p| p.len()), Some(1));
    // The same again: a whole once more, b out.
    gesture(&mut s, "directSelection", &top, shift);
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects, vec![a]);
    assert_eq!(st.selection.partial(a), None);
    // Group Selection's marquee toggles the paths it touches whole: a leaves, b joins (#8047).
    gesture(&mut s, "groupSelection", &top, shift);
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects, vec![b]);
    assert_eq!(st.selection.partial(b), None);
}

/// The Lasso: Shift adds anchors, Alt takes them away, and neither drops the rest of the
/// selection (a path selected whole stays whole when its anchors are added again).
#[test]
fn lasso_shift_adds_and_alt_subtracts() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let t = s.execute("text.create", &json!({"x": 300, "y": 300, "text": "Hi"})).unwrap();
    let t = NodeId(t["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [a.0, t.0]})).unwrap();
    // A loop round a's top-left anchor.
    let corner = [(90.0, 90.0), (110.0, 90.0), (110.0, 110.0), (90.0, 110.0), (90.0, 110.0)];
    gesture(&mut s, "lasso", &corner, Mods { shift: true, ..Default::default() });
    let st = s.doc().unwrap();
    assert_eq!((st.selection.objects.clone(), st.selection.partial(a)), (vec![a, t], None));
    gesture(&mut s, "lasso", &corner, Mods { alt: true, ..Default::default() });
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects, vec![a, t], "the type stays selected");
    assert_eq!(st.selection.partial(a).map(|p| p.len()), Some(3));
}

#[test]
fn measure_tool_leaves_document_untouched() {
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let rev = s.doc().unwrap().revision;
    let n = undo_len(&s);
    gesture(&mut s, "measure", &[(0.0, 0.0), (30.0, 40.0), (30.0, 40.0)], Mods::default());
    assert_eq!(s.doc().unwrap().revision, rev);
    assert_eq!(undo_len(&s), n);
    assert!((s.tool_options()["distance"].as_f64().unwrap() - 50.0).abs() < 1e-9);
    assert!(!s.overlays(ViewInfo::default()).is_empty());
}

#[test]
fn copy_from_requires_source() {
    let mut s = session();
    assert!(s.execute("appearance.copyFrom", &json!({})).is_err());
    assert!(s.execute("appearance.copyFrom", &json!({"source": 9999})).is_err());
}

#[test]
fn distort_rejects_malformed_coordinates_and_ids_without_editing_art() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let original = bounds(&s, a);
    let history = undo_len(&s);
    let corners = json!([[10, 0], [90, 0], [100, 100], [0, 100]]);
    for p in [
        json!({"corners": [[10, 0, 50], [90, 0], [100, 100], [0, 100]]}),
        json!({"corners": [[10, 0], [90, 0], ["bad", 100], [0, 100]]}),
        json!({"corners": [[10, 0], [90, 0], [100, 100], [0, 100], ["bad"]]}),
        json!({"corners": corners, "from": [0, 0, 100]}),
        json!({"corners": corners, "from": "not bounds"}),
        json!({"corners": corners, "ids": [a.0, "bad"]}),
        json!({"corners": corners, "ids": [a.0, u64::MAX]}),
    ] {
        assert!(s.execute("object.distort", &p).is_err(), "invalid input: {p}");
        assert!(close(bounds(&s, a), original));
        assert_eq!(undo_len(&s), history);
    }
    s.execute("object.distort", &json!({"corners": corners, "from": [0, 0, 100, 100], "ids": [a.0]})).unwrap();
    assert_eq!(undo_len(&s), history + 1);
}

/// `object.distort` with `angle` reads `from` as a box in the frame turned by it, as the Free
/// Transform tool sends a turned bounding box: corners where that box already stands leave the art.
#[test]
fn distort_reads_from_in_a_turned_frame_and_rejects_a_bad_angle() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let original = bounds(&s, a);
    let bx = vectorcraft_doc::OrientedBox { angle: 90.0, rect: Rect::new(-100.0, 0.0, 0.0, 100.0) };
    let corners: Vec<serde_json::Value> = bx.corners().iter().map(|c| json!([c.x, c.y])).collect();
    let history = undo_len(&s);
    for p in [
        json!({"corners": corners, "from": [-100, 0, 0, 100], "angle": "90", "ids": [a.0]}),
        json!({"corners": corners, "from": [-100, 0, 0, 100], "angle": [90], "ids": [a.0]}),
    ] {
        assert!(s.execute("object.distort", &p).is_err(), "invalid input: {p}");
        assert!(close(bounds(&s, a), original));
        assert_eq!(undo_len(&s), history);
    }
    s.execute("object.distort", &json!({"corners": corners, "from": [-100, 0, 0, 100], "angle": 90, "ids": [a.0]})).unwrap();
    assert!(close(bounds(&s, a), original), "{:?}", bounds(&s, a));
    assert_eq!(undo_len(&s), history, "corners where the box stands change nothing");
    // Without `from`, the box is the objects' bounds in the turned frame: the same box here.
    s.execute("object.distort", &json!({"corners": corners, "angle": 90, "ids": [a.0]})).unwrap();
    assert!(close(bounds(&s, a), original), "{:?}", bounds(&s, a));
}

/// `artboard.move` and `artboard.delete` take several artboards at once (#1139): one undo step
/// each, the art on each moving with it, and one artboard always left.
#[test]
fn several_artboards_move_and_delete_together() {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 1000, "y": 0, "width": 200, "height": 200})).unwrap();
    s.execute("artboard.new", &json!({"x": 1400, "y": 0, "width": 200, "height": 200})).unwrap();
    let r = rect(&mut s, 1050.0, 50.0, 50.0, 50.0);
    let before: Vec<Rect> = s.doc().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    let out = s.execute("artboard.move", &json!({"indices": [2, 1], "dx": 10, "dy": 20, "moveArt": true})).unwrap();
    assert_eq!(out["indices"], json!([1, 2]));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.artboards[0].rect, before[0]);
    assert_eq!(
        (d.artboards[1].rect, d.artboards[2].rect),
        (before[1] + vectorcraft_geom::Vec2::new(10.0, 20.0), before[2] + vectorcraft_geom::Vec2::new(10.0, 20.0))
    );
    assert_eq!(d.node(r).unwrap().geometric_bounds().unwrap().x0, 1060.0, "the art on artboard 2 moved with it");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.artboards[2].rect, before[2], "one undo step");
    for bad in [json!({"indices": [1, 9]}), json!({"indices": []}), json!({"indices": [-1]}), json!({"indices": "1"})] {
        assert!(s.execute("artboard.move", &json!({"indices": bad["indices"], "dx": 1})).is_err(), "{bad}");
    }
    assert!(s.execute("artboard.delete", &json!({"indices": [0, 1, 2]})).is_err(), "one is always left");
    s.execute("artboard.delete", &json!({"indices": [2, 0]})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.artboards.iter().map(|a| a.rect).collect::<Vec<_>>(), [before[1]]);
}
