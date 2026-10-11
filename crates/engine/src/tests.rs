use serde_json::json;

use super::*;
use vectorcraft_geom::Point;
use vectorcraft_tools::{PointerEvent, PointerKind, ToolKey};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

#[test]
fn create_and_undo_redo() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 100.0, 50.0);
    assert!(s.doc().unwrap().doc.node(a).is_some());
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(a).is_none());
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(a).is_some());
    assert!(s.execute("edit.redo", &json!({})).is_err());
}

#[test]
fn unknown_and_disabled() {
    let mut s = Session::new();
    assert!(matches!(s.execute("nope", &json!({})), Err(EngineError::UnknownCommand(_))));
    assert!(matches!(s.execute("object.group", &json!({})), Err(EngineError::Disabled(..))));
}

#[test]
fn group_ungroup() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.parent_of(a), Some(g));
    assert_eq!(d.node(g).unwrap().children().unwrap().len(), 2);
    s.execute("object.ungroup", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.node(g).is_none());
    assert_eq!(d.parent_of(a), d.layers.first().map(|l| l.id));
    assert_eq!(s.doc().unwrap().selection.len(), 2);
}

/// Makes 10 pt squares at (0, 90) and (50, 100) into a compound path or group with the commands in
/// `make`, each run on the selection (the last one makes `outer`), draws a 10 pt square `r` at
/// (200, 0), selects the second square and `r`, and sets the selection's `key` to that square
/// → (session, key, outer, r).
fn key_inside(make: &[&str]) -> (Session, NodeId, NodeId, NodeId) {
    let mut s = session();
    let a = rect(&mut s, 0.0, 90.0, 10.0, 10.0);
    let key = rect(&mut s, 50.0, 100.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [a.0, key.0]})).unwrap();
    let mut outer = key;
    for cmd in make {
        outer = NodeId(s.execute(cmd, &json!({})).unwrap()["id"].as_u64().unwrap());
    }
    let r = rect(&mut s, 200.0, 0.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [key.0, r.0]})).unwrap();
    s.select(|_, sel| sel.key = Some(key)).unwrap();
    (s, key, outer, r)
}

/// Aligns the selection left to its key object and, after Undo, spaces it 5 pt apart vertically
/// → the top-left corners of `outer` and `r` after each.
fn aligned_then_spaced(s: &mut Session, outer: NodeId, r: NodeId) -> [((f64, f64), (f64, f64)); 2] {
    let at = |s: &Session, id| {
        let b = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
        (b.x0, b.y0)
    };
    s.execute("object.align", &json!({"horizontal": "left", "bounds": "geometric"})).unwrap();
    let aligned = (at(s, outer), at(s, r));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("object.distributeSpacing", &json!({"axis": "vertical", "spacing": 5, "bounds": "geometric"})).unwrap();
    [aligned, (at(s, outer), at(s, r))]
}

/// Align and Distribute Spacing move a compound path as one object. With one of its members as the
/// key object, the compound path stays where it is: Align moves the others to the member's edge,
/// and Distribute Spacing spaces them from the compound path.
#[test]
fn a_compound_path_that_contains_the_key_object_stays_in_place() {
    let (mut s, key, outer, r) = key_inside(&["object.compoundPath.make"]);
    assert_eq!(s.doc().unwrap().selection.key, Some(key));
    let [aligned, spaced] = aligned_then_spaced(&mut s, outer, r);
    assert_eq!(aligned, ((0.0, 90.0), (50.0, 0.0)), "it stays, the other moved to the key");
    assert_eq!(spaced, ((0.0, 90.0), (200.0, 75.0)), "5 pt apart, it stays");
}

/// The key object stays the key when the group that contains it is added to the selection, by
/// `select.add` or by the Group Selection tool's click on a selected member. Align and Distribute
/// Spacing move that group as one object, and the group stays where it is, also when the key is in
/// a group inside it.
#[test]
fn a_selected_group_that_contains_the_key_object_stays_in_place() {
    for make in [&["object.group"][..], &["object.group", "object.group"]] {
        let (mut s, key, outer, r) = key_inside(make);
        s.execute("select.add", &json!({"ids": [outer.0]})).unwrap();
        assert_eq!(s.doc().unwrap().selection.key, Some(key), "{make:?}");
        let [aligned, spaced] = aligned_then_spaced(&mut s, outer, r);
        assert_eq!(aligned, ((0.0, 90.0), (50.0, 0.0)), "{make:?}: it stays, the other moved to the key");
        assert_eq!(spaced, ((0.0, 90.0), (200.0, 75.0)), "{make:?}: 5 pt apart, it stays");
    }
}

#[test]
fn arrange_order() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let c = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("object.arrange.bringToFront", &json!({})).unwrap();
    let order = |s: &Session| s.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id).collect::<Vec<_>>();
    assert_eq!(order(&s), vec![b, c, a]);
    s.execute("object.arrange.sendBackward", &json!({})).unwrap();
    assert_eq!(order(&s), vec![b, a, c]);
    s.execute("object.arrange.sendToBack", &json!({})).unwrap();
    assert_eq!(order(&s), vec![a, b, c]);
    s.execute("object.arrange.bringForward", &json!({})).unwrap();
    assert_eq!(order(&s), vec![b, a, c]);
}

#[test]
fn transform_and_again() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    s.execute("object.transformAgain", &json!({})).unwrap();
    let b = s.doc().unwrap().doc.node(a).unwrap().geometric_bounds().unwrap();
    assert_eq!(b.x0, 10.0);
    s.execute("object.move", &json!({"dx": 0, "dy": 20, "copy": true})).unwrap();
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 2);
}

#[test]
fn rotate_and_scale() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 20.0, 10.0);
    s.execute("object.rotate", &json!({"angle": 90})).unwrap();
    let b = s.doc().unwrap().doc.node(a).unwrap().geometric_bounds().unwrap();
    assert!((b.width() - 10.0).abs() < 1e-9 && (b.height() - 20.0).abs() < 1e-9);
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    let b = s.doc().unwrap().doc.node(a).unwrap().geometric_bounds().unwrap();
    assert!((b.width() - 20.0).abs() < 1e-9);
    // Scale Strokes & Effects is off by default: the stroke keeps its weight
    assert_eq!(s.doc().unwrap().doc.node(a).unwrap().appearance.stroke_width(), 1.0);
    // and scales with the object when asked
    s.execute("object.scale", &json!({"sx": 200, "strokes": true})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node(a).unwrap().appearance.stroke_width(), 2.0);
}

#[test]
fn copy_paste_front_back() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let _b = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    let r = s.execute("edit.pasteInFront", &json!({})).unwrap();
    let c = NodeId(r["ids"][0].as_u64().unwrap());
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.index_path(c).unwrap()[1], 1);
    s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 4);
    s.execute("edit.cut", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 3);
}

#[test]
fn fill_and_stroke_commands() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 4, "cap": "round", "dash": [3, 2]})).unwrap();
    let n = s.doc().unwrap().doc.node(a).unwrap().clone();
    assert_eq!(n.appearance.fill_paint().color().unwrap().to_hex(), "#ff0000");
    let st = n.appearance.stroke().unwrap();
    assert_eq!(st.width, 4.0);
    assert_eq!(st.cap, vectorcraft_doc::LineCap::Round);
    assert_eq!(st.dash.as_ref().unwrap().pattern, vec![3.0, 2.0]);
    s.execute("paint.swap", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node(a).unwrap().appearance.stroke_paint().color().unwrap().to_hex(), "#ff0000");
    s.execute("paint.setFill", &json!({"swatch": "Cyan"})).unwrap();
    s.execute("paint.setFill", &json!({"gradient": {"kind": "radial"}})).unwrap();
}

#[test]
fn stroke_rejects_invalid_dash_components_and_preserves_defaults_on_error() {
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 40.0, 30.0);
    s.execute("stroke.set", &json!({"weight": 3, "dash": [6, 3]})).unwrap();
    let previous_weight = s.paint.stroke_width;
    let previous_stroke = s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().cloned();

    for params in [
        json!({"weight": 10, "dash": [6, "not a number", 3]}),
        json!({"weight": 10, "ids": "not an array"}),
        json!({"weight": 10, "item": "invalid"}),
        json!({"weight": 10, "arrowAlign": "unknown"}),
    ] {
        assert!(s.execute("stroke.set", &params).is_err(), "{params} must fail");
        assert_eq!(s.paint.stroke_width, previous_weight, "failed stroke edit changed new-art defaults");
        assert_eq!(s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().cloned(), previous_stroke);
    }

    // A valid dash list still updates the selected stroke and new-art default width.
    s.execute("stroke.set", &json!({"weight": 7, "dash": [4, 2]})).unwrap();
    assert_eq!(s.paint.stroke_width, 7.0);
    assert_eq!(s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().unwrap().width, 7.0);

    // With nothing selected the panel sets up the next object drawn: a rejected edit doesn't.
    s.execute("select.none", &json!({})).unwrap();
    let next = s.new_art();
    assert!(s.execute("stroke.set", &json!({"weight": 12, "dash": [2, 2], "arrowAlign": "unknown"})).is_err());
    assert_eq!(s.new_art(), next);
    s.execute("stroke.set", &json!({"weight": 12, "dash": [2, 2]})).unwrap();
    let next = s.new_art();
    let stroke = next.stroke().unwrap();
    assert_eq!((stroke.width, stroke.dash.as_ref().map(|d| d.pattern.clone())), (12.0, Some(vec![2.0, 2.0])));
}

