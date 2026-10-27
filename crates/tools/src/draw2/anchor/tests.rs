use super::*;
use crate::testutil::*;
use vectorcraft_doc::Selection;
use vectorcraft_geom::{Anchor, ParamCurve};

/// The Anchor Point tool has its own caret cursor, not the Pen's start-a-path one, and over a
/// segment the Reshape Segment cursor, kept while the segment is dragged.
#[test]
fn anchor_point_tool_shows_its_caret_and_reshape_over_segments() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = AnchorTool::new("anchorPoint");
    let none = Mods::default();
    for at in [Point::new(20.0, 20.0), Point::new(200.0, 100.0)] {
        assert_eq!(t.cursor(&cx, at, none), Cursor::AnchorPoint, "{at:?}");
    }
    assert_eq!(t.cursor(&cx, Point::new(150.0, 101.0), none), Cursor::ReshapeSegment);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 101.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 140.0));
    assert_eq!(t.cursor(&cx, Point::new(150.0, 140.0), none), Cursor::ReshapeSegment);
}

/// Selected three-anchor blend spine; the end anchors stay pinned to the
/// centres of the two key objects.
fn blend_spine_doc(smooth: bool) -> (vectorcraft_doc::Document, NodeId, Selection) {
    let (mut d, _) = doc_with_rect();
    let layer = d.layers[0].id;
    let (id, first, last) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    let key = |id, x| {
        let path = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(x, 300.0, x + 20.0, 320.0));
        std::sync::Arc::new(vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art()))
    };
    let mut path = vectorcraft_geom::SubPath::polyline(&[Point::new(110.0, 310.0), Point::new(160.0, 310.0), Point::new(210.0, 310.0)], false);
    if smooth {
        path.anchors[1] = Anchor::smooth(Point::new(160.0, 310.0), Point::new(190.0, 330.0));
    }
    let spec =
        vectorcraft_doc::live::BlendSpec { spine: Some(vectorcraft_geom::PathData::single(path)), key_anchors: vec![0, 2], ..Default::default() };
    let blend = vectorcraft_doc::Node::new(
        id,
        vectorcraft_doc::NodeKind::Blend { children: vec![key(first, 100.0), key(last, 200.0)], spec: Box::new(spec) },
    );
    d.insert(Some(layer), 1, blend).unwrap();
    let mut selected = Selection::default();
    selected.set([id]);
    (d, id, selected)
}