#[test]
fn rejected_paint_edits_leave_new_art_defaults_and_focus_unchanged() {
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("paint.setFill", &json!({"color": "#00ff00"})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
    let defaults = s.paint.clone();
    let focused = s.fill_active;
    let original_fill = s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint();
    let original_stroke = s.doc().unwrap().doc.node(id).unwrap().appearance.stroke_paint();

    // Both shortcut commands used to change defaults before checking invalid ids.
    for (command, params) in [
        ("paint.swap", json!({"ids": "not an array"})),
        ("paint.default", json!({"ids": ["not an object id"]})),
        ("paint.setFill", json!({"color": "#123456", "ids": [false]})),
        ("paint.setStroke", json!({"color": "#123456", "ids": "not an array"})),
        ("paint.setFill", json!({"color": "#123456", "item": "invalid"})),
    ] {
        assert!(s.execute(command, &params).is_err(), "{command} should reject {params}");
        assert_eq!(s.paint, defaults, "{command} changed new-art defaults on failure");
        assert_eq!(s.fill_active, focused, "{command} changed the active proxy on failure");
        let appearance = &s.doc().unwrap().doc.node(id).unwrap().appearance;
        assert_eq!(appearance.fill_paint(), original_fill);
        assert_eq!(appearance.stroke_paint(), original_stroke);
    }

    // Successful operations still update both the selected object and new-art defaults.
    s.execute("paint.swap", &json!({})).unwrap();
    assert_eq!(s.paint.fill, defaults.stroke);
    assert_eq!(s.paint.stroke, defaults.fill);
}

#[test]
fn select_same_fill() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("paint.setFill", &json!({"color": "#00ff00"})).unwrap();
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    let _c = {
        s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
        rect(&mut s, 40.0, 0.0, 10.0, 10.0)
    };
    // Setting a fill also recolours the current selection (b), so b and c are blue, a green.
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    let r = s.execute("select.same.fillColor", &json!({})).unwrap();
    assert_eq!(r["count"], 2);
    assert!(!s.doc().unwrap().selection.contains(a));
    // Reselect after Deselect repeats it from the same reference object (#903).
    let blue = s.doc().unwrap().selection.objects.clone();
    s.execute("select.none", &json!({})).unwrap();
    s.execute("select.reselect", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, blue);
    // With its reference object gone, it fails as Select Same does, and selects nothing.
    s.execute("select.none", &json!({})).unwrap();
    for id in &blue {
        s.execute("edit.clear", &json!({"ids": [id.0]})).unwrap();
    }
    assert!(s.execute("select.reselect", &json!({})).is_err());
    assert!(s.doc().unwrap().selection.objects.is_empty());
}

#[test]
fn layers_and_lock_hide() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let l2 = NodeId(s.execute("layer.new", &json!({"name": "Ink"})).unwrap()["id"].as_u64().unwrap());
    let b = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    assert_eq!(s.doc().unwrap().doc.layer_of(b), Some(l2));
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, vec![b]);
    s.execute("object.unlockAll", &json!({})).unwrap();
    s.execute("object.hide", &json!({})).unwrap();
    s.execute("object.showAll", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(a).unwrap().visible);
    s.execute("layer.delete", &json!({"id": l2.0})).unwrap();
    assert!(s.doc().unwrap().doc.node(b).is_none());
}

/// A key object is one object of a selection of several: a Shift-click (`select.toggle`) or a
/// hidden or deleted row that leaves one object selected clears the key, and `select.key` sets
/// none on a single selected object. A Shift-drag marquee that leaves two or more objects selected
/// keeps the key in either order of its ids. `select.key` also sets none on a group selected with
/// only its own member, and sets it once another object is selected.
#[test]
fn a_lone_selected_object_is_not_the_key_object() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 100.0, 30.0, 20.0, 20.0);
    let c = rect(&mut s, 200.0, 60.0, 10.0, 10.0);
    let key = |s: &Session| s.doc().unwrap().selection.key;
    let objects = |s: &Session| s.doc().unwrap().selection.objects.clone();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("select.key", &json!({"id": b.0})).unwrap();
    assert_eq!(key(&s), Some(b));
    s.execute("select.toggle", &json!({"id": a.0})).unwrap();
    assert_eq!(objects(&s), vec![b]);
    assert_eq!(key(&s), None, "the Shift-click left one object");
    s.execute("select.key", &json!({"id": b.0})).unwrap();
    assert_eq!(key(&s), None, "one object selected");
    for ids in [[a.0, c.0], [c.0, a.0]] {
        s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
        s.execute("select.key", &json!({"id": b.0})).unwrap();
        s.execute("select.toggle", &json!({"ids": ids})).unwrap();
        assert_eq!((objects(&s), key(&s)), (vec![b, c], Some(b)), "the marquee toggled {ids:?}");
    }
    // The Group Selection tool's second click on a member adds its group.
    s.execute("select.set", &json!({"ids": [b.0, c.0]})).unwrap();
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    s.execute("select.add", &json!({"ids": [g.0]})).unwrap();
    s.execute("select.key", &json!({"id": g.0})).unwrap();
    assert_eq!(key(&s), None, "a group and its own member");
    s.execute("select.add", &json!({"ids": [a.0]})).unwrap();
    s.execute("select.key", &json!({"id": g.0})).unwrap();
    assert_eq!(key(&s), Some(g), "another object outside the group");
    s.execute("select.set", &json!({"ids": [a.0, g.0]})).unwrap();
    s.execute("select.key", &json!({"id": g.0})).unwrap();
    assert_eq!(key(&s), Some(g));
    s.execute("layer.setProps", &json!({"id": a.0, "visible": false})).unwrap();
    assert_eq!((objects(&s), key(&s)), (vec![g], None), "hiding the other object's row left one object");
    s.execute("layer.setProps", &json!({"id": a.0, "visible": true})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, g.0]})).unwrap();
    s.execute("select.key", &json!({"id": g.0})).unwrap();
    assert_eq!(key(&s), Some(g));
    s.execute("layer.delete", &json!({"id": a.0})).unwrap();
    assert_eq!(objects(&s), vec![g]);
    assert_eq!(key(&s), None, "deleting the other object's row left one object");
}

#[test]
fn clipping_and_compound() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 25.0, 25.0, 50.0, 50.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(s.doc().unwrap().doc.node(NodeId(c)).unwrap().kind_label(), "Compound Path");
    s.execute("object.compoundPath.release", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().selection.len(), 2);
    let g = s.execute("object.clippingMask.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(s.doc().unwrap().doc.node(NodeId(g)).unwrap().kind_label(), "Clip Group");
    // Edit Contents selects the clipped art; Edit Clipping Path the mask (also from inside the group).
    s.execute("object.clippingMask.editContents", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    s.execute("object.clippingMask.editMask", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, vec![b]);
}

#[test]
fn align_left() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 50.0, 30.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.align", &json!({"horizontal": "left"})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node(b).unwrap().geometric_bounds().unwrap().x0, 0.0);
}

/// #541: with the Selection tool, a click on one object of the selection makes it the key object:
/// aligning to the key moves the others to it, Distribute Spacing spaces them from it, and a click
/// on the key again lets it go.
#[test]
fn a_click_sets_the_key_object_that_align_and_spacing_keep_still() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 100.0, 30.0, 20.0, 20.0);
    let c = rect(&mut s, 300.0, 60.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0, c.0]})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let click = |s: &mut Session| {
        for kind in [PointerKind::Down, PointerKind::Up] {
            s.pointer(&PointerEvent::new(kind, 110.0, 40.0), v).unwrap();
        }
    };
    click(&mut s);
    assert_eq!(s.doc().unwrap().selection.key, Some(b));
    assert_eq!(s.doc().unwrap().selection.objects.len(), 3, "the selection stays");
    let x0 = |s: &Session, id| s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0;
    // With a key object, aligning aligns to it (the Control bar and Properties name no target).
    s.execute("object.align", &json!({"horizontal": "left", "bounds": "geometric"})).unwrap();
    assert_eq!((x0(&s, a), x0(&s, b), x0(&s, c)), (100.0, 100.0, 100.0), "the others moved to the key");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("object.distributeSpacing", &json!({"axis": "horizontal", "spacing": 5, "bounds": "geometric"})).unwrap();
    assert_eq!((x0(&s, a), x0(&s, b), x0(&s, c)), (85.0, 100.0, 125.0), "5 pt apart, the key still");
    click(&mut s);
    assert_eq!(s.doc().unwrap().selection.key, None, "a click on the key again lets it go");
}

#[test]
fn selection_tool_drag_is_one_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let undo_before = s.doc().unwrap().history.undo.len();
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 150.0, 150.0), v).unwrap();
    for x in [155.0, 160.0, 170.0, 180.0] {
        s.pointer(&PointerEvent::new(PointerKind::Drag, x, 150.0), v).unwrap();
    }
    s.pointer(&PointerEvent::new(PointerKind::Up, 180.0, 150.0), v).unwrap();
    let b = s.doc().unwrap().doc.node(a).unwrap().geometric_bounds().unwrap();
    assert_eq!(b.x0, 130.0);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_before + 1);
    assert_eq!(s.journal.last().unwrap().0, "object.transform");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node(a).unwrap().geometric_bounds().unwrap().x0, 100.0);
}

/// How many steps there are to undo and to redo, and whether the document has unsaved changes.
fn history(s: &Session) -> (usize, usize, bool) {
    let st = s.doc().unwrap();
    (st.history.undo.len(), st.history.redo.len(), st.is_dirty())
}

/// Select `ids` with a move of them left to redo, and mark the document saved.
fn select_with_a_step_to_redo(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.doc_mut().unwrap().mark_saved();
}