/// Owner priority applies to anchors, visible handles and segments alike, regardless
/// of selection insertion order; cursor and emitted command must agree.
#[test]
fn overlapping_path_and_blend_follow_selection_then_stacking() {
    use vectorcraft_doc::{Appearance, Node};
    use vectorcraft_geom::{PathData, SubPath};
    for path_on_top in [false, true] {
        for path_selected in [false, true] {
            for blend_first in [false, true] {
                let (d, blend, sel) = blend_spine_doc(true);
                let paint = paint();
                let spine = crate::direct::spines(&cx(&d, &sel, &paint)).pop().unwrap().1;
                let curve = spine.subpaths[0].segment(1).eval(0.5);
                // Point, handle and segment overlaps, including different cursor kinds.
                for (at, path_is_segment) in [
                    (Point::new(160.0, 310.0), false),
                    (Point::new(190.0, 330.0), false),
                    (Point::new(160.0, 310.0), true),
                    (Point::new(190.0, 330.0), true),
                    (curve, false),
                    (curve, true),
                ] {
                    let (mut d, mut sel) = (d.clone(), sel.clone());
                    let path = d.alloc_id();
                    let points = if path_is_segment {
                        vec![at - vectorcraft_geom::Vec2::new(0.0, 30.0), at + vectorcraft_geom::Vec2::new(0.0, 30.0)]
                    } else {
                        vec![at, at + vectorcraft_geom::Vec2::new(0.0, 60.0)]
                    };
                    let node = Node::path(path, PathData::single(SubPath::polyline(&points, false)), Appearance::default_art());
                    d.insert(Some(d.layers[0].id), if path_on_top { 2 } else { 1 }, node).unwrap();
                    if path_selected {
                        sel.set(if blend_first { vec![blend, path] } else { vec![path, blend] });
                    }
                    let c = cx(&d, &sel, &paint);
                    let path_wins = path_selected && path_on_top;
                    let expected_id = if path_wins { path } else { blend };
                    let spine = crate::direct::spines(&c).into_iter().find(|(id, _)| *id == blend).unwrap().1;
                    let spine_segment = AnchorTool::spine_point(&spine, at, c.point_tol()).is_none()
                        && AnchorTool::spine_handle(&c, &spine, at, c.point_tol()).is_none();
                    let reshape = if path_wins { path_is_segment } else { spine_segment };
                    let mut tool = AnchorTool::new("anchorPoint");
                    assert_eq!(tool.cursor(&c, at, Mods::default()), if reshape { Cursor::ReshapeSegment } else { Cursor::AnchorPoint });
                    tool.pointer(&c, &PointerEvent::new(PointerKind::Down, at.x, at.y));
                    let actions = tool.pointer(&c, &PointerEvent::new(PointerKind::Drag, at.x + 10.0, at.y + 20.0));
                    let Some(Action::Preview(cmd, args)) = actions.get(1) else { panic!("missing preview: {actions:?}") };
                    assert_eq!(
                        args["id"], expected_id.0,
                        "top={path_on_top}, selected={path_selected}, blend_first={blend_first}, at={at:?}, segment={path_is_segment}: {cmd}"
                    );
                    assert_eq!(cmd.starts_with("path."), path_wins);
                    assert_eq!(cmd.ends_with("reshapeSegment"), reshape);
                }
            }
        }
    }
}

#[test]
fn anchor_point_converts_a_blend_spine_point_before_reshaping_its_segment() {
    let (d, id, sel) = blend_spine_doc(false);
    let p = paint();
    let c = cx(&d, &sel, &p);
    let mut tool = AnchorTool::new("anchorPoint");
    assert!(tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 160.0, 310.0)).is_empty());
    let actions = tool.pointer(&c, &PointerEvent::new(PointerKind::Drag, 160.0, 340.0));
    assert_eq!(actions.first(), Some(&Action::Begin("Convert Anchor Point".into())));
    assert!(matches!(actions.get(1), Some(Action::Preview(cmd, args))
        if cmd == "object.blend.spine.convertAnchor"
        && args["id"] == id.0 && args["anchor"] == 1
        && args["to"] == "smooth" && args["y"] == 340.0));
    assert_eq!(tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 160.0, 340.0)), vec![Action::Commit]);
}

#[test]
fn shift_reshapes_a_blend_spine_segment_perpendicular() {
    let (d, id, sel) = blend_spine_doc(true);
    let p = paint();
    let c = cx(&d, &sel, &p);
    let spine = crate::direct::spines(&c).pop().unwrap().1;
    let at = spine.subpaths[0].segment(1).eval(0.5);
    for shift in [false, true] {
        let mut tool = AnchorTool::new("anchorPoint");
        tool.pointer(&c, &PointerEvent::new(PointerKind::Down, at.x, at.y));
        let actions = tool.pointer(&c, &PointerEvent::new(PointerKind::Drag, at.x, at.y + 20.0).with_mods(Mods { shift, ..Mods::default() }));
        assert!(
            matches!(actions.get(1), Some(Action::Preview(cmd, args))
            if cmd == "object.blend.spine.reshapeSegment" && args["id"] == id.0
            && args["perpendicular"] == shift),
            "{actions:?}"
        );
    }
}