/// An Align that moves nothing (one object on the artboard's left edge, a pair already aligned, a
/// pair that an earlier Align left a rounding error apart) records no undo step: the step to redo
/// stays, the document stays unmodified, and the journal, which the Actions panel records from,
/// still has the command.
#[test]
fn an_align_that_moves_nothing_records_no_undo_step() {
    let mut s = session();
    let x0 = |s: &Session, id| s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0;
    let left = json!({"horizontal": "left"});
    let a = rect(&mut s, 0.0, 50.0, 40.0, 40.0);
    select_with_a_step_to_redo(&mut s, &[a]);
    let before = history(&s);
    assert_eq!((before.1, before.2), (1, false), "a step to redo, and nothing unsaved");
    // Object › Align sends no `to`, and the Align panel aligns one object to the artboard.
    for p in [left.clone(), json!({"horizontal": "left", "to": "artboard"})] {
        s.execute("object.align", &p).unwrap();
        assert_eq!(history(&s), before, "one object and no key: {p}");
        assert_eq!(s.journal.last().unwrap(), &("object.align".to_string(), p));
    }
    let b = rect(&mut s, 0.0, 200.0, 80.0, 20.0);
    select_with_a_step_to_redo(&mut s, &[a, b]);
    let before = history(&s);
    s.execute("object.align", &left).unwrap();
    assert_eq!(history(&s), before, "a pair already aligned");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!((x0(&s, a), x0(&s, b)), (5.0, 5.0), "Redo brings back the move");
    // An Align that moves something is one undo step, and it clears Redo.
    let c = rect(&mut s, 120.0, 300.0, 20.0, 20.0);
    select_with_a_step_to_redo(&mut s, &[a, c]);
    let undo = history(&s).0;
    s.execute("object.align", &left).unwrap();
    assert_eq!(history(&s), (undo + 1, 0, true));
    assert_eq!(x0(&s, c), 5.0);
    // Aligning a rectangle at 51.8 to one at 10.7 leaves it a rounding error off 10.7, and a
    // second Align records no step.
    let d = rect(&mut s, 10.7, 400.0, 20.0, 20.0);
    let e = rect(&mut s, 51.8, 450.0, 20.0, 20.0);
    s.execute("select.set", &json!({"ids": [d.0, e.0]})).unwrap();
    s.execute("object.align", &left).unwrap();
    assert!(x0(&s, e) != 10.7 && (x0(&s, e) - 10.7).abs() < 1e-12, "{}", x0(&s, e));
    s.doc_mut().unwrap().mark_saved();
    let before = history(&s);
    s.execute("object.align", &left).unwrap();
    assert_eq!(history(&s), before, "a pair the first Align aligned");
}

/// Distribute and Distribute Spacing over objects already evenly spaced move nothing and record no
/// undo step either.
#[test]
fn a_distribute_that_moves_nothing_records_no_undo_step() {
    let mut s = session();
    // The rectangles are 20 pt wide and 30 pt apart.
    let ids = [0.0, 50.0, 100.0].map(|x| rect(&mut s, x, 0.0, 20.0, 20.0));
    select_with_a_step_to_redo(&mut s, &ids);
    let before = history(&s);
    let calls = [
        ("object.distribute", json!({"horizontal": "left"})),
        ("object.distribute", json!({"horizontal": "center"})),
        ("object.distribute", json!({"horizontal": "right"})),
        ("object.distributeSpacing", json!({"axis": "horizontal"})),
        ("object.distributeSpacing", json!({"axis": "horizontal", "spacing": 30})),
    ];
    for (id, p) in calls {
        s.execute(id, &p).unwrap();
        assert_eq!(history(&s), before, "{id} {p}");
        assert_eq!(s.journal.last().unwrap(), &(id.to_string(), p));
    }
    // Distribute Spacing keeps the key object still and spaces the others from it.
    s.execute("select.key", &json!({"id": ids[1].0})).unwrap();
    assert_eq!(s.doc().unwrap().selection.key, Some(ids[1]));
    s.execute("object.distributeSpacing", &json!({"axis": "horizontal", "spacing": 30})).unwrap();
    assert_eq!(history(&s), before, "spaced 30 pt from the key object");
    // After the middle one moves 10 pt right, Distribute moves it back in one undo step.
    s.execute("select.set", &json!({"ids": [ids[1].0]})).unwrap();
    s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    s.execute("select.set", &json!({"ids": ids.map(|i| i.0)})).unwrap();
    let undo = history(&s).0;
    s.execute("object.distribute", &json!({"horizontal": "left"})).unwrap();
    assert_eq!(history(&s).0, undo + 1);
    assert_eq!(s.doc().unwrap().doc.node(ids[1]).unwrap().geometric_bounds().unwrap().x0, 50.0);
    // For rectangles 30.1 pt apart, the gap Distribute Spacing computes moves the third one by a
    // rounding error.
    let ids = [0.1, 50.2, 100.3].map(|x| rect(&mut s, x, 100.0, 20.0, 20.0));
    select_with_a_step_to_redo(&mut s, &ids);
    let before = history(&s);
    s.execute("object.distributeSpacing", &json!({"axis": "horizontal"})).unwrap();
    assert_eq!(history(&s), before, "30.1 pt apart");
    // Distributing these three leaves the middle one a rounding error off halfway between the
    // others, and a second Distribute records no step.
    let ids = [40.045, 433.482, 1935.587].map(|x| rect(&mut s, x, 200.0, 20.0, 20.0));
    s.execute("select.set", &json!({"ids": ids.map(|i| i.0)})).unwrap();
    s.execute("object.distribute", &json!({"horizontal": "left"})).unwrap();
    s.doc_mut().unwrap().mark_saved();
    let before = history(&s);
    s.execute("object.distribute", &json!({"horizontal": "left"})).unwrap();
    assert_eq!(history(&s), before, "three objects the first Distribute spaced");
}

/// Move by 0 pt, Rotate and Shear by 0°, Scale to 100%, an identity Transform, Set Bounds with the
/// current position and size, a Nudge of 0 (of objects, anchors or ruler guides), a Transform with
/// no object to transform and a Selection tool drag that ends where it started record no undo
/// step. Transform Again then repeats the identity, and a Nudge of 0 of a live rectangle's anchor
/// leaves the rectangle a live shape.
#[test]
fn a_transform_that_moves_nothing_records_no_undo_step() {
    let mut s = session();
    // The Nudges of a guide and of anchors below use this ruler guide and this plain path.
    s.execute("guide.add", &json!({"vertical": true, "pos": 600})).unwrap();
    let made = s.execute("path.create", &json!({"anchors": [{"x": 300, "y": 300}, {"x": 400, "y": 300}]})).unwrap();
    let path = NodeId(made["id"].as_u64().unwrap());
    let a = rect(&mut s, 10.0, 20.0, 40.0, 30.0);
    select_with_a_step_to_redo(&mut s, &[a]);
    let before = history(&s);
    let calls = [
        ("object.move", json!({"dx": 0, "dy": 0})),
        ("object.rotate", json!({"angle": 0})),
        ("object.scale", json!({"sx": 100})),
        ("object.shear", json!({"angle": 0})),
        ("object.transform", json!({"matrix": [1, 0, 0, 1, 0, 0]})),
        ("object.nudge", json!({"dx": 0, "dy": 0})),
        ("object.setBounds", json!({})),
        ("object.transformAgain", json!({})),
    ];
    for (id, p) in calls {
        assert_eq!(s.execute(id, &p).unwrap(), json!({"ids": [a.0]}), "{id} {p}");
        assert_eq!(history(&s), before, "{id} {p}");
        assert_eq!(s.journal.last().unwrap(), &(id.to_string(), p));
    }
    assert_eq!(s.doc().unwrap().last_transform, Some((Affine::IDENTITY, false)));
    // A Nudge of 0 with an anchor of the rectangle direct-selected keeps it a live shape.
    let is_live = |s: &Session| matches!(s.doc().unwrap().doc.node(a).unwrap().kind, NodeKind::Path { live: Some(_), .. });
    assert!(is_live(&s), "a rectangle is a live shape");
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 0]]})).unwrap();
    s.execute("object.nudge", &json!({"dx": 0, "dy": 0})).unwrap();
    assert!(is_live(&s), "a Nudge of 0 keeps it a live shape");
    assert_eq!(history(&s), before, "an anchor of the live rectangle, then object.nudge");
    // A Nudge of 0 with anchors direct-selected or with only a ruler guide selected, and a
    // Transform with nothing selected or with ids that name no object, with or without a copy,
    // record no undo step either.
    let calls = [
        ("select.anchors", json!({"id": path.0, "anchors": [[0, 0]]}), "object.nudge", json!({"dx": 0, "dy": 0})),
        ("guide.select", json!({"indexes": [0]}), "object.nudge", json!({"dx": 0, "dy": 0})),
        ("select.set", json!({"ids": []}), "object.transform", json!({"matrix": [1, 0, 0, 1, 5, 0]})),
        ("select.set", json!({"ids": []}), "object.transform", json!({"matrix": [1, 0, 0, 1, 5, 0], "copy": true})),
        ("select.set", json!({"ids": [a.0]}), "object.transform", json!({"matrix": [1, 0, 0, 1, 5, 0], "ids": [999999]})),
        ("select.set", json!({"ids": [a.0]}), "object.transform", json!({"matrix": [1, 0, 0, 1, 5, 0], "ids": [999999], "copy": true})),
    ];
    for (select, sp, id, p) in calls {
        s.execute(select, &sp).unwrap();
        s.execute(id, &p).unwrap();
        assert_eq!(history(&s), before, "{select} {sp}, then {id} {p}");
    }
    // A copy of no object lists no copies and keeps the selection.
    let p = json!({"matrix": [1, 0, 0, 1, 5, 0], "ids": [999999], "copy": true});
    assert_eq!(s.execute("object.transform", &p).unwrap(), json!({"ids": []}));
    assert_eq!(s.doc().unwrap().selection.objects, [a]);
    // A Selection tool drag that ends where it started records no undo step either. Back at the
    // start, the revision advances, so views redraw the object where it was.
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let x0 = |s: &Session| s.doc().unwrap().doc.node(a).unwrap().geometric_bounds().unwrap().x0;
    let revision = |s: &Session| s.doc().unwrap().revision;
    s.pointer(&PointerEvent::new(PointerKind::Down, 30.0, 35.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 60.0, 35.0), v).unwrap();
    assert_ne!(x0(&s), 10.0, "the preview moves it");
    let moved = revision(&s);
    s.pointer(&PointerEvent::new(PointerKind::Drag, 30.0, 35.0), v).unwrap();
    assert_eq!(x0(&s), 10.0);
    assert!(revision(&s) > moved, "views redraw it back at the start");
    s.pointer(&PointerEvent::new(PointerKind::Up, 30.0, 35.0), v).unwrap();
    assert_eq!(history(&s), before, "a drag back to the start");
    assert_eq!(s.journal.last().unwrap().0, "object.transform");
    // With a copy, a Move by 0 pt duplicates the object in one undo step.
    let count = |s: &Session| s.doc().unwrap().doc.layers[0].children().unwrap().len();
    let n = count(&s);
    s.execute("object.move", &json!({"dx": 0, "dy": 0, "copy": true})).unwrap();
    assert_eq!(history(&s), (before.0 + 1, 0, true));
    assert_eq!(count(&s), n + 1);
}

/// Arrange commands that leave the stacking order as it is record no undo step: Bring to Front and
/// Bring Forward on the front objects, Send to Back and Send Backward on the back ones, and Send to
/// Current Layer on objects already on top of the current layer.
#[test]
fn an_arrange_that_changes_no_order_records_no_undo_step() {
    let mut s = session();
    let ids = [0.0, 30.0, 60.0].map(|x| rect(&mut s, x, 0.0, 20.0, 20.0));
    let order = |s: &Session| s.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id).collect::<Vec<_>>();
    let calls = [
        (vec![ids[2]], "object.arrange.bringToFront"),
        (vec![ids[2]], "object.arrange.bringForward"),
        (vec![ids[0]], "object.arrange.sendToBack"),
        (vec![ids[0]], "object.arrange.sendBackward"),
        (vec![ids[2]], "object.arrange.sendToCurrentLayer"),
        (vec![ids[1], ids[2]], "object.arrange.bringToFront"),
        (vec![ids[1], ids[2]], "object.arrange.bringForward"),
        (vec![ids[0], ids[1]], "object.arrange.sendToBack"),
        (vec![ids[0], ids[1]], "object.arrange.sendBackward"),
        (vec![ids[1], ids[2]], "object.arrange.sendToCurrentLayer"),
    ];
    for (sel, c) in calls {
        select_with_a_step_to_redo(&mut s, &sel);
        let before = history(&s);
        s.execute(c, &json!({})).unwrap();
        assert_eq!(history(&s), before, "{c} {sel:?}");
        assert_eq!(order(&s), ids, "{c} {sel:?}");
    }
    // Bring to Front on the back object is one undo step, and it clears Redo.
    select_with_a_step_to_redo(&mut s, &[ids[0]]);
    let undo = history(&s).0;
    s.execute("object.arrange.bringToFront", &json!({})).unwrap();
    assert_eq!(history(&s), (undo + 1, 0, true));
    assert_eq!(order(&s), [ids[1], ids[2], ids[0]]);
}

/// Ungroup, Release Compound Path and Release Clipping Mask with none of their objects selected,
/// and Expand Shape with no live shape selected, record no undo step and keep the selection and its
/// key object.
#[test]
fn ungroup_release_and_expand_shape_with_nothing_to_act_on_record_no_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 20.0, 20.0);
    let b = rect(&mut s, 40.0, 0.0, 20.0, 20.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    // Expand Shape makes the two rectangles plain paths in one undo step.
    let undo = history(&s).0;
    s.execute("object.expandShape", &json!({})).unwrap();
    assert_eq!(history(&s).0, undo + 1);
    select_with_a_step_to_redo(&mut s, &[a, b]);
    s.execute("select.key", &json!({"id": a.0})).unwrap();
    let (before, selection) = (history(&s), s.doc().unwrap().selection.clone());
    assert_eq!(selection.key, Some(a));
    for c in ["object.ungroup", "object.compoundPath.release", "object.clippingMask.release", "object.expandShape"] {
        s.execute(c, &json!({})).unwrap();
        assert_eq!(history(&s), before, "{c}");
        assert_eq!(s.doc().unwrap().selection, selection, "{c}");
        assert_eq!(s.journal.last().unwrap().0, c);
    }
    // Ungroup with a group selected is one undo step.
    s.execute("object.group", &json!({})).unwrap();
    let undo = history(&s).0;
    s.execute("object.ungroup", &json!({})).unwrap();
    assert_eq!(history(&s).0, undo + 1);
}

/// Unlock All with nothing locked and Show All with nothing hidden record no undo step and keep the
/// selection. Lock with every selected object already locked and Hide with every one already
/// hidden record no undo step and deselect them.
#[test]
fn lock_hide_unlock_all_and_show_all_with_nothing_to_do_record_no_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 20.0, 20.0);
    select_with_a_step_to_redo(&mut s, &[a]);
    let before = history(&s);
    for c in ["object.unlockAll", "object.showAll"] {
        assert_eq!(s.execute(c, &json!({})).unwrap(), json!({"count": 0}), "{c}");
        assert_eq!(history(&s), before, "{c}");
        assert_eq!(s.doc().unwrap().selection.objects, [a], "{c}");
    }
    // With the object locked or hidden, each is one undo step that selects it.
    for (off, on) in [("object.lock", "object.unlockAll"), ("object.hide", "object.showAll")] {
        s.execute(off, &json!({})).unwrap();
        let undo = history(&s).0;
        assert_eq!(s.execute(on, &json!({})).unwrap(), json!({"count": 1}), "{on}");
        assert_eq!(history(&s).0, undo + 1, "{on}");
        assert_eq!(s.doc().unwrap().selection.objects, [a], "{on}");
    }
    // Object Properties locks (then hides) a new rectangle `b`, and `select.set` selects it.
    // Lock (then Hide) deselects it and records no undo step. With `a` selected too, it is one
    // undo step.
    for (c, props) in [("object.lock", json!({"locked": true})), ("object.hide", json!({"visible": false}))] {
        let b = rect(&mut s, 40.0, 0.0, 20.0, 20.0);
        s.execute("object.setProps", &props).unwrap();
        select_with_a_step_to_redo(&mut s, &[a]);
        s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
        let before = history(&s);
        s.execute(c, &json!({})).unwrap();
        assert_eq!(history(&s), before, "{c}");
        assert!(s.doc().unwrap().selection.objects.is_empty(), "{c}");
        s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
        s.execute(c, &json!({})).unwrap();
        assert_eq!(history(&s), (before.0 + 1, 0, true), "{c} with a");
        s.execute("edit.undo", &json!({})).unwrap();
    }
}

/// Set Bounds with the current position and size records no undo step on a rotated object, with
/// and without Use Preview Bounds, and on an unrotated object with Use Preview Bounds. In these
/// cases computing the transform can leave a rounding error in it.
#[test]
fn set_bounds_with_the_current_position_and_size_records_no_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 20.0, 40.0, 30.0);
    s.execute("stroke.set", &json!({"weight": 5})).unwrap();
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    let c = rect(&mut s, 0.1, 0.05, 10.1, 6.06);
    s.execute("stroke.set", &json!({"weight": 0.3})).unwrap();
    select_with_a_step_to_redo(&mut s, &[a]);
    let before = history(&s);
    for (id, preview) in [(a, false), (a, true), (c, true)] {
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("prefs.set", &json!({"key": "usePreviewBounds", "value": preview})).unwrap();
        // The Transform panel's values: X and Y place the reference point, W and H are the sides.
        let b = s.transform_box(&[id]).unwrap();
        let (rp, w, h) = (b.reference_point(4), b.rect.width(), b.rect.height());
        for p in [json!({}), json!({"x": rp.x, "y": rp.y, "width": w, "height": h}), json!({"width": w, "proportional": true})] {
            assert_eq!(s.execute("object.setBounds", &p).unwrap(), json!({"ids": [id.0]}), "{p}");
            assert_eq!(history(&s), before, "{id:?}, Use Preview Bounds {preview}: {p}");
            assert_eq!(s.journal.last().unwrap().0, "object.setBounds");
        }
    }
    // A new width is one undo step, and it clears Redo.
    s.execute("object.setBounds", &json!({"width": 50})).unwrap();
    assert_eq!(history(&s), (before.0 + 1, 0, true));
}

/// Object Properties with the values the objects already have records no undo step: every
/// property, data already set or already absent, and no object. A value that differs on one of
/// the objects is one undo step, and an id that names no object is still an error.
#[test]
fn object_properties_already_set_record_no_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 20.0, 20.0);
    let b = rect(&mut s, 40.0, 0.0, 20.0, 20.0);
    let set = json!({"ids": [a.0], "name": "A", "opacity": 50, "blend": "Multiply", "knockout": "on", "data": {"pivot": "1,2"}});
    s.execute("object.setProps", &set).unwrap();
    select_with_a_step_to_redo(&mut s, &[a, b]);
    let before = history(&s);
    let defaults = json!({
        "id": b.0, "name": "", "visible": true, "locked": false, "opacity": 100, "blend": "Normal", "isolate": false,
        "knockout": "neutral", "knockoutShape": false, "data": {"pivot": null}
    });
    let calls =
        [set, defaults, json!({"id": a.0, "data": {"data-pivot": "1,2", "other": null}}), json!({"visible": true}), json!({}), json!({"ids": []})];
    for p in calls {
        s.execute("object.setProps", &p).unwrap();
        assert_eq!(history(&s), before, "{p}");
        assert_eq!(s.journal.last().unwrap(), &("object.setProps".to_string(), p));
    }
    // `a` already has 50% opacity, and `b` does not.
    s.execute("object.setProps", &json!({"opacity": 50})).unwrap();
    assert_eq!(history(&s), (before.0 + 1, 0, true));
    assert!(s.execute("object.setProps", &json!({"ids": [999999], "visible": true})).is_err());
}