#[test]
fn anchor_point_keeps_blend_spine_handles_visible_and_editable_after_switch() {
    let (d, id, sel) = blend_spine_doc(true);
    let p = paint();
    let c = cx(&d, &sel, &p);
    // A new tool instance models switching from Direct Selection.
    let mut tool = AnchorTool::new("anchorPoint");
    let overlays = tool.overlays(&c);
    assert!(overlays.iter().any(|o| matches!(o, Overlay::Handle { p, .. } if *p == Point::new(190.0, 330.0))));
    assert!(overlays.iter().any(|o| matches!(o, Overlay::Handle { p, .. } if *p == Point::new(130.0, 290.0))));
    tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 190.0, 330.0));
    let actions = tool.pointer(&c, &PointerEvent::new(PointerKind::Drag, 190.0, 350.0));
    assert!(matches!(actions.get(1), Some(Action::Preview(cmd, args))
        if cmd == "object.blend.spine.moveAnchor"
        && args["id"] == id.0 && args["anchor"] == 1 && args["handle"] == "out"
        && args["independent"] == true));
    assert_eq!(tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 190.0, 350.0)), vec![Action::Commit]);

    tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 160.0, 310.0));
    assert_eq!(
        tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 160.0, 310.0)),
        vec![Action::Exec("object.blend.spine.convertAnchor".into(), json!({"id": id.0, "anchor": 1, "to": "corner"}),)],
    );
}

/// P1-1: a handle 2pt from its anchor must not intercept a click at the anchor centre;
/// dragging the anchor converts it, while a click on an eligible handle still retracts it.
#[test]
fn anchor_point_prefers_a_blend_spine_anchor_over_its_short_handle() {
    let (mut d, _) = doc_with_rect();
    let layer = d.layers[0].id;
    let (id, first, last) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    let key = |id, x| {
        let path = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(x, 300.0, x + 20.0, 320.0));
        std::sync::Arc::new(vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art()))
    };
    let mut path = vectorcraft_geom::SubPath::polyline(&[Point::new(110.0, 310.0), Point::new(160.0, 310.0), Point::new(210.0, 310.0)], false);
    path.anchors[1] = Anchor::smooth(Point::new(160.0, 310.0), Point::new(162.0, 310.0));
    let spec =
        vectorcraft_doc::live::BlendSpec { spine: Some(vectorcraft_geom::PathData::single(path)), key_anchors: vec![0, 2], ..Default::default() };
    let blend = vectorcraft_doc::Node::new(
        id,
        vectorcraft_doc::NodeKind::Blend { children: vec![key(first, 100.0), key(last, 200.0)], spec: Box::new(spec) },
    );
    d.insert(Some(layer), 1, blend).unwrap();
    let mut selected = Selection::default();
    selected.set([id]);
    let p = paint();
    let c = cx(&d, &selected, &p);
    // Clicking the anchor centre converts the smooth point to a corner.
    let mut tool = AnchorTool::new("anchorPoint");
    tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 160.0, 310.0));
    assert_eq!(
        tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 160.0, 310.0)),
        vec![Action::Exec("object.blend.spine.convertAnchor".into(), json!({"id": id.0, "anchor": 1, "to": "corner"}),)],
    );
    // Clicking the eligible handle end retracts only that handle.
    let mut tool = AnchorTool::new("anchorPoint");
    tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 162.0, 310.0));
    assert_eq!(
        tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 162.0, 310.0)),
        vec![Action::Exec(
            "object.blend.spine.moveAnchor".into(),
            json!({"id": id.0, "anchor": 1, "handle": "out", "x": 160.0, "y": 310.0, "independent": true}),
        )],
    );
}