/// Live Shape Properties that a shape already has record no undo step: a polygon's sides, angle,
/// radius, side length and equal sides, an ellipse's pie, Invert Pie on a whole ellipse, and the
/// radius and kind its corners have (given corners, the Direct-Selected one, or all). A value that
/// differs is one undo step. Setting the radius of a rectangle that kept a scale in its transform
/// is one undo step too, because it folds the scale into the rectangle's size.
#[test]
fn live_shape_properties_already_set_record_no_undo_step() {
    let mut s = session();
    let id = |v: Value| NodeId(v["id"].as_u64().unwrap());
    let r = rect(&mut s, 0.0, 0.0, 100.0, 60.0);
    let polygon = id(s.execute("shape.polygon", &json!({"cx": 300, "cy": 100, "radius": 50, "sides": 6})).unwrap());
    let ellipse = id(s.execute("shape.ellipse", &json!({"x": 0, "y": 200, "width": 80, "height": 40})).unwrap());
    let star = id(s.execute("shape.star", &json!({"cx": 300, "cy": 300, "radius1": 60, "radius2": 30})).unwrap());
    // The rectangle's corners get 8 pt radii, its top-right corner gets 12 pt, and the polygon gets
    // 30 pt sides.
    for p in [json!({"id": r.0, "radius": 8}), json!({"id": r.0, "corners": [1], "radius": 12}), json!({"id": polygon.0, "sideLength": 30})] {
        s.execute("object.setLiveShape", &p).unwrap();
    }
    let NodeKind::Path { live: Some(l), .. } = &s.doc().unwrap().doc.node(polygon).unwrap().kind else { panic!("not a live polygon") };
    let (angle, radius) = (l.polygon_angle().unwrap(), l.polygon_radius().unwrap());
    select_with_a_step_to_redo(&mut s, &[r, polygon, ellipse, star]);
    let before = history(&s);
    let calls = [
        json!({"id": polygon.0, "sides": 6, "polygonAngle": angle, "polygonRadius": radius, "makeSidesEqual": true}),
        json!({"id": polygon.0, "sideLength": 30, "kind": "round"}),
        json!({"id": ellipse.0, "pieStart": 0, "pieEnd": 360}),
        json!({"id": ellipse.0, "invertPie": true}),
        json!({"id": r.0, "corners": [0, 2, 3], "radius": 8, "kind": "round"}),
        json!({"items": [{"id": r.0, "corners": [1]}], "radius": 12}),
        json!({"id": star.0, "radius": 0}),
    ];
    for p in calls {
        s.execute("object.setLiveShape", &p).unwrap();
        assert_eq!(history(&s), before, "{p}");
        assert_eq!(s.journal.last().unwrap(), &("object.setLiveShape".to_string(), p));
    }
    // With an anchor of the top-left corner direct-selected, the radius applies to that corner.
    s.execute("select.anchors", &json!({"id": r.0, "anchors": [[0, 0]]})).unwrap();
    s.execute("object.setLiveShape", &json!({"radius": 8})).unwrap();
    assert_eq!(history(&s), before, "the Direct-Selected corner");
    for p in [json!({"id": polygon.0, "sides": 7}), json!({"id": ellipse.0, "pieEnd": 90}), json!({"radius": 9})] {
        let undo = history(&s).0;
        s.execute("object.setLiveShape", &p).unwrap();
        assert_eq!(history(&s).0, undo + 1, "{p}");
    }
    // The rectangle is now 50 × 60 pt with 10 pt corners, stretched to 100 × 60 pt by its
    // transform, as files from before #291 saved it. Setting the radius its corners show makes the
    // corners circles.
    {
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        let NodeKind::Path { path, live: Some(live), .. } = &mut d.node_mut(r).unwrap().kind else { panic!("not a live rectangle") };
        let xf = Affine::scale_non_uniform(2.0, 1.0);
        *live = vectorcraft_doc::LiveShape::Rectangle { w: 50.0, h: 60.0, radii: [10.0; 4], kinds: Default::default(), xf };
        *path = live.to_path();
    }
    let shown = vectorcraft_doc::LiveCorners::of(s.doc().unwrap().doc.node(r).unwrap()).unwrap().radius(0);
    let undo = history(&s).0;
    s.execute("object.setLiveShape", &json!({"id": r.0, "radius": shown})).unwrap();
    assert_eq!(history(&s).0, undo + 1, "a stretched rectangle");
}

#[test]
fn rectangle_tool_draws() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("rectangle", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 10.0, 10.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 60.0, 40.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 110.0, 60.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 110.0, 60.0), v).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers[0].children().unwrap().len(), 1);
    let b = d.layers[0].children().unwrap()[0].geometric_bounds().unwrap();
    assert_eq!((b.width(), b.height()), (100.0, 50.0));
    assert_eq!(s.doc().unwrap().history.undo.len(), 1);
}

#[test]
fn pen_tool_draws_closed_path() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("pen", v).unwrap();
    for (x, y) in [(10.0, 10.0), (100.0, 10.0), (100.0, 100.0)] {
        s.pointer(&PointerEvent::new(PointerKind::Down, x, y), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, x, y), v).unwrap();
    }
    s.pointer(&PointerEvent::new(PointerKind::Down, 10.0, 10.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 10.0, 10.0), v).unwrap();
    let d = &s.doc().unwrap().doc;
    let n = &d.layers[0].children().unwrap()[0];
    let p = n.path_data().unwrap();
    assert_eq!(p.anchor_count(), 3);
    assert!(p.is_closed());
}

/// Auto Add/Delete: a Pen click on a selected path's segment adds an anchor, unless General →
/// Disable Auto Add/Delete is on.
#[test]
fn pen_click_on_a_selected_path_adds_an_anchor_unless_disabled() {
    let mut s = session();
    let v = ViewInfo::default();
    let r = s.execute("path.create", &json!({"anchors": [{"x": 10, "y": 100}, {"x": 110, "y": 100}, {"x": 210, "y": 100}]})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.all", &json!({})).unwrap();
    s.select_tool("pen", v).unwrap();
    let click = |s: &mut Session| {
        s.pointer(&PointerEvent::new(PointerKind::Down, 60.0, 101.0), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, 60.0, 101.0), v).unwrap();
    };
    click(&mut s);
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers[0].children().unwrap().len(), 1);
    assert_eq!(d.node(id).unwrap().path_data().unwrap().anchor_count(), 4);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("prefs.set", &json!({"key": "disableAutoAddDelete", "value": true})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    click(&mut s);
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers[0].children().unwrap().len(), 2, "a new path starts");
    assert_eq!(d.node(id).unwrap().path_data().unwrap().anchor_count(), 3);
}

#[test]
fn direct_selection_moves_one_anchor() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("select.none", &json!({})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("directSelection", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 90.0, 90.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 90.0, 90.0), v).unwrap();
    let p = s.doc().unwrap().doc.node(a).unwrap().path_data().unwrap().clone();
    assert_eq!(p.subpaths[0].anchors[0].p, vectorcraft_geom::Point::new(90.0, 90.0));
    assert_eq!(p.subpaths[0].anchors[1].p, vectorcraft_geom::Point::new(200.0, 100.0));
}

/// Dragging a curved segment with Direct Selection bends it (its anchors stay), and dragging a
/// straight one moves its two anchors, not the whole path; each is one undo step.
#[test]
fn direct_selection_drags_segments() {
    let mut s = session();
    let v = ViewInfo::default();
    let drag = |s: &mut Session, from: (f64, f64), to: (f64, f64)| {
        s.pointer(&PointerEvent::new(PointerKind::Down, from.0, from.1), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Drag, to.0, to.1), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, to.0, to.1), v).unwrap();
    };
    let arch = &json!({"anchors": [{"x": 100, "y": 200, "out": [100, 100]}, {"x": 300, "y": 200, "in": [300, 100]}]});
    let arch = NodeId(s.execute("path.create", arch).unwrap()["id"].as_u64().unwrap());
    let square = &json!({"anchors": [{"x": 150, "y": 400}, {"x": 150, "y": 300}, {"x": 250, "y": 300}, {"x": 250, "y": 400}], "closed": true});
    let square = NodeId(s.execute("path.create", square).unwrap()["id"].as_u64().unwrap());
    s.execute("select.none", &json!({})).unwrap();
    s.select_tool("directSelection", v).unwrap();
    let steps = |s: &Session| s.doc().unwrap().history.undo.len();
    let before = steps(&s);
    // The arch's middle, 40 up: the curve bends, its two anchors stay.
    drag(&mut s, (200.0, 125.0), (200.0, 85.0));
    let sp = s.doc().unwrap().doc.node(arch).unwrap().path_data().unwrap().subpaths[0].clone();
    let (a, b) = (sp.anchors[0], sp.anchors[1]);
    assert_eq!((a.p, b.p), (vectorcraft_geom::Point::new(100.0, 200.0), vectorcraft_geom::Point::new(300.0, 200.0)), "the anchors stay");
    assert!(a.h_out.y < 100.0 && b.h_in.y < 100.0, "the curve bent up: {a:?} {b:?}");
    assert_eq!(steps(&s), before + 1);
    // The square's top edge, 40 up: its two anchors move, the bottom two stay.
    drag(&mut s, (200.0, 300.0), (200.0, 260.0));
    let sq = s.doc().unwrap().doc.node(square).unwrap().geometric_bounds().unwrap();
    assert_eq!((sq.y0, sq.y1, sq.x0, sq.x1), (260.0, 400.0, 150.0, 250.0), "the edge moved, the rest stayed");
    assert_eq!(steps(&s), before + 2);
    // Its fill still moves the whole square.
    drag(&mut s, (200.0, 350.0), (200.0, 330.0));
    let sq = s.doc().unwrap().doc.node(square).unwrap().geometric_bounds().unwrap();
    assert_eq!((sq.y0, sq.y1), (240.0, 380.0), "the whole path moved");
}

#[test]
fn text_create_has_bounds() {
    let mut s = session();
    let r = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Hello VectorCraft", "size": 24})).unwrap();
    let n = s.doc().unwrap().doc.node(NodeId(r["id"].as_u64().unwrap())).unwrap().clone();
    let b = n.geometric_bounds().unwrap();
    assert!(b.width() > 100.0, "{b:?}");
}

#[test]
fn inspect_lists_everything() {
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let v = s.execute("document.inspect", &json!({})).unwrap();
    assert_eq!(v["layers"][0]["children"][0]["kind"], "Rectangle");
    assert!(s.commands().len() > 100);
}

/// `document.node {summary: true}` answers the `document.inspect` shape for one node:
/// the same fields the full-tree summary carries, without serializing the whole tree.
#[test]
fn document_node_summary_matches_inspect() {
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 10.0, 10.0).0;
    let full = s.execute("document.node", &json!({"id": id})).unwrap();
    assert!(full.get("bounds").is_none(), "full object JSON carries geometry, not bounds: {full}");
    let summary = s.execute("document.node", &json!({"id": id, "summary": true})).unwrap();
    let v = s.execute("document.inspect", &json!({})).unwrap();
    let node = v["layers"][0]["children"].as_array().unwrap().iter().find(|n| n["id"] == id).cloned().unwrap();
    assert_eq!(summary, node, "{summary} vs {node}");
    assert!(s.execute("document.node", &json!({"id": 999999, "summary": true})).is_err());
}

/// `depth` and `childLimit` slice a summary read; a level that shows fewer children
/// than it has reports `childCount`, and invalid values are rejected, not reinterpreted.
#[test]
fn document_node_summary_slices_with_depth_and_limit() {
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    s.execute("select.all", &json!({})).unwrap();
    let inner = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    rect(&mut s, 40.0, 0.0, 10.0, 10.0);
    s.execute("select.all", &json!({})).unwrap();
    let outer = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    let mut read = |p: Value| s.execute("document.node", &p).unwrap();

    let g = read(json!({"id": outer, "summary": true}));
    assert_eq!(g["children"].as_array().map(Vec::len), Some(2));
    assert!(g.get("childCount").is_none(), "nothing truncated: {g}");
    // Limits beyond the tree are the whole tree, byte for byte.
    assert_eq!(read(json!({"id": outer, "summary": true, "depth": u64::MAX, "childLimit": u64::MAX})), g);

    let one = read(json!({"id": outer, "summary": true, "childLimit": 1}));
    assert_eq!(one["children"].as_array().map(Vec::len), Some(1));
    assert_eq!(one["childCount"], 2);

    let flat = read(json!({"id": outer, "summary": true, "depth": 0}));
    assert!(flat.get("children").is_none(), "{flat}");
    assert_eq!(flat["childCount"], 2);

    // Limits compose per level: the outer children show; the inner group's own
    // children don't, and it says so.
    let shallow = read(json!({"id": outer, "summary": true, "depth": 1}));
    let kids = shallow["children"].as_array().unwrap();
    assert_eq!(kids.len(), 2);
    assert!(shallow.get("childCount").is_none(), "{shallow}");
    let nested = kids.iter().find(|n| n["id"] == inner).unwrap();
    assert!(nested.get("children").is_none(), "{nested}");
    assert_eq!(nested["childCount"], 2);

    for bad in [
        json!({"id": outer, "summary": true, "depth": -1}),
        json!({"id": outer, "summary": true, "depth": 1.5}),
        json!({"id": outer, "summary": true, "childLimit": "many"}),
        // A slice of the full object JSON would silently be the whole subtree.
        json!({"id": outer, "depth": 0}),
        json!({"id": outer, "summary": false, "childLimit": 1}),
    ] {
        assert!(s.execute("document.node", &bad).is_err(), "{bad}");
    }
}

/// `document.inspect` slices the layer tree with the same options; artboards and the
/// rest always come whole, and the default output is unchanged.
#[test]
fn document_inspect_slices_the_layer_tree() {
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    s.execute("select.all", &json!({})).unwrap();
    s.execute("object.group", &json!({})).unwrap();

    let full = s.execute("document.inspect", &json!({})).unwrap();
    assert!(full["layers"][0].get("childCount").is_none(), "{full}");
    let flat = s.execute("document.inspect", &json!({"depth": 0})).unwrap();
    assert!(flat["layers"][0].get("children").is_none(), "{flat}");
    assert_eq!(flat["layers"][0]["childCount"], 1);
    assert_eq!(flat["artboards"], full["artboards"]);
    assert_eq!(flat["objects"], full["objects"]);
    let one = s.execute("document.inspect", &json!({"childLimit": 0})).unwrap();
    assert_eq!(one["layers"][0]["children"], json!([]));
    assert_eq!(one["layers"][0]["childCount"], 1);
    assert!(s.execute("document.inspect", &json!({"depth": "deep"})).is_err());
}

/// `document.find` searches names, kinds and type content across the whole tree and
/// answers ids with ancestor paths plus the total, so capped replies stay explicit.
#[test]
fn document_find_searches_names_kinds_and_text() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0).0;
    s.execute("object.setProps", &json!({"ids": [a], "name": "Hero Banner"})).unwrap();
    rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    s.execute("select.all", &json!({})).unwrap();
    let group = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    let mut find = |p: Value| s.execute("document.find", &p).unwrap();

    let v = find(json!({"name": "hero"}));
    assert_eq!(v["total"], 1);
    assert_eq!(v["matches"][0]["id"], a);
    assert_eq!(v["matches"][0]["path"], json!([1, group]));

    let v = find(json!({"kind": "group"}));
    assert_eq!(v["total"], 1);
    assert_eq!(v["matches"][0]["id"], group);

    // `limit: 0` counts without listing; a missing filter is rejected, not a full dump.
    let v = find(json!({"kind": "rectangle", "limit": 0}));
    assert_eq!(v["matches"].as_array().map(Vec::len), Some(0));
    assert_eq!(v["total"], 2);
    assert!(s.execute("document.find", &json!({})).is_err());
    assert!(s.execute("document.find", &json!({"name": ""})).is_err());
    assert!(s.execute("document.find", &json!({"name": "hero", "limit": "many"})).is_err());
    // A filter of the wrong type is rejected, not dropped (dropping it would widen the search).
    assert!(s.execute("document.find", &json!({"kind": "group", "name": 5})).is_err());
    assert_eq!(s.execute("document.find", &json!({"kind": "nope"})).unwrap()["total"], 0);
}

/// Type reports the paint its characters show, and its own object-level paint apart from it.
#[test]
fn inspect_reports_the_paint_of_types_characters() {
    let mut s = session();
    let id = s.execute("text.create", &json!({"x": 50, "y": 60, "text": "HI", "size": 24})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000", "ids": [id]})).unwrap();
    let node = |s: &mut Session| {
        let v = s.execute("document.inspect", &json!({})).unwrap();
        v["layers"][0]["children"].as_array().unwrap().iter().find(|n| n["id"] == id).cloned().unwrap()
    };
    let t = node(&mut s);
    assert_eq!((t["fill"].as_str(), t["stroke"].as_str(), t["strokeWidth"].as_f64()), (Some("#ff0000"), Some("None"), Some(0.0)), "{t}");
    assert!(t.get("objectFill").is_none(), "no object-level paint: {t}");
    s.execute("paint.setFill", &json!({"color": "#0000ff", "ids": [id]})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#00ff00", "ids": [id]})).unwrap();
    s.execute("stroke.set", &json!({"weight": 3, "ids": [id]})).unwrap();
    let t = node(&mut s);
    assert_eq!((t["fill"].as_str(), t["stroke"].as_str(), t["strokeWidth"].as_f64()), (Some("#0000ff"), Some("#00ff00"), Some(3.0)), "{t}");
    // An object-level fill on the type stays distinguishable from its characters' paint.
    s.execute("appearance.addFill", &json!({"ids": [id]})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ffff00", "item": 0, "ids": [id]})).unwrap();
    let t = node(&mut s);
    assert_eq!(t["objectFill"], "#ffff00", "{t}");
    assert_eq!(t["fill"], "#0000ff", "{t}");
    // A path keeps reporting its own appearance.
    let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000", "ids": [r]})).unwrap();
    let v = s.execute("document.inspect", &json!({})).unwrap();
    let rect = v["layers"][0]["children"].as_array().unwrap().iter().find(|n| n["id"] == r).cloned().unwrap();
    assert_eq!(rect["fill"], "#ff0000");
    assert!(rect.get("objectFill").is_none());
}

#[test]
fn every_command_has_unique_id_and_doc() {
    let mut ids: Vec<&str> = command_specs().iter().map(|c| c.id).collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n);
    assert!(command_specs().iter().all(|c| !c.params.is_empty() && !c.label.is_empty()));
}

#[test]
fn every_command_survives_empty_params() {
    // Robustness: no command may panic on {} (with and without a selection).
    // The view toggles among them flip the process-wide proof view.
    let _proof_view = crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    for sel in [false, true] {
        for c in command_specs() {
            let mut s = session();
            let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
            rect(&mut s, 20.0, 0.0, 10.0, 10.0);
            s.execute("edit.copy", &json!({})).ok();
            if sel {
                s.execute("select.all", &json!({})).unwrap();
            } else {
                s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
            }
            let _ = s.execute(c.id, &json!({}));
        }
    }
}

#[test]
fn flare_tool_two_step_gesture_is_one_undo() {
    let mut s = session();
    let v = ViewInfo::default();
    let undo_before = s.doc().unwrap().history.undo.len();
    s.select_tool("flare", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 200.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 240.0, 200.0), v).unwrap();
    // ↓ twice: 13 rays (one compound path with a subpath per ray).
    s.tool_key(ToolKey::Down, Default::default(), v).unwrap();
    s.tool_key(ToolKey::Down, Default::default(), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 240.0, 200.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Move, 500.0, 400.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 500.0, 400.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 500.0, 400.0), v).unwrap();
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), undo_before + 1);
    let g = st.doc.node(st.selection.objects[0]).unwrap();
    assert_eq!(g.name.as_deref(), Some("Flare"));
    // Halo + rays + centre + 10 rings; the rings reach towards the end point.
    let NodeKind::Group { children, .. } = &g.kind else { panic!("flare is a group") };
    assert_eq!(children.len(), 13);
    let b = g.geometric_bounds().unwrap();
    assert!(b.x1 > 400.0 && b.y1 > 300.0, "{b:?}");
    let centre = children.iter().find(|c| c.name.as_deref() == Some("Center")).unwrap().geometric_bounds().unwrap();
    assert!((centre.width() - 80.0).abs() < 0.5);
    let rays = children.iter().find(|c| c.name.as_deref() == Some("Rays")).unwrap();
    assert_eq!(rays.path_data().unwrap().subpaths.len(), 13);
}