/// P2-2: with Show handles when multiple anchors are selected off, spine handles hide
/// and cannot intercept pointer input, as for ordinary paths.
#[test]
fn anchor_point_hides_blend_spine_handles_when_multiple_is_off() {
    let (d, _, sel) = blend_spine_doc(true);
    let p = paint();
    let single = ToolContext { handles_multiple: false, ..cx(&d, &sel, &p) };
    let tool = AnchorTool::new("anchorPoint");
    assert!(tool.overlays(&single).iter().all(|o| !matches!(o, Overlay::Handle { .. })));
    // The hidden handle endpoint is neither an editable handle nor on the curve.
    let mut tool = AnchorTool::new("anchorPoint");
    tool.pointer(&single, &PointerEvent::new(PointerKind::Down, 190.0, 330.0));
    assert!(matches!(tool.state, State::Idle));
    assert!(tool.pointer(&single, &PointerEvent::new(PointerKind::Up, 190.0, 330.0)).is_empty());
    // A drag on the curve away from anchors and handles reshapes the spine segment.
    let (_, spine) = crate::direct::spines(&single).pop().unwrap();
    let at = spine.subpaths[0].segment(1).eval(0.5);
    assert!(AnchorTool::spine_handle(&single, &spine, at, single.point_tol()).is_none());
    assert!(AnchorTool::spine_point(&spine, at, single.point_tol()).is_none());
    let mut tool = AnchorTool::new("anchorPoint");
    assert_eq!(tool.cursor(&single, at, Mods::default()), Cursor::ReshapeSegment);
    tool.pointer(&single, &PointerEvent::new(PointerKind::Down, at.x, at.y));
    let actions = tool.pointer(&single, &PointerEvent::new(PointerKind::Drag, at.x, at.y + 20.0));
    assert!(matches!(actions.get(1), Some(Action::Preview(cmd, _)) if cmd == "object.blend.spine.reshapeSegment"));
}

/// Hover and click agree: the caret shows where a press converts a spine point or
/// edits a spine handle, and ReshapeSegment only where a press reshapes a segment.
#[test]
fn anchor_point_cursor_matches_spine_press_precedence() {
    let (d, _, sel) = blend_spine_doc(true);
    let p = paint();
    let c = cx(&d, &sel, &p);
    let tool = AnchorTool::new("anchorPoint");
    let none = Mods::default();
    assert_eq!(tool.cursor(&c, Point::new(160.0, 310.0), none), Cursor::AnchorPoint);
    assert_eq!(tool.cursor(&c, Point::new(190.0, 330.0), none), Cursor::AnchorPoint);
    let spine = crate::direct::spines(&c).pop().unwrap().1;
    assert_eq!(tool.cursor(&c, spine.subpaths[0].segment(1).eval(0.5), none), Cursor::ReshapeSegment);
}

/// P1-2: Shift-dragging a blend spine handle constrains it to 45-degree steps round
/// its anchor, as for ordinary Anchor Point handles.
#[test]
fn anchor_point_shift_constrains_a_blend_spine_handle() {
    let (d, id, sel) = blend_spine_doc(true);
    let p = paint();
    let c = cx(&d, &sel, &p);
    let shift = Mods { shift: true, ..Mods::default() };
    let mut tool = AnchorTool::new("anchorPoint");
    tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 190.0, 330.0));
    let drag = PointerEvent { kind: PointerKind::Drag, pos: Point::new(200.0, 340.0), mods: shift, pressure: 1.0 };
    let actions = tool.pointer(&c, &drag);
    // The pointer (40, 30) keeps its length of 50; Shift snaps it to 45 degrees.
    assert!(matches!(actions.get(1), Some(Action::Preview(cmd, args))
        if cmd == "object.blend.spine.moveAnchor"
        && args["id"] == id.0 && args["anchor"] == 1 && args["handle"] == "out"
        && args["independent"] == true
        && (args["x"].as_f64().unwrap() - 160.0 - 50.0 * std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-6
        && (args["y"].as_f64().unwrap() - 310.0 - 50.0 * std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-6));
    assert_eq!(tool.pointer(&c, &PointerEvent::new(PointerKind::Up, 200.0, 340.0)), vec![Action::Commit]);
}

#[test]
fn add_and_delete_anchor_clicks() {
    let (d, id) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = AnchorTool::new("addAnchor");
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 101.0));
    assert!(matches!(&a[0], Action::Exec(c, v) if c == "path.insertAnchor" && v["id"] == id.0 && v["segment"] == 0));
    assert!((t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 20.0, 20.0))).is_empty());
    let mut t = AnchorTool::new("deleteAnchor");
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.0, 101.0));
    assert_eq!(a, vec![Action::Exec("path.removeAnchor".into(), json!({"id": id.0, "subpath": 0, "anchor": 1}))]);
}