#[test]
fn reshape_moves_the_grabbed_point_and_its_neighbourhood() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    // Grab the middle of the top edge: a new anchor appears there and moves the full delta.
    s.execute("path.reshape", &json!({"id": r.0, "x": 50.0, "y": 0.0, "dx": 0.0, "dy": -40.0})).unwrap();
    let st = s.doc().unwrap();
    let pd = st.doc.node(r).unwrap().path_data().unwrap().clone();
    assert_eq!(pd.anchor_count(), 5);
    let pts: Vec<_> = pd.anchors().map(|(_, _, a)| a.p).collect();
    assert!(pts.iter().any(|p| (p.x - 50.0).abs() < 1e-6 && (p.y + 40.0).abs() < 1e-6), "{pts:?}");
    // Top corners follow partially, bottom corners (far away) stay put.
    let tl = pts.iter().find(|p| p.x < 1.0 && p.y < 0.0).expect("top-left moved up");
    assert!(tl.y > -40.0);
    assert!(pts.iter().filter(|p| (p.y - 100.0).abs() < 1e-9).count() == 2);
}

#[test]
fn graph_tool_drag_creates_a_graph_and_asks_for_data() {
    let mut s = session();
    let v = ViewInfo::default();
    let undo_before = s.doc().unwrap().history.undo.len();
    s.select_tool("pieGraph", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 100.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 300.0, 250.0), v).unwrap();
    let ui = s.pointer(&PointerEvent::new(PointerKind::Up, 300.0, 250.0), v).unwrap();
    assert!(ui.iter().any(|r| matches!(r, crate::UiRequest::Dialog(k, _) if k == "graphData")), "{ui:?}");
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), undo_before + 1);
    let g = st.doc.node(st.selection.objects[0]).unwrap();
    assert_eq!(g.graph.as_ref().unwrap().kind, vectorcraft_doc::GraphKind::Pie);
    assert_eq!(g.graph.as_ref().unwrap().rect, vectorcraft_geom::Rect::new(100.0, 100.0, 300.0, 250.0));
}

#[test]
fn stale_current_layer_id_never_targets_a_reused_id() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 5.0, 5.0, 10.0, 10.0);
    let layer = NodeId(s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("edit.undo", &json!({})).unwrap();
    // The undone layer's id is handed out again, here to a compound path.
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let c = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    assert_eq!(c, layer, "precondition: id reused");
    let d = rect(&mut s, 50.0, 50.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [d.0]})).unwrap();
    s.execute("object.arrange.sendToCurrentLayer", &json!({})).unwrap();
    let doc = &s.doc().unwrap().doc;
    assert!(doc.node(c).unwrap().children().unwrap().iter().all(|n| n.path_data().is_some()));
    assert!(doc.parent_of(d).is_some_and(|p| doc.node(p).unwrap().is_layer()));
    // layer.delete without an id must not delete the compound path.
    s.execute("layer.delete", &json!({})).ok();
    assert!(s.doc().unwrap().doc.node(c).is_some());
}

#[test]
fn object_mosaic_tiles_follow_the_image_colours() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"color": "#ff0000", "ids": [a.0]})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true, "ids": [a.0]})).unwrap();
    let b = rect(&mut s, 50.0, 0.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"color": "#0000ff", "ids": [b.0]})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true, "ids": [b.0]})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.rasterize", &json!({"ppi": 72})).unwrap();
    let r = s.execute("object.createObjectMosaic", &json!({"columns": 2, "rows": 1, "spacingX": 4, "deleteRaster": true})).unwrap();
    assert_eq!(r["tiles"], json!(2));
    let st = s.doc().unwrap();
    let g = st.doc.node(NodeId(r["id"].as_u64().unwrap())).unwrap();
    let tiles = g.children().unwrap();
    let fill = |n: &vectorcraft_doc::Node| n.appearance.fill_paint();
    assert_eq!(fill(&tiles[0]), vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0)));
    assert_eq!(fill(&tiles[1]), vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb(0.0, 0.0, 1.0)));
    // 4 pt spacing: each 50 pt tile shrinks to 46 pt; the image is gone.
    assert!((tiles[0].geometric_bounds().unwrap().width() - 46.0).abs() < 1e-6);
    assert!(!st.doc.layers[0].children().unwrap().iter().any(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Image(_))));
}

#[test]
fn snap_to_pixel_rounds_drawing_and_moves() {
    let mut s = session();
    let v = ViewInfo { snap_to_pixel: true, smart_guides: false, ..Default::default() };
    s.select_tool("rectangle", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 10.3, 10.6), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 60.4, 40.2), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 60.4, 40.2), v).unwrap();
    let id = s.doc().unwrap().selection.objects[0];
    let b = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (10.0, 11.0, 60.0, 40.0));
    s.select_tool("selection", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 30.0, 30.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 37.3, 34.8), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 37.3, 34.8), v).unwrap();
    let b = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
    assert_eq!((b.x0, b.y0), (17.0, 16.0));
}

#[test]
fn shaper_turns_rough_strokes_into_live_shapes_and_scribbles_punch_fills() {
    let mut s = session();
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    s.select_tool("shaper", v).unwrap();
    let stroke = |s: &mut Session, pts: &[(f64, f64)]| {
        s.pointer(&PointerEvent::new(PointerKind::Down, pts[0].0, pts[0].1), v).unwrap();
        for p in &pts[1..] {
            s.pointer(&PointerEvent::new(PointerKind::Drag, p.0, p.1), v).unwrap();
        }
        let l = pts[pts.len() - 1];
        s.pointer(&PointerEvent::new(PointerKind::Up, l.0, l.1), v).unwrap();
    };
    // A rough rectangle.
    let mut rect = vec![];
    for (a, b) in
        [((100.0, 100.0), (300.0, 102.0)), ((300.0, 102.0), (298.0, 200.0)), ((298.0, 200.0), (101.0, 199.0)), ((101.0, 199.0), (103.0, 104.0))]
    {
        for i in 0..20 {
            let t = i as f64 / 20.0;
            rect.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
    }
    stroke(&mut s, &rect);
    let id = s.doc().unwrap().selection.objects[0];
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    assert!(
        matches!(n.kind, vectorcraft_doc::NodeKind::Path { live: Some(vectorcraft_doc::LiveShape::Rectangle { .. }), .. }),
        "{:?}",
        n.kind_label()
    );
    // A zig-zag inside it clears its fill while retaining the editable original and stroke.
    let zig: Vec<(f64, f64)> = (0..30).map(|i| (150.0 + (i % 2) as f64 * 80.0, 120.0 + i as f64 * 2.0)).collect();
    stroke(&mut s, &zig);
    assert_eq!(s.doc().unwrap().doc.node(id).unwrap(), &n);
    let group = s.doc().unwrap().doc.node(s.doc().unwrap().selection.objects[0]).unwrap();
    assert!(group.shaper.is_some());
    assert!(group.children().unwrap().iter().skip(1).all(|n| n.appearance.fill_paint().is_none()));
    assert!(group.children().unwrap().iter().skip(1).any(|n| !n.appearance.stroke_paint().is_none()));
}

#[test]
fn shaper_square_started_mid_edge_closes_on_pointer_release_and_undoes_once() {
    let mut s = session();
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    s.select_tool("shaper", v).unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    for (kind, x, y) in [
        (PointerKind::Down, 150.0, 100.0),
        (PointerKind::Drag, 200.0, 105.0),
        (PointerKind::Drag, 194.0, 200.0),
        (PointerKind::Drag, 110.0, 191.0),
        (PointerKind::Drag, 100.0, 100.0),
        (PointerKind::Up, 150.0, 100.0),
    ] {
        s.pointer(&PointerEvent::new(kind, x, y), v).unwrap();
    }
    let id = *s.doc().unwrap().selection.objects.first().expect("recognized square");
    assert!(matches!(
        s.doc().unwrap().doc.node(id).unwrap().kind,
        vectorcraft_doc::NodeKind::Path { live: Some(vectorcraft_doc::LiveShape::Rectangle { .. }), .. }
    ));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(id).is_none());
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(id).is_some());
}

#[test]
fn shaper_creates_upright_or_inverted_live_triangles_with_one_undo_step() {
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    for direction in [0.0_f64, 180.0] {
        let mut s = session();
        s.select_tool("shaper", v).unwrap();
        let undo = s.doc().unwrap().history.undo.len();
        let corners: Vec<Point> = (0..3)
            .map(|i| {
                let angle = (direction + 7.0 - 90.0).to_radians() + std::f64::consts::TAU * i as f64 / 3.0;
                Point::new(200.0 + 70.0 * angle.cos(), 200.0 + 70.0 * angle.sin())
            })
            .collect();
        let mut pts = vec![];
        for i in 0..3 {
            for j in 0..20 {
                pts.push(corners[i] + (corners[(i + 1) % 3] - corners[i]) * (j as f64 / 20.0));
            }
        }
        // Begin midway along an edge; the release closes the final segment.
        pts.rotate_left(7);
        s.pointer(&PointerEvent::new(PointerKind::Down, pts[0].x, pts[0].y), v).unwrap();
        for p in &pts[1..] {
            s.pointer(&PointerEvent::new(PointerKind::Drag, p.x, p.y), v).unwrap();
        }
        s.pointer(&PointerEvent::new(PointerKind::Up, pts[0].x, pts[0].y), v).unwrap();
        let id = *s.doc().unwrap().selection.objects.first().expect("recognized triangle");
        let node = s.doc().unwrap().doc.node(id).unwrap().clone();
        let vectorcraft_doc::NodeKind::Path { live: Some(vectorcraft_doc::LiveShape::Polygon { sides: 3, xf, .. }), .. } = &node.kind else {
            panic!("expected live triangle, got {:?}", node.kind_label());
        };
        let [a, b, c, d, _, _] = xf.as_coeffs();
        let (sin, cos) = direction.to_radians().sin_cos();
        for (actual, expected) in [(a, cos), (b, sin), (c, -sin), (d, cos)] {
            assert!((actual - expected).abs() < 1e-12, "direction {direction}, transform {xf:?}");
        }
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(s.doc().unwrap().doc.node(id).is_none());
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc.node(id).unwrap(), &node);
    }
}

#[test]
fn a_panicking_command_rolls_back_instead_of_crashing() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 100.0, 50.0);
    let before = s.doc().unwrap().doc.clone();
    let undo_steps = s.doc().unwrap().history.undo.len();
    // A bug halfway through a command: the document was already changed in place, inside a
    // nested command.
    let r = s.run_guarded("test.panic", |s| {
        let st = s.doc_mut()?;
        Arc::make_mut(&mut st.doc).remove(a)?;
        st.selection = Selection::default();
        s.run_nested(|_| panic!("boom"))
    });
    match r {
        Err(EngineError::Internal { cmd, msg }) => assert_eq!((cmd.as_str(), msg.as_str()), ("test.panic", "boom")),
        other => panic!("expected an internal error, got {other:?}"),
    }
    let st = s.doc().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &before), "document rolled back");
    assert_eq!(st.selection.objects, vec![a]);
    assert_eq!(st.history.undo.len(), undo_steps);
    assert_eq!(s.depth, 0);
    // The session keeps working (and journaling top-level commands).
    let b = rect(&mut s, 200.0, 10.0, 50.0, 50.0);
    assert!(s.doc().unwrap().doc.node(b).is_some());
    assert_eq!(s.journal.last().map(|j| j.0.as_str()), Some("shape.rectangle"));
}

/// New layers and sublayers share one "Layer N" numbering, whatever art the document holds, and
/// never repeat a name.
#[test]
fn new_layers_and_sublayers_are_numbered_together() {
    let name = |s: &Session, id: &Value| s.doc().unwrap().doc.node(NodeId(id["id"].as_u64().unwrap())).unwrap().display_name().to_string();
    let mut s = session();
    let sub = s.execute("layer.newSublayer", &json!({})).unwrap();
    assert_eq!(name(&s, &sub), "Layer 2", "not its parent's name");
    for x in [0.0, 60.0, 120.0, 180.0, 240.0] {
        rect(&mut s, x, 0.0, 50.0, 50.0);
    }
    let layer = s.execute("layer.new", &json!({})).unwrap();
    assert_eq!(name(&s, &layer), "Layer 3", "after the sublayer, not a repeat of it");
    let sub = s.execute("layer.newSublayer", &json!({})).unwrap();
    assert_eq!(name(&s, &sub), "Layer 4", "the art doesn't count");
}

/// #785: objects given are used as given (a value that isn't an object id fails, never the
/// selection instead), and `text.setRangeStyle` works on the selected type object without `id`.
#[test]
fn given_targets_are_used_as_given_and_range_style_takes_the_selected_type() {
    let mut s = session();
    let first = s.execute("text.create", &json!({"x": 10, "y": 60, "text": "first", "size": 30})).unwrap()["id"].clone();
    s.execute("text.create", &json!({"x": 10, "y": 140, "text": "second", "size": 30})).unwrap();
    for bad in [
        json!({"id": "$1.id", "size": 60}),
        json!({"id": 2.5, "size": 60}),
        json!({"ids": [first.clone(), "$1.id"], "size": 60}),
        json!({"ids": 3, "size": 60}),
    ] {
        assert!(s.execute("text.setStyle", &bad).is_err(), "{bad}");
    }
    // Another kind of id (an effect's) is still not an object id.
    let sel = s.doc().unwrap().selection.objects.clone();
    assert!(s.execute("effect.apply", &json!({"id": "stylize.dropShadow"})).is_ok());
    assert_eq!(s.doc().unwrap().selection.objects, sel);
    // The selected type object (the second) is styled when no id is given.
    let r = s.execute("text.setRangeStyle", &json!({"start": 0, "end": 3, "style": "Bold"})).unwrap();
    assert_ne!(r["id"], first);
    s.execute("select.all", &json!({})).unwrap();
    assert!(s.execute("text.setRangeStyle", &json!({"start": 0, "end": 3, "size": 40})).is_err(), "two type objects: which one?");
    assert!(s.execute("text.setRangeStyle", &json!({"id": "two", "size": 40})).is_err());
}

#[test]
fn malformed_anchor_selection_never_replaces_or_partially_changes_selection() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 30.0, 20.0);
    let b = rect(&mut s, 50.0, 10.0, 30.0, 20.0);
    let expected = s.doc().unwrap().selection.objects.clone();
    let expected_anchors = s.doc().unwrap().selection.anchors.clone();
    for p in [
        json!({"id": a.0, "anchors": [[0, 0], ["bad", 2]]}),
        json!({"id": a.0, "anchors": [[0, 0, 1]]}),
        json!({"id": a.0, "anchors": "not an array"}),
        json!({"id": u64::MAX, "anchors": []}),
        json!({"id": a.0, "anchors": [], "mode": "missing"}),
    ] {
        assert!(s.execute("select.anchors", &p).is_err(), "{p}");
        assert_eq!(s.doc().unwrap().selection.objects, expected);
        assert_eq!(s.doc().unwrap().selection.anchors, expected_anchors);
    }
    for p in [
        json!({"items": [{"id": a.0, "anchors": [[0, 0]]}, {"id": b.0, "anchors": [[0, "bad"]]}]}),
        json!({"items": [{"id": a.0, "anchors": [[0, 0]]}, {"id": "bad", "anchors": [[0, 0]]}]}),
        json!({"items": [{"id": u64::MAX, "anchors": []}]}),
        json!({"items": [{"id": a.0}]}),
    ] {
        assert!(s.execute("select.anchorsMany", &p).is_err(), "{p}");
        assert_eq!(s.doc().unwrap().selection.objects, expected);
        assert_eq!(s.doc().unwrap().selection.anchors, expected_anchors);
    }
    s.execute("select.anchorsMany", &json!({"items": [{"id": a.0, "anchors": [[0, 0]]}]})).unwrap();
    assert!(s.doc().unwrap().selection.contains(a));
    assert!(!s.doc().unwrap().selection.contains(b));
}

#[test]
fn path_commands_reject_bad_anchors_before_modifying_geometry() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 20.0, 50.0, 30.0);
    let original_points: Vec<_> = s.doc().unwrap().doc.node(a).unwrap().path_data().unwrap().anchors().map(|(_, _, an)| an.p).collect();
    let history = s.doc().unwrap().history.undo.len();
    let valid = json!({"x": 10, "y": 20});
    for invalid in [
        json!({"x": "bad", "y": 40}),
        json!({"x": 50}),
        json!({"x": 50, "y": 40, "in": [0]}),
        json!({"x": 50, "y": 40, "out": [1, "bad"]}),
        json!({"x": 50, "y": 40, "smooth": "yes"}),
    ] {
        let anchors = json!([valid.clone(), invalid]);
        assert!(s.execute("path.create", &json!({"anchors": anchors})).is_err(), "{anchors}");
        assert!(s.execute("path.setAnchors", &json!({"id": a.0, "subpaths": [{"anchors": anchors}]})).is_err(), "{anchors}");
        let points: Vec<_> = s.doc().unwrap().doc.node(a).unwrap().path_data().unwrap().anchors().map(|(_, _, an)| an.p).collect();
        assert_eq!(points, original_points);
        assert_eq!(s.doc().unwrap().history.undo.len(), history);
    }
    assert!(s.execute("path.setAnchors", &json!({"id": a.0, "subpaths": [{"anchors": []}, {"closed": true}]})).is_err());
    assert!(s.execute("path.setAnchors", &json!({"id": a.0, "subpaths": [{"anchors": [valid.clone()], "closed": "yes"}]})).is_err());
    // The Pen's append reads its anchor the same way.
    assert!(s.execute("path.appendAnchor", &json!({"id": a.0, "x": 5, "y": 5, "out": [1]})).is_err());
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    s.execute("path.setAnchors", &json!({"id": a.0, "subpaths": [{"anchors": [{"x": 0, "y": 0}, {"x": 30, "y": 40}], "closed": false}]})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node(a).unwrap().path_data().unwrap().anchor_count(), 2);
}