#[test]
fn scissors_splits_at_segment() {
    let (d, id) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = AnchorTool::new("scissors");
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.0, 150.0));
    assert!(
        matches!(&a[0], Action::Exec(c, v) if c == "path.split" && v["id"] == id.0 && v["segment"] == 1 && (v["t"].as_f64().unwrap() - 0.5).abs() < 1e-6)
    );
}

/// The Add Anchor Point, Delete Anchor Point, Anchor Point and Scissors tools pick anchors and
/// segments only on the art a click can pick: not on a Shaper Group's original shapes unless
/// they are isolated (Construction mode), and in isolation mode only on the isolated object.
#[test]
fn anchor_tools_leave_out_shaper_sources_and_art_outside_isolation() {
    let (d, _, sources, face) = doc_with_shaper_group();
    let rect = d.layers[0].children().unwrap()[0].id;
    let b = d.node(sources).and_then(|n| n.children()).unwrap()[1].id;
    let (s, p) = (Selection::default(), paint());
    // The path the first command of a press with `tool` at (x, y) acts on.
    let picked = |isolation: Option<NodeId>, tool: &str, x: f64, y: f64| {
        let cx = ToolContext { isolation, ..cx(&d, &s, &p) };
        let mut t = AnchorTool::new(tool);
        let mut a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, x, y));
        a.extend(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x + 20.0, y - 10.0)));
        a.iter().find_map(|a| match a {
            Action::Exec(_, v) | Action::Preview(_, v) => v["id"].as_u64().map(NodeId),
            _ => None,
        })
    };
    // The second square's top right corner and its right side, which only its original shape
    // has.
    for (tool, x, y) in [("deleteAnchor", 150.0, 300.0), ("anchorPoint", 150.0, 300.0), ("addAnchor", 150.0, 350.0), ("scissors", 150.0, 350.0)] {
        assert_eq!(picked(None, tool, x, y), None, "{tool}");
        assert_eq!(picked(Some(sources), tool, x, y), Some(b), "{tool}");
    }
    // The face's top left corner, outside the isolated rectangle.
    assert_eq!(picked(None, "deleteAnchor", 0.0, 300.0), Some(face));
    assert_eq!(picked(Some(rect), "deleteAnchor", 0.0, 300.0), None);
    assert_eq!(picked(Some(rect), "deleteAnchor", 100.0, 100.0), Some(rect));
}

#[test]
fn anchor_point_drag_pulls_handles_click_converts() {
    let (d, id) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = AnchorTool::new("anchorPoint");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 120.0, 90.0));
    assert_eq!(a[0], Action::Begin("Convert Anchor Point".into()));
    assert!(matches!(&a[1], Action::Preview(c, v) if c == "path.convertAnchor" && v["to"] == "smooth" && v["id"] == id.0));
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 120.0, 90.0)), vec![Action::Commit]);
    // Clicking a corner without handles does nothing.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
    assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 100.0, 100.0)).is_empty());
    // Dragging a segment reshapes it.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 200.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 230.0));
    assert!(matches!(&a[1], Action::Preview(c, v) if c == "path.reshapeSegment" && v["dy"] == 30.0));
}

/// Shift held while a segment is dragged keeps its handles at right angles to the line between
/// its anchors (`perpendicular`).
#[test]
fn shift_reshapes_a_segment_perpendicular() {
    let (d, id) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let shift = Mods { shift: true, ..Mods::default() };
    for (mods, perpendicular) in [(Mods::default(), false), (shift, true)] {
        let mut t = AnchorTool::new("anchorPoint");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 200.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 160.0, 230.0).with_mods(mods));
        assert!(
            matches!(&a[1], Action::Preview(c, v) if c == "path.reshapeSegment" && v["id"] == id.0 && v["perpendicular"] == perpendicular),
            "{a:?}"
        );
    }
}

/// #494: the Anchor Point tool picks within Selection & Anchor Display → Tolerance, and drags
/// only the handles shown.
#[test]
fn anchor_point_picks_within_the_tolerance_and_drags_shown_handles() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0), Point::new(300.0, 300.0)], false);
    sp.anchors[1] = vectorcraft_geom::Anchor::smooth(Point::new(200.0, 300.0), Point::new(240.0, 300.0));
    let path = vectorcraft_geom::PathData::single(sp);
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let press = |c: &ToolContext, x: f64, y: f64| {
        let mut t = AnchorTool::new("anchorPoint");
        t.pointer(c, &PointerEvent::new(PointerKind::Down, x, y));
        t.pointer(c, &PointerEvent::new(PointerKind::Drag, x + 10.0, y + 30.0))
    };
    let dragged = |a: Vec<Action>| match a.as_slice() {
        [Action::Begin(_), Action::Preview(c, _)] => c.clone(),
        _ => String::new(),
    };
    // The segment 6 px off: picked with an 8 px tolerance, not with the default 3 px.
    assert_eq!(dragged(press(&cx(&d, &s, &p), 150.0, 306.0)), "");
    let wide = ToolContext { selection_tolerance: 8.0, ..cx(&d, &s, &p) };
    assert_eq!(dragged(press(&wide, 150.0, 306.0)), "path.reshapeSegment");
    // The middle anchor's handle drags; with Show handles when multiple anchors are selected
    // off it's hidden (three anchors selected) and the press finds the segment under it.
    assert_eq!(dragged(press(&cx(&d, &s, &p), 240.0, 301.0)), "path.setHandle");
    let single = ToolContext { handles_multiple: false, ..cx(&d, &s, &p) };
    assert_eq!(dragged(press(&single, 240.0, 301.0)), "path.reshapeSegment");
}

/// #2267: clicking a handle end retracts that handle only; the anchor becomes a corner and
/// keeps the other handle, as Illustrator's Anchor Point tool does.
#[test]
fn anchor_point_click_on_a_handle_retracts_only_that_handle() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0), Point::new(300.0, 300.0)], false);
    sp.anchors[1] = vectorcraft_geom::Anchor::smooth(Point::new(200.0, 300.0), Point::new(240.0, 320.0));
    let path = vectorcraft_geom::PathData::single(sp);
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let retract = |which: &str| {
        vec![Action::Exec(
            "path.setHandle".into(),
            json!({"id": id.0, "subpath": 0, "anchor": 1, "which": which, "x": 200.0, "y": 300.0, "independent": true}),
        )]
    };
    let mut t = AnchorTool::new("anchorPoint");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 240.0, 320.0));
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 240.0, 320.0)), retract("out"));
    // A hand that shakes by a pixel still clicks; the incoming handle sits opposite, at (160, 280).
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 160.0, 280.0));
    assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 160.5, 280.5)).is_empty());
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 160.5, 280.5)), retract("in"));
    // The command leaves the other handle where it was.
    let mut a = vectorcraft_geom::Anchor::smooth(Point::new(200.0, 300.0), Point::new(240.0, 320.0));
    a.set_handle(true, a.p, true);
    assert!(!a.has_out());
    assert_eq!(a.h_in, Point::new(160.0, 280.0));
}

/// Shift held at the press on an anchor moves it, handles and all, instead of pulling out
/// handles; Shift still held keeps the move at 45° steps, released frees it. A Shift-click
/// that doesn't move leaves the anchor as it was.
#[test]
fn anchor_point_shift_drag_moves_the_anchor() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0), Point::new(300.0, 300.0)], false);
    sp.anchors[1] = vectorcraft_geom::Anchor::smooth(Point::new(200.0, 300.0), Point::new(240.0, 320.0));
    let path = vectorcraft_geom::PathData::single(sp);
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, path, vectorcraft_doc::Appearance::default_art())).unwrap();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let shift = Mods { shift: true, ..Mods::default() };
    let moved = |x: f64, y: f64| Action::Preview("path.moveAnchor".into(), json!({"id": id.0, "subpath": 0, "anchor": 1, "x": x, "y": y}));
    let mut t = AnchorTool::new("anchorPoint");
    // Pressed a pixel off the anchor: it keeps that offset to the pointer.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 201.0, 301.0).with_mods(shift));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 231.0, 341.0));
    assert_eq!(a, vec![Action::Begin("Move".into()), moved(230.0, 340.0)]);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 241.0, 306.0).with_mods(shift));
    assert!(
        matches!(&a[..], [Action::Preview(c, v)] if c == "path.moveAnchor" && v["y"].as_f64().is_some_and(|y| (y - 300.0).abs() < 1e-9)),
        "{a:?}"
    );
    assert_eq!(t.cursor(&cx, Point::new(241.0, 306.0), shift), Cursor::Arrow);
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 241.0, 306.0)), vec![Action::Commit]);
    // Without Shift the same press still pulls out handles.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.0, 300.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 230.0, 340.0));
    assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, _)] if c == "path.convertAnchor"), "{a:?}");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 230.0, 340.0));
    // A Shift-click that doesn't move does nothing (no conversion to a corner).
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.0, 300.0).with_mods(shift));
    assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 200.0, 300.0).with_mods(shift)).is_empty());
}

/// #3694: Space held mid-drag on a convert moves the anchor, handles and all.
#[test]
fn anchor_point_space_mid_drag_moves_the_anchor() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let sp = vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0), Point::new(300.0, 300.0)], false);
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, vectorcraft_geom::PathData::single(sp), vectorcraft_doc::Appearance::default_art()))
        .unwrap();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let space = Mods { space: true, ..Mods::default() };
    let moved = |x: f64, y: f64| Action::Preview("path.moveAnchor".into(), json!({"id": id.0, "subpath": 0, "anchor": 1, "x": x, "y": y}));
    let mut t = AnchorTool::new("anchorPoint");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.0, 300.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 230.0, 340.0));
    assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, _)] if c == "path.convertAnchor"), "{a:?}");
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 240.0, 350.0).with_mods(space));
    assert_eq!(a, vec![moved(200.0, 300.0)]);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 250.0, 360.0).with_mods(space));
    assert_eq!(a, vec![moved(210.0, 310.0)]);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 260.0, 370.0));
    assert!(matches!(&a[..], [Action::Preview(c, _)] if c == "path.convertAnchor"), "{a:?}");
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 260.0, 370.0)), vec![Action::Commit]);
}

#[test]
fn anchor_point_shift_drag_honors_measurement_labels() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let sp = vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0)], false);
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, vectorcraft_geom::PathData::single(sp), vectorcraft_doc::Appearance::default_art()))
        .unwrap();
    let s = Selection::default();
    let p = paint();
    let base = cx(&d, &s, &p);
    let on = ToolContext { measurement_labels: true, ..base };
    let off = ToolContext { measurement_labels: false, ..base };
    let shift = Mods { shift: true, ..Mods::default() };
    let mut t = AnchorTool::new("anchorPoint");
    t.pointer(&on, &PointerEvent::new(PointerKind::Down, 200.0, 300.0).with_mods(shift));
    t.pointer(&on, &PointerEvent::new(PointerKind::Drag, 230.0, 340.0));
    let want = on.offset_label(30.0, 40.0);
    assert!(t.overlays(&on).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &want)));
    assert!(!t.overlays(&off).iter().any(|o| matches!(o, Overlay::Measure { .. })));
}
