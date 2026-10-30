use super::*;
use crate::testutil::*;
use vectorcraft_doc::Selection;

/// Direct Selection and Group Selection grab a selected clipped object (picked in the Layers
/// panel) where its mask hides it, as on its visible part: a drag moves it. With nothing of it
/// selected there, a press starts a marquee.
#[test]
fn a_press_where_a_mask_hides_a_selected_object_drags_it() {
    let (d, _, _, art) = doc_with_masked_square();
    let p = paint();
    let mut picked = Selection::default();
    picked.add(art);
    let none = Selection::default();
    // (370, 450): on the art, outside the mask (400..500).
    for group in [false, true] {
        let c = cx(&d, &picked, &p);
        let mut t = DirectSelectionTool::new(group);
        let down = t.pointer(&c, &PointerEvent::new(PointerKind::Down, 370.0, 450.0));
        assert!(!down.iter().any(|a| matches!(a, Action::Exec(c, _) if c.starts_with("select."))), "{down:?} (group: {group})");
        let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 390.0, 470.0));
        assert!(a.iter().any(|a| matches!(a, Action::Begin(n) if n == "Move")), "{a:?} (group: {group})");
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 390.0, 470.0));
        // With Shift, a marquee.
        let shift = Mods { shift: true, ..Mods::default() };
        let mut t = DirectSelectionTool::new(group);
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 370.0, 450.0).with_mods(shift));
        let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 390.0, 470.0).with_mods(shift));
        assert!(!a.iter().any(|a| matches!(a, Action::Begin(_))), "{a:?} (group: {group})");
        assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Marquee(_))), "a marquee (group: {group})");
        let c = cx(&d, &none, &p);
        let mut t = DirectSelectionTool::new(group);
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 370.0, 450.0));
        let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 390.0, 470.0));
        assert!(!a.iter().any(|a| matches!(a, Action::Begin(_))), "{a:?} (group: {group})");
    }
}

/// #414: a ruler guide is picked over the art, but an anchor on it is picked first.
#[test]
fn direct_and_group_selection_pick_ruler_guides() {
    let (mut d, id) = doc_with_rect();
    d.guides.push(vectorcraft_doc::Guide::new(true, 100.0));
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let down = |t: &mut DirectSelectionTool, y: f64| t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, y));
    let pick = vec![Action::Exec("guide.select".into(), json!({"indexes": [0]}))];
    for group in [false, true] {
        let mut t = DirectSelectionTool::new(group);
        assert_eq!(down(&mut t, 150.0), pick, "on the rect's edge (group: {group})");
        assert!(t.busy());
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 100.0, 150.0));
        assert_eq!(t.cursor(&c, Point::new(101.0, 300.0), Mods::default()), Cursor::ResizeH);
    }
    let mut t = DirectSelectionTool::new(false);
    assert_eq!(down(&mut t, 100.0), vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 0]], "mode": "set"}))]);
}

/// A Cmd/Ctrl+Shift double-click with Direct Selection or Group Selection releases the ruler
/// guide or the guide path under the pointer; a plain double-click on a ruler guide, or one while
/// guides are hidden or locked, releases nothing.
#[test]
fn a_cmd_shift_double_click_releases_the_guide_under_the_pointer() {
    let (mut d, id) = doc_with_rect();
    d.guides.push(vectorcraft_doc::Guide::new(false, 300.0));
    let Some(NodeKind::Path { guide, .. }) = d.node_mut(id).map(|n| &mut n.kind) else { panic!("the rectangle is a path") };
    *guide = true;
    let (s, p) = (Selection::default(), paint());
    let both = Mods { cmd: true, shift: true, ..Mods::default() };
    let index = vec![Action::Exec("view.guides.release".into(), json!({"index": 0}))];
    let ids = vec![Action::Exec("view.guides.release".into(), json!({"ids": [id.0]}))];
    let locked = ToolContext { guides: false, ..cx(&d, &s, &p) };
    for group in [false, true] {
        let dbl = |c: &ToolContext, x: f64, y: f64, m: Mods| {
            DirectSelectionTool::new(group).pointer(c, &PointerEvent::new(PointerKind::DoubleClick, x, y).with_mods(m))
        };
        let c = cx(&d, &s, &p);
        assert_eq!(dbl(&c, 350.0, 301.0, both), index, "on the ruler guide (group: {group})");
        assert_eq!(dbl(&c, 200.0, 150.0, both), ids, "on the guide path (group: {group})");
        assert!(dbl(&c, 350.0, 301.0, Mods::default()).is_empty(), "a plain double-click (group: {group})");
        assert!(dbl(&c, 350.0, 301.0, Mods { cmd: true, ..Mods::default() }).is_empty(), "Cmd/Ctrl alone (group: {group})");
        assert!(dbl(&locked, 350.0, 301.0, both).is_empty(), "guides locked (group: {group})");
        assert!(dbl(&locked, 200.0, 150.0, both).is_empty(), "guides locked (group: {group})");
    }
}

/// An anchor is picked from 2 px past its drawn square even where its segment is nearer, and
/// further with a larger anchor Size or Tolerance (#593); away from it the segment is.
#[test]
fn anchors_are_picked_before_the_segments_through_them() {
    let (d, id) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let corner = vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 0]], "mode": "set"}))];
    let press = |cx: &ToolContext, x: f64, y: f64| DirectSelectionTool::new(false).pointer(cx, &PointerEvent::new(PointerKind::Down, x, y));
    let c = cx(&d, &s, &p);
    // 4.3 px from the corner, 0.5 px from the top edge.
    assert_eq!(press(&c, 104.2, 100.5), corner);
    assert_ne!(press(&c, 110.0, 100.5), corner, "the segment, away from the anchor");
    assert_ne!(press(&c, 105.0, 100.5), corner, "past the default reach (4.5 px)");
    let large = ToolContext { anchor_size: 7, ..cx(&d, &s, &p) };
    assert_eq!(press(&large, 106.0, 100.5), corner, "Size 7 draws them 9 px wide");
    let loose = ToolContext { selection_tolerance: 8.0, ..cx(&d, &s, &p) };
    assert_eq!(press(&loose, 107.5, 100.5), corner);
    // On screen: at 200% the reach is half as far in the document.
    let zoomed = ToolContext { zoom: 2.0, ..cx(&d, &s, &p) };
    assert_eq!(press(&zoomed, 102.1, 100.2), corner);
    assert_ne!(press(&zoomed, 102.6, 100.2), corner);
}

/// A press on a clip group's clipping path, where the art it clips lies over its edge, takes
/// the clipping path's segment, so the mask can be reshaped on the canvas; inside the mask the
/// art is taken, as before.
#[test]
fn a_press_on_a_clipping_path_takes_its_segment_over_the_art() {
    let (d, _, clip, art) = doc_with_masked_square();
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let press = |x: f64, y: f64| DirectSelectionTool::new(false).pointer(&c, &PointerEvent::new(PointerKind::Down, x, y));
    let names = |actions: &[Action], id: NodeId| {
        actions.iter().any(|a| matches!(a, Action::Exec(_, v) if v["id"] == json!(id.0) || v["ids"] == json!([id.0])))
    };
    // The middle of the right edge, on it and just inside it.
    for x in [500.0, 498.5] {
        let a = press(x, 450.0);
        assert!(names(&a, clip), "at {x}: {a:?}");
    }
    let a = press(450.0, 450.0);
    assert!(names(&a, art) && !names(&a, clip), "{a:?}");
}

/// A blend of two 20 pt squares centred on (110, 310) and (210, 310), selected.
fn blend_doc() -> (vectorcraft_doc::Document, NodeId, NodeId) {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let key = |id: NodeId, x: f64| {
        let r = vectorcraft_geom::shapes::rectangle(Rect::new(x, 300.0, x + 20.0, 320.0));
        std::sync::Arc::new(vectorcraft_doc::Node::path(id, r, vectorcraft_doc::Appearance::default_art()))
    };
    let (g, k1, k2) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    let b = vectorcraft_doc::Node::new(g, NodeKind::Blend { children: vec![key(k1, 100.0), key(k2, 200.0)], spec: Default::default() });
    d.insert(Some(l), 1, b).unwrap();
    (d, g, k1)
}

/// Highlight anchors on mouse over (#394): the anchor under the pointer, within the selection
/// tolerance, shows enlarged; none with the preference off or away from anchors.
#[test]
fn the_anchor_under_the_pointer_is_highlighted() {
    let (d, id) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let big = |t: &DirectSelectionTool, cx: &ToolContext| {
        t.overlays(cx).into_iter().find_map(|o| match o {
            Overlay::Anchor { p, size, .. } if size > 5.0 => Some(p),
            _ => None,
        })
    };
    let mut t = DirectSelectionTool::new(false);
    let on = cx(&d, &s, &p);
    t.pointer(&on, &PointerEvent::new(PointerKind::Move, 102.0, 101.0));
    assert_eq!(big(&t, &on), Some(Point::new(100.0, 100.0)), "the corner of {id:?}");
    t.pointer(&on, &PointerEvent::new(PointerKind::Move, 150.0, 100.0));
    assert_eq!(big(&t, &on), None, "mid-segment: no anchor");
    let off = ToolContext { highlight_anchors: false, ..cx(&d, &s, &p) };
    t.pointer(&off, &PointerEvent::new(PointerKind::Move, 102.0, 101.0));
    assert_eq!(big(&t, &off), None, "off");
}

#[test]
fn dragging_a_spine_point_moves_it() {
    let (d, g, _) = blend_doc();
    let mut s = Selection::default();
    s.set([g]);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = DirectSelectionTool::new(false);
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 211.0, 310.0)), vec![]);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 221.0, 350.0));
    assert_eq!(a[0], Action::Begin("Reshape Spine".into()));
    assert_eq!(a[1], Action::Preview("object.blend.spine.moveAnchor".into(), json!({"id": g.0, "anchor": 1, "x": 220.0, "y": 350.0})));
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 221.0, 350.0)), vec![Action::Commit]);
    assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Anchor { filled: true, .. })), "the clicked point shows");
}

/// #1111: the Anchor Point tool bends a selected blend's straight spine; Direct Selection
/// reshapes a curved spine segment and leaves a straight one alone, as it does a path's.
#[test]
fn spine_segments_bend_with_the_anchor_point_tool_and_direct_selection() {
    let (mut d, g, _) = blend_doc();
    let mut s = Selection::default();
    s.set([g]);
    let p = paint();
    let drag = |t: &mut dyn crate::Tool, cx: &ToolContext| {
        t.pointer(cx, &PointerEvent::new(PointerKind::Down, 160.0, 311.0));
        let a = t.pointer(cx, &PointerEvent::new(PointerKind::Drag, 160.0, 351.0));
        t.pointer(cx, &PointerEvent::new(PointerKind::Up, 160.0, 351.0));
        a
    };
    let straight = cx(&d, &s, &p);
    let a = drag(crate::create("anchorPoint").as_mut(), &straight);
    assert!(
        matches!(a.last(), Some(Action::Preview(c, v)) if c == "object.blend.spine.reshapeSegment" && v["id"] == g.0 && v["segment"] == 0 && v["dy"] == 40.0),
        "{a:?}"
    );
    let a = drag(&mut DirectSelectionTool::new(false), &straight);
    assert!(!a.iter().any(|x| matches!(x, Action::Preview(c, _) if c == "object.blend.spine.reshapeSegment")), "straight: {a:?}");
    // The spine bent upward: Direct Selection takes it from where it passes.
    let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(110.0, 310.0), Point::new(210.0, 310.0)], false);
    sp.bend_segment(0, 0.5, vectorcraft_geom::Vec2::new(0.0, 20.0));
    let Some(NodeKind::Blend { spec, .. }) = d.node_mut(g).map(|n| &mut n.kind) else { panic!("blend") };
    (spec.spine, spec.key_anchors) = (Some(PathData::single(sp)), vec![0, 1]);
    let mut t = DirectSelectionTool::new(false);
    let curved = cx(&d, &s, &p);
    t.pointer(&curved, &PointerEvent::new(PointerKind::Down, 160.0, 330.0));
    let a = t.pointer(&curved, &PointerEvent::new(PointerKind::Drag, 160.0, 345.0));
    assert_eq!(a[0], Action::Begin("Reshape Spine".into()));
    assert!(
        matches!(&a[1], Action::Preview(c, v) if c == "object.blend.spine.reshapeSegment" && v["segment"] == 0 && v["dx"] == 0.0 && v["dy"] == 15.0),
        "{a:?}"
    );
    assert_eq!(t.pointer(&curved, &PointerEvent::new(PointerKind::Up, 160.0, 345.0)), vec![Action::Commit]);
}

/// Shift held while a curved segment is dragged sends `perpendicular`. The test drags a path's
/// segment with Direct Selection, and a blend spine's segment with Direct Selection and with the
/// Anchor Point tool.
#[test]
fn shift_reshapes_a_segment_perpendicular() {
    let shift = Mods { shift: true, ..Mods::default() };
    let p = paint();
    let drag = |t: &mut dyn crate::Tool, cx: &ToolContext, from: (f64, f64), mods: Mods| {
        t.pointer(cx, &PointerEvent::new(PointerKind::Down, from.0, from.1));
        let a = t.pointer(cx, &PointerEvent::new(PointerKind::Drag, from.0 + 10.0, from.1 - 20.0).with_mods(mods));
        t.pointer(cx, &PointerEvent::new(PointerKind::Up, from.0 + 10.0, from.1 - 20.0).with_mods(mods));
        a
    };
    let sent = |a: &[Action], cmd: &str| match a.last() {
        Some(Action::Preview(c, v)) if c == cmd => v["perpendicular"].as_bool(),
        _ => None,
    };
    let (d, _) = arch_doc();
    let s = Selection::default();
    let arch = cx(&d, &s, &p);
    let (mut b, g, _) = blend_doc();
    let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(110.0, 310.0), Point::new(210.0, 310.0)], false);
    sp.bend_segment(0, 0.5, vectorcraft_geom::Vec2::new(0.0, 20.0));
    let Some(NodeKind::Blend { spec, .. }) = b.node_mut(g).map(|n| &mut n.kind) else { panic!("blend") };
    (spec.spine, spec.key_anchors) = (Some(PathData::single(sp)), vec![0, 1]);
    let mut bs = Selection::default();
    bs.set([g]);
    let spine = cx(&b, &bs, &p);
    for (mods, perpendicular) in [(Mods::default(), false), (shift, true)] {
        let a = drag(&mut DirectSelectionTool::new(false), &arch, (150.0, 262.5), mods);
        assert_eq!(sent(&a, "path.reshapeSegment"), Some(perpendicular), "{a:?}");
        let a = drag(&mut DirectSelectionTool::new(false), &spine, (160.0, 330.0), mods);
        assert_eq!(sent(&a, "object.blend.spine.reshapeSegment"), Some(perpendicular), "{a:?}");
        let a = drag(crate::create("anchorPoint").as_mut(), &spine, (160.0, 330.0), mods);
        assert_eq!(sent(&a, "object.blend.spine.reshapeSegment"), Some(perpendicular), "{a:?}");
    }
}

#[test]
fn direct_and_group_selection_pick_blend_keys() {
    let (d, g, k1) = blend_doc();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let a = DirectSelectionTool::new(false).pointer(&cx, &PointerEvent::new(PointerKind::Down, 104.0, 304.0));
    assert_eq!(a, vec![Action::Exec("select.set".into(), json!({"ids": [k1.0]}))]);
    let a = DirectSelectionTool::new(true).pointer(&cx, &PointerEvent::new(PointerKind::Down, 104.0, 304.0));
    assert_eq!(a, vec![Action::Exec("select.set".into(), json!({"ids": [k1.0]}))]);
    // A click on a step (between the keys) still selects the blend.
    let a = DirectSelectionTool::new(true).pointer(&cx, &PointerEvent::new(PointerKind::Down, 160.0, 304.0));
    assert_eq!(a, vec![Action::Exec("select.set".into(), json!({"ids": [g.0]}))]);
}

#[test]
fn dragging_a_frame_corner_or_edge_reshapes_area_type() {
    let (d, text) = doc_with_area_type();
    let p = paint();
    let s = Selection::default();
    let cx1 = cx(&d, &s, &p);
    let mut t = DirectSelectionTool::new(false);
    // The bottom-right corner of the 120 × 40 frame at (300, 300) selects the text...
    assert_eq!(
        t.pointer(&cx1, &PointerEvent::new(PointerKind::Down, 421.0, 339.0)),
        vec![Action::Exec("select.set".into(), json!({"ids": [text.0]})), Action::Notify(FRAME_PICKED.into())]
    );
    // ...and drags that corner.
    let a = t.pointer(&cx1, &PointerEvent::new(PointerKind::Drag, 461.0, 399.0));
    assert_eq!(a[0], Action::Begin("Reshape Type Area".into()));
    assert_eq!(a[1], Action::Preview("text.reshapeArea".into(), json!({"id": text.0, "anchors": [[0, 2]], "dx": 40.0, "dy": 60.0})));
    assert_eq!(t.pointer(&cx1, &PointerEvent::new(PointerKind::Up, 461.0, 399.0)), vec![Action::Commit, Action::Notify(FRAME_PICKED.into())]);
    // The top edge drags both its ends; a click alone changes nothing.
    let mut s = Selection::default();
    s.add(text);
    let cx2 = cx(&d, &s, &p);
    assert!(t.pointer(&cx2, &PointerEvent::new(PointerKind::Down, 350.0, 301.0)).is_empty());
    let a = t.pointer(&cx2, &PointerEvent::new(PointerKind::Drag, 350.0, 281.0));
    assert_eq!(a[1], Action::Preview("text.reshapeArea".into(), json!({"id": text.0, "anchors": [[0, 0], [0, 1]], "dx": 0.0, "dy": -20.0})));
    t.pointer(&cx2, &PointerEvent::new(PointerKind::Up, 350.0, 281.0));
    assert_eq!(t.pointer(&cx2, &PointerEvent::new(PointerKind::Down, 300.0, 340.0)), vec![Action::Notify(FRAME_PICKED.into())]);
    assert!(t.pointer(&cx2, &PointerEvent::new(PointerKind::Up, 300.0, 340.0)).is_empty());
    // The selected frame's corners show, the one clicked filled.
    assert_eq!(t.overlays(&cx2).iter().filter(|o| matches!(o, Overlay::Anchor { filled: false, .. })).count(), 3);
    assert_eq!(t.overlays(&cx2).iter().filter(|o| matches!(o, Overlay::Anchor { p, filled: true, .. } if *p == Point::new(300.0, 340.0))).count(), 1);
}

/// The frame anchor squares of the selected area type that `t` draws: (filled, hollow).
fn frame_squares(t: &DirectSelectionTool, c: &ToolContext) -> (Vec<Point>, Vec<Point>) {
    let mut out = (vec![], vec![]);
    for o in t.overlays(c) {
        if let Overlay::Anchor { p, filled, size: 5.0, .. } = o {
            if filled { out.0.push(p) } else { out.1.push(p) }
        }
    }
    out
}

/// A Direct Selection marquee selects the frame anchors of area type inside it, which show
/// filled: a drag of one moves them all, the arrow keys nudge them, and Shift toggles them.
#[test]
fn a_marquee_selects_frame_anchors_of_area_type() {
    let (d, text) = doc_with_area_type();
    let p = paint();
    let none = Selection::default();
    let cx1 = cx(&d, &none, &p);
    let mut t = DirectSelectionTool::new(false);
    let ev = |k, x, y| PointerEvent::new(k, x, y);
    let shift = |k, x, y| PointerEvent::new(k, x, y).with_mods(Mods { shift: true, ..Mods::default() });
    // A marquee round the 120 × 40 frame's right-hand corners, (420, 300) and (420, 340).
    assert!(t.pointer(&cx1, &ev(PointerKind::Down, 400.0, 280.0)).is_empty());
    t.pointer(&cx1, &ev(PointerKind::Drag, 440.0, 360.0));
    assert_eq!(
        t.pointer(&cx1, &ev(PointerKind::Up, 440.0, 360.0)),
        vec![
            Action::Exec("select.anchorsMany".into(), json!({"items": [{"id": text.0, "anchors": []}], "mode": "set"})),
            Action::Notify(FRAME_PICKED.into()),
        ]
    );
    let mut s = Selection::default();
    s.add(text);
    let cx2 = cx(&d, &s, &p);
    t.notify(&cx2, FRAME_PICKED);
    let (filled, hollow) = frame_squares(&t, &cx2);
    assert_eq!(filled, vec![Point::new(420.0, 300.0), Point::new(420.0, 340.0)]);
    assert_eq!(hollow, vec![Point::new(300.0, 300.0), Point::new(300.0, 340.0)]);
    // A drag of one of them moves both.
    assert_eq!(t.pointer(&cx2, &ev(PointerKind::Down, 420.0, 340.0)), vec![Action::Notify(FRAME_PICKED.into())]);
    let a = t.pointer(&cx2, &ev(PointerKind::Drag, 440.0, 360.0));
    assert_eq!(a[0], Action::Begin("Reshape Type Area".into()));
    assert_eq!(a[1], Action::Preview("text.reshapeArea".into(), json!({"id": text.0, "anchors": [[0, 1], [0, 2]], "dx": 20.0, "dy": 20.0})));
    assert_eq!(t.pointer(&cx2, &ev(PointerKind::Up, 440.0, 360.0)), vec![Action::Commit, Action::Notify(FRAME_PICKED.into())]);
    // The arrow keys nudge them by the keyboard increment, ten times that with Shift.
    let nudged = |anchors: Value, dx: f64, dy: f64| {
        vec![
            Action::Exec("text.reshapeArea".into(), json!({"id": text.0, "anchors": anchors, "dx": dx, "dy": dy})),
            Action::Notify(FRAME_PICKED.into()),
        ]
    };
    assert!(t.claims_key(&cx2, ToolKey::Right) && !t.claims_key(&cx2, ToolKey::Enter));
    assert_eq!(t.key(&cx2, ToolKey::Right, Mods::default()), nudged(json!([[0, 1], [0, 2]]), 1.0, 0.0));
    assert_eq!(t.key(&cx2, ToolKey::Up, Mods { shift: true, ..Mods::default() }), nudged(json!([[0, 1], [0, 2]]), 0.0, -10.0));
    // Any other change of the document or the selection (Deselect, then the same type selected
    // again) leaves the type selected whole: the arrow keys are no longer the tool's.
    let mut later = cx(&d, &s, &p);
    later.revision = (0, 1);
    assert!(frame_squares(&t, &later).0.is_empty());
    assert!(!t.claims_key(&later, ToolKey::Right));
    // Shift-click takes one out; the other still nudges.
    assert_eq!(t.pointer(&cx2, &shift(PointerKind::Down, 420.0, 300.0)), vec![Action::Notify(FRAME_PICKED.into())]);
    assert!(t.pointer(&cx2, &shift(PointerKind::Up, 420.0, 300.0)).is_empty());
    assert_eq!(frame_squares(&t, &cx2).0, vec![Point::new(420.0, 340.0)]);
    assert_eq!(t.key(&cx2, ToolKey::Down, Mods::default()), nudged(json!([[0, 2]]), 0.0, 1.0));
    // A Shift-marquee round the bottom corners toggles them: (300, 340) joins, (420, 340) leaves.
    t.pointer(&cx2, &shift(PointerKind::Down, 280.0, 330.0));
    t.pointer(&cx2, &shift(PointerKind::Drag, 440.0, 360.0));
    let a = t.pointer(&cx2, &shift(PointerKind::Up, 440.0, 360.0));
    assert_eq!(a[0], Action::Exec("select.anchorsMany".into(), json!({"items": [], "mode": "toggle"})), "the type stays selected");
    assert_eq!(frame_squares(&t, &cx2).0, vec![Point::new(300.0, 340.0)]);
    // When the selection changes, the frame anchors are no longer selected.
    assert!(frame_squares(&t, &cx1).0.is_empty());
    assert!(!t.claims_key(&cx1, ToolKey::Right) && t.key(&cx1, ToolKey::Right, Mods::default()).is_empty());
    // A marquee that also reaches a path's anchor (the rectangle's corner at (200, 200)) selects
    // the type whole, as before: the arrow keys move the path's anchor and the type alike.
    let rect = d.layers[0].children().and_then(|c| c.first()).map(|n| n.id).unwrap();
    t.pointer(&cx1, &ev(PointerKind::Down, 440.0, 360.0));
    t.pointer(&cx1, &ev(PointerKind::Drag, 190.0, 190.0));
    let a = t.pointer(&cx1, &ev(PointerKind::Up, 190.0, 190.0));
    let items = json!([{"id": rect.0, "anchors": [[0, 2]]}, {"id": text.0, "anchors": []}]);
    assert_eq!(a[0], Action::Exec("select.anchorsMany".into(), json!({"items": items, "mode": "set"})));
    let mut mixed = Selection::default();
    mixed.add(rect);
    mixed.anchors.insert(rect, [(0, 2)].into());
    mixed.add(text);
    let cx3 = cx(&d, &mixed, &p);
    t.notify(&cx3, FRAME_PICKED);
    assert!(frame_squares(&t, &cx3).0.is_empty());
    assert!(!t.claims_key(&cx3, ToolKey::Right));
    // Group Selection's marquee selects the type whole, without its frame anchors.
    let mut g = DirectSelectionTool::new(true);
    g.pointer(&cx1, &ev(PointerKind::Down, 400.0, 280.0));
    g.pointer(&cx1, &ev(PointerKind::Drag, 440.0, 360.0));
    assert_eq!(
        g.pointer(&cx1, &ev(PointerKind::Up, 440.0, 360.0)),
        vec![Action::Exec("select.anchorsMany".into(), json!({"items": [{"id": text.0, "anchors": []}], "mode": "set"}))]
    );
}

/// A click without a drag on one of several selected frame anchors selects it alone, as a click on
/// a selected path anchor does; a drag of it still moves them all.
#[test]
fn a_click_on_a_selected_frame_anchor_selects_it_alone() {
    let (d, text) = doc_with_area_type();
    let p = paint();
    let none = Selection::default();
    let cx1 = cx(&d, &none, &p);
    let mut t = DirectSelectionTool::new(false);
    let ev = |k, x, y| PointerEvent::new(k, x, y);
    // A marquee round the right-hand corners, (420, 300) and (420, 340).
    t.pointer(&cx1, &ev(PointerKind::Down, 400.0, 280.0));
    t.pointer(&cx1, &ev(PointerKind::Drag, 440.0, 360.0));
    t.pointer(&cx1, &ev(PointerKind::Up, 440.0, 360.0));
    let mut s = Selection::default();
    s.add(text);
    let cx2 = cx(&d, &s, &p);
    t.notify(&cx2, FRAME_PICKED);
    assert_eq!(frame_squares(&t, &cx2).0, vec![Point::new(420.0, 300.0), Point::new(420.0, 340.0)]);
    // A click on (420, 340) keeps both until the release, then selects it alone.
    assert_eq!(t.pointer(&cx2, &ev(PointerKind::Down, 420.0, 340.0)), vec![Action::Notify(FRAME_PICKED.into())]);
    assert_eq!(frame_squares(&t, &cx2).0.len(), 2, "both, for a drag");
    assert_eq!(t.pointer(&cx2, &ev(PointerKind::Up, 420.0, 340.0)), vec![Action::Notify(FRAME_PICKED.into())]);
    t.notify(&cx2, FRAME_PICKED);
    assert_eq!(frame_squares(&t, &cx2).0, vec![Point::new(420.0, 340.0)]);
    assert_eq!(
        t.key(&cx2, ToolKey::Right, Mods::default()),
        vec![
            Action::Exec("text.reshapeArea".into(), json!({"id": text.0, "anchors": [[0, 2]], "dx": 1.0, "dy": 0.0})),
            Action::Notify(FRAME_PICKED.into()),
        ]
    );
}

/// #3980: type on a path shows its path's anchors and a drag of one reshapes the path, the last
/// one too although the end bracket stands on it; the bracket's stem above it drags the bracket.
#[test]
fn dragging_an_anchor_of_type_on_a_path_reshapes_its_path() {
    let (d, text) = doc_with_path_type();
    let p = paint();
    let s = Selection::default();
    let cx1 = cx(&d, &s, &p);
    let mut t = DirectSelectionTool::new(false);
    assert_eq!(
        t.pointer(&cx1, &PointerEvent::new(PointerKind::Down, 101.0, 301.0)),
        vec![Action::Exec("select.set".into(), json!({"ids": [text.0]}))]
    );
    let a = t.pointer(&cx1, &PointerEvent::new(PointerKind::Drag, 121.0, 261.0));
    assert_eq!(a[0], Action::Begin("Reshape Type Path".into()));
    assert_eq!(a[1], Action::Preview("text.reshapePath".into(), json!({"id": text.0, "anchors": [[0, 0]], "dx": 20.0, "dy": -40.0})));
    assert_eq!(t.pointer(&cx1, &PointerEvent::new(PointerKind::Up, 121.0, 261.0)), vec![Action::Commit]);
    let mut s = Selection::default();
    s.add(text);
    let cx2 = cx(&d, &s, &p);
    let anchors: Vec<Point> =
        t.overlays(&cx2).iter().filter_map(|o| if let Overlay::Anchor { p, filled: false, .. } = o { Some(*p) } else { None }).collect();
    assert_eq!(anchors, vec![Point::new(100.0, 300.0), Point::new(400.0, 300.0)]);
    assert!(t.pointer(&cx2, &PointerEvent::new(PointerKind::Down, 400.0, 300.0)).is_empty());
    let a = t.pointer(&cx2, &PointerEvent::new(PointerKind::Drag, 400.0, 260.0));
    assert_eq!(a.last(), Some(&Action::Preview("text.reshapePath".into(), json!({"id": text.0, "anchors": [[0, 1]], "dx": 0.0, "dy": -40.0}))));
    t.pointer(&cx2, &PointerEvent::new(PointerKind::Up, 400.0, 260.0));
    t.pointer(&cx2, &PointerEvent::new(PointerKind::Down, 400.0, 290.0));
    let a = t.pointer(&cx2, &PointerEvent::new(PointerKind::Drag, 340.0, 290.0));
    assert!(matches!(a.last(), Some(Action::Preview(c, _)) if c == "type.pathOptions"), "{a:?}");
}

#[test]
fn click_anchor_selects_it() {
    let (d, id) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = DirectSelectionTool::new(false);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
    assert_eq!(a, vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 0]], "mode": "set"}))]);
}

/// Pressing an anchor of a path whose anchors are all selected (stored as the whole path)
/// keeps them all selected, so the drag moves them all; Shift still deselects that anchor.
#[test]
fn pressing_an_anchor_of_a_fully_selected_path_keeps_the_selection() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = DirectSelectionTool::new(false);
    assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0)).is_empty());
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 120.0, 100.0));
    assert!(matches!(a.as_slice(), [Action::Begin(_), Action::Preview(c, _)] if c == "object.transform"), "{a:?}");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 120.0, 100.0));
    let shift = Mods { shift: true, ..Default::default() };
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0).with_mods(shift));
    assert_eq!(a, vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 0]], "mode": "toggle"}))]);
}

/// A click (no drag) on an anchor that is selected with others, whether the whole path is or
/// some of its anchors, selects that anchor alone, as in Illustrator: a press keeps the
/// selection so a drag moves it all, the release without a drag narrows it. Shift still
/// toggles, and a click on a path's only selected anchor changes nothing visible.
#[test]
fn a_click_on_a_selected_anchor_selects_it_alone() {
    let (d, id) = doc_with_rect();
    let p = paint();
    let alone = vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 0]], "mode": "set"}))];
    let mut whole = Selection::default();
    whole.set([id]);
    for s in [whole, anchors_of(id, &[(0, 0), (0, 1)])] {
        let cx = cx(&d, &s, &p);
        let mut t = DirectSelectionTool::new(false);
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0)).is_empty(), "{s:?}");
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 100.0, 100.0)), alone, "{s:?}");
        // Dragged instead: moved, selection kept.
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0)).is_empty());
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 120.0, 100.0));
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 120.0, 100.0)), vec![Action::Commit], "{s:?}");
    }
    // An anchor not yet selected is selected on the press, and the release adds nothing.
    let none = Selection::default();
    let cx = cx(&d, &none, &p);
    let mut t = DirectSelectionTool::new(false);
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0)), alone);
    assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 100.0, 100.0)).is_empty());
}

/// Alt-pressing an anchor picks the whole path, as Alt on a segment does, and the drag copies
/// it as the Selection tool's Alt-drag does.
#[test]
fn alt_dragging_an_anchor_copies_the_whole_path() {
    let (d, id) = doc_with_rect();
    let alt = Mods { alt: true, ..Default::default() };
    let mut whole = Selection::default();
    whole.set([id]);
    let p = paint();
    for (s, pick) in [(Selection::default(), true), (anchors_of(id, &[(0, 0)]), true), (whole, false)] {
        let cx = cx(&d, &s, &p);
        let mut t = DirectSelectionTool::new(false);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0).with_mods(alt));
        let set = vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))];
        assert_eq!(a, if pick { set } else { vec![] }, "{s:?}");
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 120.0, 100.0).with_mods(alt));
        assert!(matches!(a.as_slice(), [Action::Begin(_), Action::Preview(c, v)] if c == "object.transform" && v["copy"] == true), "{a:?}");
    }
}

#[test]
fn marquee_selects_anchors() {
    let (d, id) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = DirectSelectionTool::new(false);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 50.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 250.0, 150.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 150.0));
    assert_eq!(a, vec![Action::Exec("select.anchorsMany".into(), json!({"items": [{"id": id.0, "anchors": [[0, 1]]}], "mode": "set"}))]);
}

/// #4654: a marquee over part of a segment, with neither of its anchors inside, selects that
/// segment as a click on it does; with Shift it toggles it. Group Selection selects objects.
#[test]
fn a_marquee_over_part_of_a_segment_selects_it() {
    let (d, id) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    // Across the middle of the square's top edge, (100, 100)–(200, 100).
    let drag = |t: &mut DirectSelectionTool, m: Mods| {
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 140.0, 80.0).with_mods(m));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 160.0, 120.0).with_mods(m));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 160.0, 120.0).with_mods(m))
    };
    let segment = |mode: &str| Action::Exec("select.segments".into(), json!({"id": id.0, "segments": [[0, 0]], "mode": mode}));
    let mut t = DirectSelectionTool::new(false);
    assert_eq!(drag(&mut t, Mods::default()), vec![Action::Exec("select.anchorsMany".into(), json!({"items": [], "mode": "set"})), segment("add")]);
    let shift = Mods { shift: true, ..Mods::default() };
    assert_eq!(drag(&mut t, shift).get(1), Some(&segment("toggle")));
    let mut g = DirectSelectionTool::new(true);
    assert!(!drag(&mut g, Mods::default()).iter().any(|a| matches!(a, Action::Exec(c, _) if c == "select.segments")));
}

#[test]
fn marquee_selects_ruler_guides() {
    let (mut d, id) = doc_with_rect();
    d.guides.push(vectorcraft_doc::Guide::new(false, 80.0));
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = DirectSelectionTool::new(false);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 50.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 250.0, 150.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 150.0));
    assert_eq!(
        a,
        vec![
            Action::Exec("select.anchorsMany".into(), json!({"items": [{"id": id.0, "anchors": [[0, 1]]}], "mode": "set"})),
            Action::Exec("guide.select".into(), json!({"indexes": [0], "toggle": true})),
        ]
    );
}

/// #2441: Group Selection picks one path of a compound path (the outer shape or a hole);
/// a second click on it adds the compound path, like a group around a member.
#[test]
fn group_selection_picks_a_path_of_a_compound_path() {
    let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let (outer, hole, compound) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    let rect = |id, a: f64, b: f64| {
        std::sync::Arc::new(vectorcraft_doc::Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(a, a, b, b)), Default::default()))
    };
    let mut c = vectorcraft_doc::Node::new(
        compound,
        vectorcraft_doc::NodeKind::Compound {
            children: vec![rect(outer, 0.0, 200.0), rect(hole, 50.0, 150.0)],
            rule: vectorcraft_geom::FillRule::EvenOdd,
        },
    );
    c.appearance = vectorcraft_doc::Appearance::default_art().into();
    d.insert(Some(l), 0, c).unwrap();
    let p = paint();
    let none = Selection::default();
    let click = |s: &Selection, x, y| DirectSelectionTool::new(true).pointer(&cx(&d, s, &p), &PointerEvent::new(PointerKind::Down, x, y));
    let set = |id: NodeId| vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))];
    assert_eq!(click(&none, 25.0, 100.0), set(outer), "the fill between the outline and the hole");
    assert_eq!(click(&none, 50.0, 100.0), set(hole), "the hole's outline");
    assert!(click(&none, 100.0, 100.0).is_empty(), "inside the hole nothing is hit");
    let mut s = Selection::default();
    s.set([outer]);
    assert_eq!(click(&s, 25.0, 100.0), vec![Action::Exec("select.add".into(), json!({"ids": [compound.0]}))]);
}

/// Group Selection Shift-click on a selected compound-path member deselects it on release.
#[test]
fn group_selection_shift_click_deselects_a_compound_path_member() {
    let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let (outer, hole, compound) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    let rect = |id, a: f64, b: f64| {
        std::sync::Arc::new(vectorcraft_doc::Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(a, a, b, b)), Default::default()))
    };
    let mut c = vectorcraft_doc::Node::new(
        compound,
        vectorcraft_doc::NodeKind::Compound {
            children: vec![rect(outer, 0.0, 200.0), rect(hole, 50.0, 150.0)],
            rule: vectorcraft_geom::FillRule::EvenOdd,
        },
    );
    c.appearance = vectorcraft_doc::Appearance::default_art().into();
    d.insert(Some(l), 0, c).unwrap();
    let p = paint();
    let mut s = Selection::default();
    s.set([outer]);
    let shift = Mods { shift: true, ..Mods::default() };
    let cx = cx(&d, &s, &p);
    let mut t = DirectSelectionTool::new(true);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 25.0, 100.0).with_mods(shift));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 25.0, 100.0).with_mods(shift));
    assert_eq!(a, vec![Action::Exec("select.toggle".into(), json!({"id": outer.0}))]);
}

/// A path inside an inner group, a sibling path outside it, and the groups' ids.
/// The inside path is the rectangle at (100, 100); the outside one at (300, 300).
fn nested_group_doc() -> (vectorcraft_doc::Document, NodeId, NodeId, NodeId, NodeId) {
    let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let (inside, outside, inner, outer) = (d.alloc_id(), d.alloc_id(), d.alloc_id(), d.alloc_id());
    let path = |id, x| {
        std::sync::Arc::new(vectorcraft_doc::Node::path(
            id,
            vectorcraft_geom::shapes::rectangle(Rect::new(x, x, x + 100.0, x + 100.0)),
            vectorcraft_doc::Appearance::default_art(),
        ))
    };
    let inner_g = vectorcraft_doc::Node::group(inner, vec![path(inside, 100.0)]);
    let outer_g = vectorcraft_doc::Node::group(outer, vec![std::sync::Arc::new(inner_g), path(outside, 300.0)]);
    d.insert(Some(l), 0, outer_g).unwrap();
    (d, inside, outside, inner, outer)
}

/// Isolation mode: a marquee and an anchor click take the path inside the isolated group and
/// leave the path beside it. Group Selection stops at the isolated group instead of adding
/// the group around it.
#[test]
fn isolation_mode_leaves_art_outside_the_isolated_group() {
    let (d, inside, outside, inner, outer) = nested_group_doc();
    let p = paint();
    let none = Selection::default();
    let isolated = ToolContext { isolation: Some(inner), ..cx(&d, &none, &p) };
    let mut t = DirectSelectionTool::new(false);
    t.pointer(&isolated, &PointerEvent::new(PointerKind::Down, 50.0, 50.0));
    let a = t.pointer(&isolated, &PointerEvent::new(PointerKind::Up, 450.0, 450.0));
    let items = a.first().and_then(|act| match act {
        Action::Exec(c, v) if c == "select.anchorsMany" => v.get("items").cloned(),
        _ => None,
    });
    let items = items.unwrap();
    assert!(items.as_array().is_some_and(|a| a.iter().all(|item| item["id"] == inside.0)), "marquee: {a:?}");
    assert!(items.as_array().is_some_and(|a| !a.is_empty()), "the path inside is still selected: {a:?}");

    let a = DirectSelectionTool::new(false).pointer(&isolated, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
    assert!(a.is_empty(), "a click on {outside:?}, outside isolation, starts a marquee: {a:?}");
    let a = DirectSelectionTool::new(false).pointer(&isolated, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
    assert_eq!(a, vec![Action::Exec("select.anchors".into(), json!({"id": inside.0, "anchors": [[0, 0]], "mode": "set"}))]);

    let mut s = Selection::default();
    s.set([inside, inner]);
    let climbed = ToolContext { isolation: Some(inner), ..cx(&d, &s, &p) };
    let a = DirectSelectionTool::new(true).pointer(&climbed, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
    assert!(a.is_empty(), "does not add the group outside isolation ({outer:?}): {a:?}");
}

/// #4466: in isolation mode a Direct Selection double-click outside the isolated art (on the
/// object beside it, or on nothing) leaves isolation mode, as the Selection tool's does; on the
/// isolated art it doesn't.
#[test]
fn a_double_click_outside_isolation_leaves_it() {
    let (d, _, _, inner, _) = nested_group_doc();
    let p = paint();
    let none = Selection::default();
    let isolated = ToolContext { isolation: Some(inner), ..cx(&d, &none, &p) };
    let double = |x: f64, y: f64| DirectSelectionTool::new(false).pointer(&isolated, &PointerEvent::new(PointerKind::DoubleClick, x, y));
    let exit = vec![Action::Exec("object.exitIsolation".into(), json!({}))];
    assert_eq!(double(350.0, 350.0), exit, "on the object beside the isolated group");
    assert_eq!(double(700.0, 700.0), exit, "on nothing");
    assert!(double(150.0, 150.0).is_empty(), "on the isolated art");
    assert!(DirectSelectionTool::new(false).pointer(&cx(&d, &none, &p), &PointerEvent::new(PointerKind::DoubleClick, 700.0, 700.0)).is_empty());
}

/// With Direct Selection, an Alt-click on a path selected whole adds its group on release, and
/// each further Alt-click adds the next group up, as Group Selection's clicks do, on an anchor,
/// a segment and the fill alike. The climb stops below the layer and at the isolated group.
#[test]
fn alt_clicks_on_a_path_selected_whole_add_its_groups() {
    let (d, inside, _, inner, outer) = nested_group_doc();
    let p = paint();
    let alt = Mods { alt: true, ..Default::default() };
    let add = |id: NodeId| vec![Action::Exec("select.add".into(), json!({"ids": [id.0]}))];
    let click = |cx: &ToolContext, x: f64, y: f64| {
        let mut t = DirectSelectionTool::new(false);
        let down = t.pointer(cx, &PointerEvent::new(PointerKind::Down, x, y).with_mods(alt));
        assert!(down.is_empty(), "({x}, {y}): {down:?}");
        t.pointer(cx, &PointerEvent::new(PointerKind::Up, x, y).with_mods(alt))
    };
    for (x, y) in [(100.0, 100.0), (150.0, 100.0), (150.0, 150.0)] {
        let mut s = Selection::default();
        s.set([inside]);
        assert_eq!(click(&cx(&d, &s, &p), x, y), add(inner), "({x}, {y}): the path's group");
        s.set([inside, inner]);
        assert_eq!(click(&cx(&d, &s, &p), x, y), add(outer), "({x}, {y}): the group around it");
        s.set([inside, inner, outer]);
        assert!(click(&cx(&d, &s, &p), x, y).is_empty(), "({x}, {y}): the layer is not added");
        s.set([inside, inner]);
        let isolated = ToolContext { isolation: Some(inner), ..cx(&d, &s, &p) };
        assert!(click(&isolated, x, y).is_empty(), "({x}, {y}): not the group outside isolation");
    }
}

/// In a sublayer, a Group Selection click and a Direct Selection Alt-click climb to the group
/// and stop there: the sublayer is not added.
#[test]
fn group_clicks_in_a_sublayer_stop_below_the_sublayer() {
    use vectorcraft_doc::{Appearance, Document, LayerColor, Node};
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let (sub, g, a) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    d.insert(Some(l), 0, Node::layer(sub, "Sub", LayerColor::Preset(1))).unwrap();
    d.insert(Some(sub), 0, Node::group(g, vec![])).unwrap();
    d.insert(Some(g), 0, Node::path(a, vectorcraft_geom::shapes::rectangle(Rect::new(100.0, 100.0, 200.0, 200.0)), Appearance::default_art()))
        .unwrap();
    let p = paint();
    let alt = Mods { alt: true, ..Default::default() };
    let click = |s: &Selection, group: bool, mods: Mods| {
        let c = cx(&d, s, &p);
        let mut t = DirectSelectionTool::new(group);
        let down = t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 150.0).with_mods(mods));
        let up = t.pointer(&c, &PointerEvent::new(PointerKind::Up, 150.0, 150.0).with_mods(mods));
        [down, up].concat()
    };
    let add = |id: NodeId| vec![Action::Exec("select.add".into(), json!({"ids": [id.0]}))];
    let mut s = Selection::default();
    s.set([a]);
    assert_eq!(click(&s, true, Mods::default()), add(g), "Group Selection adds the group");
    assert_eq!(click(&s, false, alt), add(g), "an Alt-click adds the group");
    s.set([a, g]);
    assert!(click(&s, true, Mods::default()).is_empty(), "Group Selection does not add the sublayer");
    assert!(click(&s, false, alt).is_empty(), "an Alt-click does not add the sublayer");
}

/// When a group holds a sublayer, the climb stops at the sublayer: it adds neither the sublayer
/// nor the group above it.
#[test]
fn group_clicks_stop_at_a_sublayer_inside_a_group() {
    use vectorcraft_doc::{Appearance, Document, LayerColor, Node};
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let (outer, sub, g, a) = (d.alloc_id(), d.alloc_id(), d.alloc_id(), d.alloc_id());
    d.insert(Some(l), 0, Node::group(outer, vec![])).unwrap();
    d.insert(Some(outer), 0, Node::layer(sub, "Sub", LayerColor::Preset(1))).unwrap();
    d.insert(Some(sub), 0, Node::group(g, vec![])).unwrap();
    d.insert(Some(g), 0, Node::path(a, vectorcraft_geom::shapes::rectangle(Rect::new(100.0, 100.0, 200.0, 200.0)), Appearance::default_art()))
        .unwrap();
    let p = paint();
    let alt = Mods { alt: true, ..Default::default() };
    let click = |s: &Selection, group: bool, mods: Mods| {
        let c = cx(&d, s, &p);
        let mut t = DirectSelectionTool::new(group);
        let down = t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 150.0).with_mods(mods));
        let up = t.pointer(&c, &PointerEvent::new(PointerKind::Up, 150.0, 150.0).with_mods(mods));
        [down, up].concat()
    };
    let add = |id: NodeId| vec![Action::Exec("select.add".into(), json!({"ids": [id.0]}))];
    let mut s = Selection::default();
    s.set([a]);
    assert_eq!(click(&s, true, Mods::default()), add(g), "Group Selection adds the group");
    assert_eq!(click(&s, false, alt), add(g), "an Alt-click adds the group");
    s.set([a, g]);
    assert!(click(&s, true, Mods::default()).is_empty(), "Group Selection does not add the sublayer");
    assert!(click(&s, false, alt).is_empty(), "an Alt-click does not add the sublayer");
}

/// An Alt-drag from a path selected whole inside a group copies the path and adds no group, and
/// Alt+Shift-click on a segment or the fill still toggles the path, adding no group either.
#[test]
fn alt_drags_and_alt_shift_clicks_on_a_path_in_a_group_add_no_group() {
    let (d, inside, ..) = nested_group_doc();
    let p = paint();
    let mut s = Selection::default();
    s.set([inside]);
    let cx = cx(&d, &s, &p);
    let alt = Mods { alt: true, ..Default::default() };
    for (x, y) in [(100.0, 100.0), (150.0, 100.0), (150.0, 150.0)] {
        let mut t = DirectSelectionTool::new(false);
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, x, y).with_mods(alt)).is_empty());
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x + 20.0, y).with_mods(alt));
        assert!(matches!(a.as_slice(), [Action::Begin(_), Action::Preview(c, v)] if c == "object.transform" && v["copy"] == true), "{a:?}");
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, x + 20.0, y).with_mods(alt)), vec![Action::Commit]);
    }
    let alt_shift = Mods { alt: true, shift: true, ..Default::default() };
    for (x, y) in [(150.0, 100.0), (150.0, 150.0)] {
        let mut t = DirectSelectionTool::new(false);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, x, y).with_mods(alt_shift));
        assert_eq!(a, vec![Action::Exec("select.toggle".into(), json!({"id": inside.0}))], "({x}, {y})");
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, x, y).with_mods(alt_shift)).is_empty(), "({x}, {y})");
    }
}

/// #927: a marquee across type selects the type object whole, beside the anchors it holds.
#[test]
fn marquee_selects_type_it_touches() {
    // The rectangle at (100, 100)–(200, 200) and area type at (300, 300).
    let (d, text) = doc_with_area_type();
    let rect = d.layers[0].children().and_then(|c| c.first()).map(|n| n.id).unwrap();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = DirectSelectionTool::new(false);
    // From empty canvas across the type to inside the rectangle (its bottom-right corner).
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 450.0, 450.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 150.0));
    let items = json!([{"id": rect.0, "anchors": [[0, 2]]}, {"id": text.0, "anchors": []}]);
    // The frame's corners are inside too: they are selected with the type.
    assert_eq!(a, vec![Action::Exec("select.anchorsMany".into(), json!({"items": items, "mode": "set"})), Action::Notify(FRAME_PICKED.into())]);
    // Away from the type, only anchors.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 50.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 150.0));
    assert_eq!(a, vec![Action::Exec("select.anchorsMany".into(), json!({"items": [{"id": rect.0, "anchors": [[0, 1]]}], "mode": "set"}))]);
}

/// A marquee across a member of an Intertwine group selects the member's anchors, as in a
/// plain group, and not the Intertwine group whole.
#[test]
fn marquee_selects_the_anchors_of_intertwine_members() {
    let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let (a, b, g) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    let path = |id, x| {
        std::sync::Arc::new(vectorcraft_doc::Node::path(
            id,
            vectorcraft_geom::shapes::rectangle(Rect::new(x, 100.0, x + 100.0, 200.0)),
            vectorcraft_doc::Appearance::default_art(),
        ))
    };
    let intertwine = vectorcraft_doc::Node::new(g, vectorcraft_doc::NodeKind::Intertwine { children: vec![path(a, 100.0), path(b, 300.0)] });
    d.insert(Some(l), 0, intertwine).unwrap();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    for group in [false, true] {
        let mut t = DirectSelectionTool::new(group);
        // Around the first member's top-right corner.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 50.0));
        let actions = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 150.0));
        let items = json!([{"id": a.0, "anchors": [[0, 1]]}]);
        assert_eq!(actions, vec![Action::Exec("select.anchorsMany".into(), json!({"items": items, "mode": "set"}))], "group: {group}");
    }
}

/// Shift-drag a marquee: the anchors inside toggle (#483), with Group Selection too.
#[test]
fn shift_marquee_toggles_anchors() {
    let (d, id) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let shift = Mods { shift: true, ..Default::default() };
    for group in [false, true] {
        let mut t = DirectSelectionTool::new(group);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 50.0).with_mods(shift));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 250.0, 150.0).with_mods(shift));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 150.0).with_mods(shift));
        assert_eq!(a, vec![Action::Exec("select.anchorsMany".into(), json!({"items": [{"id": id.0, "anchors": [[0, 1]]}], "mode": "toggle"}))]);
    }
}

/// Group Selection: E pressed during a marquee drag makes it select the objects entirely inside
/// it whole, a group's member without its group. Direct Selection does not take E.
#[test]
fn e_during_a_group_selection_marquee_selects_the_objects_inside() {
    use vectorcraft_doc::{Appearance, Node};
    let (mut d, id) = doc_with_rect();
    let l = d.layers[0].id;
    let rect = |id, r| Node::path(id, vectorcraft_geom::shapes::rectangle(r), Appearance::default_art());
    let (g, a, b) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    d.insert(Some(l), 1, Node::group(g, vec![])).unwrap();
    d.insert(Some(g), 0, rect(a, Rect::new(100.0, 220.0, 120.0, 240.0))).unwrap();
    d.insert(Some(g), 1, rect(b, Rect::new(100.0, 300.0, 120.0, 320.0))).unwrap();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let e = ToolKey::Letter('e');
    let drag = |group: bool, mods: Mods| {
        let mut t = DirectSelectionTool::new(group);
        assert!(!t.claims_key(&cx, e), "no marquee yet");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 50.0).with_mods(mods));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 300.0, 250.0).with_mods(mods));
        assert_eq!(t.claims_key(&cx, e), group);
        t.key(&cx, e, Mods::default());
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 300.0, 250.0).with_mods(mods))
    };
    assert_eq!(drag(true, Mods::default()), vec![Action::Exec("select.set".into(), json!({"ids": [id.0, a.0]}))]);
    let shift = Mods { shift: true, ..Default::default() };
    assert_eq!(drag(true, shift), vec![Action::Exec("select.toggle".into(), json!({"ids": [id.0, a.0]}))]);
    // Direct Selection's marquee still selects the anchors inside it.
    let anchors = |id: NodeId, list: Value| json!({"id": id.0, "anchors": list});
    let all = json!([[0, 0], [0, 1], [0, 2], [0, 3]]);
    assert_eq!(
        drag(false, Mods::default()),
        vec![Action::Exec("select.anchorsMany".into(), json!({"items": [anchors(id, all.clone()), anchors(a, all)], "mode": "set"}))]
    );
}

/// An open path from (100, 300) to (200, 300) arching up through (150, 262.5).
fn arch_doc() -> (vectorcraft_doc::Document, NodeId) {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0)], false);
    sp.anchors[0].h_out = Point::new(100.0, 250.0);
    sp.anchors[1].h_in = Point::new(200.0, 250.0);
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, PathData::single(sp), vectorcraft_doc::Appearance::default_art())).unwrap();
    (d, id)
}

/// #1743: Escape mid-drag cancels it; the release after it does nothing more.
#[test]
fn escape_cancels_a_drag() {
    let (d, _) = arch_doc();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = DirectSelectionTool::new(false);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 262.5));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 240.0));
    assert!(t.busy());
    assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
    assert!(!t.busy());
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 220.0)), vec![]);
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 220.0)), vec![]);
    assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![], "nothing to cancel");
}

/// Escape also cancels a corner widget drag and a drag of the brackets of type on a path, and
/// the release after it does nothing. After a press on a widget that has not moved, Escape drops
/// the press, so its release selects no corner.
#[test]
fn escape_cancels_a_corner_widget_or_bracket_drag() {
    let p = paint();
    let (d, rect) = doc_with_rect();
    let mut s = Selection::default();
    s.set([rect]);
    let corners = cx(&d, &s, &p);
    let (d2, text) = doc_with_path_type();
    let mut s2 = Selection::default();
    s2.add(text);
    let brackets = cx(&d2, &s2, &p);
    // (what, its context, the press, the drag's first step)
    for (what, c, from, to) in [
        ("corner widget", &corners, Point::new(110.0, 110.0), Point::new(120.0, 120.0)),
        ("bracket", &brackets, Point::new(400.0, 290.0), Point::new(340.0, 290.0)),
    ] {
        let mut t = DirectSelectionTool::new(false);
        assert!(t.pointer(c, &PointerEvent::new(PointerKind::Down, from.x, from.y)).is_empty(), "{what}");
        let a = t.pointer(c, &PointerEvent::new(PointerKind::Drag, to.x, to.y));
        assert!(matches!(a.first(), Some(Action::Begin(_))), "{what}: {a:?}");
        assert!(t.busy(), "{what}");
        assert_eq!(t.key(c, ToolKey::Escape, Mods::default()), vec![Action::Cancel], "{what}: Escape cancels the drag");
        assert!(!t.busy(), "{what}");
        assert!(t.pointer(c, &PointerEvent::new(PointerKind::Drag, to.x - 5.0, to.y)).is_empty(), "{what}: the drag is over");
        assert!(t.pointer(c, &PointerEvent::new(PointerKind::Up, to.x - 5.0, to.y)).is_empty(), "{what}: the release does nothing");
    }
    let mut t = DirectSelectionTool::new(false);
    t.pointer(&corners, &PointerEvent::new(PointerKind::Down, 110.0, 110.0));
    assert_eq!(t.key(&corners, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
    assert!(t.pointer(&corners, &PointerEvent::new(PointerKind::Up, 110.0, 110.0)).is_empty(), "the click selects no corner");
}

#[test]
fn dragging_a_curved_segment_reshapes_it_and_a_straight_one_moves_its_anchors() {
    let (d, arch) = arch_doc();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = DirectSelectionTool::new(false);
    // The arch: its segment gets selected, and the drag bends it.
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 262.5));
    assert_eq!(a, vec![Action::Exec("select.segments".into(), json!({"id": arch.0, "segments": [[0, 0]]}))]);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 242.5));
    assert_eq!(a[0], Action::Begin("Reshape".into()));
    let Action::Preview(cmd, v) = &a[1] else { panic!("{a:?}") };
    assert_eq!(
        (cmd.as_str(), v["id"].clone(), v["segment"].clone(), v["dx"].clone(), v["dy"].clone()),
        ("path.reshapeSegment", json!(arch.0), json!(0), json!(0.0), json!(-20.0))
    );
    assert!((v["t"].as_f64().unwrap() - 0.5).abs() < 0.01, "{v}");
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 242.5)), vec![Action::Commit]);
    // A straight edge of the rectangle: its two anchors get selected and move.
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 101.0));
    assert!(matches!(&a[..], [Action::Exec(c, v)] if c == "select.segments" && v["segments"] == json!([[0, 0]])), "{a:?}");
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 81.0));
    assert_eq!(a[1], Action::Preview("path.moveAnchors".into(), json!({"dx": 0.0, "dy": -20.0})));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 81.0));
    // Alt on the stroke picks the whole path, and Alt-drag copies it.
    let alt = Mods { alt: true, ..Default::default() };
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 101.0).with_mods(alt));
    assert!(matches!(&a[..], [Action::Exec(c, _)] if c == "select.set"), "{a:?}");
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 81.0).with_mods(alt));
    assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, v)] if c == "object.transform" && v["copy"] == true), "{a:?}");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 81.0).with_mods(alt));
    // The fill still selects the whole path, and a path selected as a whole still moves whole.
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
    assert!(matches!(&a[..], [Action::Exec(c, _)] if c == "select.set"), "{a:?}");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 150.0));
    let rect = d.layers[0].children().unwrap()[0].id;
    let mut whole = Selection::default();
    whole.set([rect]);
    let cx = crate::testutil::cx(&d, &whole, &p);
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 101.0)), vec![]);
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 81.0));
    assert!(matches!(&a[..], [Action::Begin(_), Action::Preview(c, _)] if c == "object.transform"), "{a:?}");
}

/// #494: the handles shown are the handles dragged. A path selected as a whole (the one the
/// Pen just drew) shows all its handles, and they drag; the direct-selected anchors' handles
/// only otherwise; none with Show handles when multiple anchors are selected off and several
/// anchors selected. A handle shorter than the tolerance leaves its anchor to be picked, and
/// the tolerance is the preference's, in screen pixels at any zoom.
#[test]
fn the_handles_shown_are_the_handles_dragged() {
    let (d, arch) = arch_doc();
    let p = paint();
    // Does a press at (x, y) take a handle?
    let handle = |c: &ToolContext, x: f64, y: f64| {
        DirectSelectionTool::new(false).pointer(c, &PointerEvent::new(PointerKind::Down, x, y)) == vec![Action::Begin("Reshape".into())]
    };
    let mut whole = Selection::default();
    whole.set([arch]);
    let c = cx(&d, &whole, &p);
    assert!(handle(&c, 101.0, 251.0), "the first anchor's handle, the path selected whole");
    let mut t = DirectSelectionTool::new(false);
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 101.0, 251.0));
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 120.0, 240.0));
    assert!(matches!(&a[..], [Action::Preview(c, v)] if c == "path.setHandle" && v["anchor"] == 0 && v["which"] == "out"), "{a:?}");
    // Only the second anchor direct-selected: the first one's handle is hidden.
    let second = anchors_of(arch, &[(0, 1)]);
    assert!(!handle(&cx(&d, &second, &p), 101.0, 251.0));
    assert!(handle(&cx(&d, &second, &p), 201.0, 251.0));
    // Show handles when multiple anchors are selected off: the whole path's are hidden.
    assert!(!handle(&ToolContext { handles_multiple: false, ..cx(&d, &whole, &p) }, 101.0, 251.0));
    assert!(handle(&ToolContext { handles_multiple: false, ..cx(&d, &second, &p) }, 201.0, 251.0));
    // The tolerance is the preference's, in screen pixels: 3 px is 0.75 pt at 400%.
    let zoomed = ToolContext { zoom: 4.0, ..cx(&d, &whole, &p) };
    assert!(!handle(&zoomed, 101.0, 251.0));
    assert!(handle(&zoomed, 100.5, 250.5));
    assert!(handle(&ToolContext { selection_tolerance: 8.0, ..cx(&d, &whole, &p) }, 106.0, 254.0));
}

/// #494: a handle 2 pt long, inside the tolerance round its anchor, doesn't hide the anchor.
#[test]
fn a_short_handle_leaves_its_anchor_to_be_picked() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let mut sp = vectorcraft_geom::SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(200.0, 300.0)], false);
    sp.anchors[0].h_out = Point::new(102.0, 300.0);
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, PathData::single(sp), vectorcraft_doc::Appearance::default_art())).unwrap();
    let s = anchors_of(id, &[(0, 0)]);
    let p = paint();
    let c = cx(&d, &s, &p);
    let mut t = DirectSelectionTool::new(false);
    assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 300.0)), vec![], "the anchor, already selected");
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 90.0, 290.0));
    assert_eq!(a[0], Action::Begin("Move".into()));
    t.pointer(&c, &PointerEvent::new(PointerKind::Up, 90.0, 290.0));
    assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 102.5, 300.0)), vec![Action::Begin("Reshape".into())], "the handle");
}

/// Direct Selection's cursors from Illustrator's manual: the white arrow at rest; over an anchor
/// a square, hollow on an unselected path and filled on a selected one; the Reshape Segment
/// cursor over a curved segment a drag reshapes, not over a straight one (which moves) or a
/// path selected whole (which moves whole); Group Selection's plus with Alt and as its tool.
#[test]
fn hovering_shows_the_anchor_square_reshape_and_group_cursors() {
    let (d, arch) = arch_doc();
    let p = paint();
    let m = Mods::default();
    let pt = Point::new;
    let none = Selection::default();
    let c = cx(&d, &none, &p);
    let t = DirectSelectionTool::new(false);
    assert_eq!(t.cursor(&c, pt(20.0, 20.0), m), Cursor::ArrowHollow);
    assert_eq!(t.cursor(&c, pt(100.0, 300.0), m), Cursor::DirectAnchor);
    assert_eq!(t.cursor(&c, pt(150.0, 262.5), m), Cursor::ReshapeSegment);
    assert_eq!(t.cursor(&c, pt(150.0, 100.0), m), Cursor::ArrowHollow, "a straight segment moves");
    assert_eq!(t.cursor(&c, pt(150.0, 150.0), m), Cursor::ArrowHollow, "the fill");
    let alt = Mods { alt: true, ..m };
    assert_eq!(t.cursor(&c, pt(100.0, 300.0), alt), Cursor::GroupSelection);
    assert_eq!(DirectSelectionTool::new(true).cursor(&c, pt(100.0, 300.0), m), Cursor::GroupSelection);
    let mut whole = Selection::default();
    whole.set([arch]);
    let c = cx(&d, &whole, &p);
    assert_eq!(t.cursor(&c, pt(200.0, 300.0), m), Cursor::DirectAnchorSelected);
    assert_eq!(t.cursor(&c, pt(150.0, 262.5), m), Cursor::ArrowHollow, "a path selected whole moves whole");
    // One anchor selected: its handle end shows and keeps the plain arrow, Alt or not (a press
    // takes the handle).
    let s = anchors_of(arch, &[(0, 0)]);
    let c = cx(&d, &s, &p);
    assert_eq!(t.cursor(&c, pt(100.0, 250.0), m), Cursor::ArrowHollow, "a handle end");
    assert_eq!(t.cursor(&c, pt(100.0, 250.0), alt), Cursor::ArrowHollow, "a handle end with Alt");
    assert_eq!(t.cursor(&c, pt(200.0, 300.0), m), Cursor::DirectAnchorSelected);
    // Bending the curve keeps the Reshape Segment cursor.
    let mut t = DirectSelectionTool::new(false);
    let c = cx(&d, &none, &p);
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 262.5));
    t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 150.0, 230.0));
    assert_eq!(t.cursor(&c, pt(150.0, 230.0), m), Cursor::ReshapeSegment);
}

/// A drag shows the filled arrow, hollow while it snaps to a point; an Alt-drag copy shows the
/// double arrowhead.
#[test]
fn drags_show_the_filled_arrow_hollow_on_a_snap_and_alt_the_copy_cursor() {
    let (d, _, b) = two_squares();
    let p = paint();
    let none = Mods::default();
    let at = |kind, x, y, m: Mods| PointerEvent::new(kind, x, y).with_mods(m);
    let one = anchors_of(b, &[(0, 0)]);
    let c = cx(&d, &one, &p);
    let mut t = DirectSelectionTool::new(false);
    t.pointer(&c, &at(PointerKind::Down, 300.0, 300.0, none));
    t.pointer(&c, &at(PointerKind::Drag, 250.0, 260.0, none));
    assert_eq!(t.cursor(&c, Point::new(250.0, 260.0), none), Cursor::Arrow);
    // Onto A's bottom-right corner.
    t.pointer(&c, &at(PointerKind::Drag, 203.0, 199.0, none));
    assert_eq!(t.cursor(&c, Point::new(203.0, 199.0), none), Cursor::ArrowHollow);
    t.pointer(&c, &at(PointerKind::Up, 203.0, 199.0, none));
    // Shift with a Constrain Angle of 10°: the constraint alone isn't a snap.
    let shift = Mods { shift: true, ..none };
    let tilted = ToolContext { constrain_angle: 10.0, ..cx(&d, &one, &p) };
    t.pointer(&tilted, &at(PointerKind::Down, 300.0, 300.0, shift));
    t.pointer(&tilted, &at(PointerKind::Drag, 380.0, 262.0, shift));
    assert_eq!(t.cursor(&tilted, Point::new(380.0, 262.0), shift), Cursor::Arrow);
    t.pointer(&tilted, &at(PointerKind::Up, 380.0, 262.0, shift));
    let mut whole = Selection::default();
    whole.set([b]);
    let c = cx(&d, &whole, &p);
    let alt = Mods { alt: true, ..none };
    t.pointer(&c, &at(PointerKind::Down, 300.0, 300.0, alt));
    t.pointer(&c, &at(PointerKind::Drag, 250.0, 260.0, alt));
    assert_eq!(t.cursor(&c, Point::new(250.0, 260.0), alt), Cursor::SelectionCopy);
}

/// [`doc_with_rect`] (A, 100..200) and a second square B, 300..400: (A, B).
fn two_squares() -> (vectorcraft_doc::Document, NodeId, NodeId) {
    let (mut d, a) = doc_with_rect();
    let l = d.layers[0].id;
    let b = d.alloc_id();
    let sq = vectorcraft_geom::shapes::rectangle(Rect::new(300.0, 300.0, 400.0, 400.0));
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(b, sq, vectorcraft_doc::Appearance::default_art())).unwrap();
    (d, a, b)
}

/// An anchor of a selected path is picked before the one of an unselected path on a layer
/// above that covers it, as Highlight anchors on mouse over already marks it (#1508); with
/// nothing selected, the topmost path's anchor is picked.
#[test]
fn a_selected_path_s_anchor_is_picked_before_one_above_it() {
    let (mut d, a) = doc_with_rect();
    let top = d.add_layer(None);
    let b = d.alloc_id();
    let sq = vectorcraft_geom::shapes::rectangle(Rect::new(101.0, 100.0, 201.0, 200.0));
    d.insert(Some(top), 0, vectorcraft_doc::Node::path(b, sq, vectorcraft_doc::Appearance::default_art())).unwrap();
    let p = paint();
    let pick = |id: NodeId| vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 0]], "mode": "set"}))];
    let s = anchors_of(a, &[(0, 2)]);
    let mut t = DirectSelectionTool::new(false);
    assert_eq!(t.pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, 101.0, 100.0)), pick(a));
    let mut whole = Selection::default();
    whole.set([a]);
    let mut t = DirectSelectionTool::new(false);
    let cx2 = cx(&d, &whole, &p);
    assert!(t.pointer(&cx2, &PointerEvent::new(PointerKind::Down, 101.0, 100.0)).is_empty());
    assert_eq!(t.pointer(&cx2, &PointerEvent::new(PointerKind::Up, 101.0, 100.0)), pick(a));
    let none = Selection::default();
    let mut t = DirectSelectionTool::new(false);
    assert_eq!(t.pointer(&cx(&d, &none, &p), &PointerEvent::new(PointerKind::Down, 101.0, 100.0)), pick(b));
}

/// The anchors `anchors` of path `id` direct-selected.
fn anchors_of(id: NodeId, anchors: &[AnchorRef]) -> Selection {
    let mut s = Selection::default();
    s.set([id]);
    s.anchors.insert(id, anchors.iter().copied().collect());
    s
}

/// Press at `from`, drag to `to`: the move the drag previews, and the smart guide labels shown.
fn drag(t: &mut DirectSelectionTool, cx: &ToolContext, from: (f64, f64), to: (f64, f64), mods: Mods) -> (Value, Vec<String>) {
    t.pointer(cx, &PointerEvent::new(PointerKind::Down, from.0, from.1).with_mods(mods));
    let a = t.pointer(cx, &PointerEvent::new(PointerKind::Drag, to.0, to.1).with_mods(mods));
    let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
    let labels = t
        .overlays(cx)
        .into_iter()
        .filter_map(|o| match o {
            Overlay::Label { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(t.pointer(cx, &PointerEvent::new(PointerKind::Up, to.0, to.1).with_mods(mods)), vec![Action::Commit]);
    assert!(t.overlays(cx).iter().all(|o| !matches!(o, Overlay::Label { .. })), "the guides go with the drag");
    (v.clone(), labels)
}

/// Alt-dragging a path by an anchor copies it, and that anchor snaps as a dragged anchor does:
/// onto another path's anchor, or onto an anchor of the original the copy leaves behind.
#[test]
fn alt_dragged_copies_snap_by_the_anchor_pressed() {
    let (d, _, b) = two_squares();
    let p = paint();
    let alt = Mods { alt: true, ..Default::default() };
    let mut whole = Selection::default();
    whole.set([b]);
    let c = cx(&d, &whole, &p);
    let mut t = DirectSelectionTool::new(false);
    let copy =
        |dx: f64, dy: f64| json!({"matrix": matrix_json(vectorcraft_geom::Affine::translate(vectorcraft_geom::Vec2::new(dx, dy))), "copy": true});
    // B's top-left corner, pressed 1 pt off, onto A's bottom-right one.
    let (v, labels) = drag(&mut t, &c, (301.0, 301.0), (203.0, 199.0), alt);
    assert_eq!((v, labels), (copy(-100.0, -100.0), vec!["anchor".to_string()]));
    // B's top-left corner onto the original B's top-right one: the copy sits beside it.
    let (v, labels) = drag(&mut t, &c, (300.0, 300.0), (402.0, 301.0), alt);
    assert_eq!((v, labels), (copy(100.0, 0.0), vec!["anchor".to_string()]));
    // The history says what the drag did.
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 300.0, 300.0).with_mods(alt));
    let out = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 250.0, 250.0).with_mods(alt));
    assert_eq!(out.first(), Some(&Action::Begin("Copy".into())));
}

/// Dragging a fully selected path by an anchor moves it whole, so Alt pressed or released
/// during the drag turns the copy on and off, as with the Selection tool; the anchor pressed
/// snaps all along, onto the original only while it is left behind.
#[test]
fn alt_during_a_drag_by_an_anchor_toggles_the_copy() {
    let (d, _, b) = two_squares();
    let p = paint();
    let (none, alt) = (Mods::default(), Mods { alt: true, ..Default::default() });
    let mut whole = Selection::default();
    whole.set([b]);
    let c = cx(&d, &whole, &p);
    let mut t = DirectSelectionTool::new(false);
    let preview = |dx: f64, dy: f64, copy: bool| {
        Action::Preview(
            "object.transform".into(),
            json!({"matrix": matrix_json(vectorcraft_geom::Affine::translate(vectorcraft_geom::Vec2::new(dx, dy))), "copy": copy}),
        )
    };
    assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 300.0, 300.0)).is_empty());
    assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 250.0, 250.0)), vec![Action::Begin("Move".into()), preview(-50.0, -50.0, false)]);
    // The tool sees the document as previewed: B moved along, not where the copy leaves it.
    let (mut moved, _, b2) = two_squares();
    let l = moved.layers[0].id;
    let sq = vectorcraft_geom::shapes::rectangle(Rect::new(250.0, 250.0, 350.0, 350.0));
    moved.remove(b2).unwrap();
    moved.insert(Some(l), 1, vectorcraft_doc::Node::path(b, sq, vectorcraft_doc::Appearance::default_art())).unwrap();
    let c = cx(&moved, &whole, &p);
    // Alt down mid-drag: a copy, whose pressed corner can land on the original's top-right one.
    assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 402.0, 301.0).with_mods(alt)), vec![preview(100.0, 0.0, true)]);
    // Alt up: a move again, which leaves nothing behind to snap to.
    assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 402.0, 301.0).with_mods(none)), vec![preview(102.0, 1.0, false)]);
    assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 402.0, 301.0)), vec![Action::Commit]);
    // With only some anchors selected, the drag still moves just those.
    let one = anchors_of(b, &[(0, 0)]);
    let c = cx(&d, &one, &p);
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 250.0, 260.0).with_mods(alt));
    assert!(matches!(a.last(), Some(Action::Preview(c, _)) if c == "path.moveAnchors"), "{a:?}");
}

/// A drag from the fill of a path selected whole, while another path has some anchors selected,
/// moves the selected anchors and the paths selected whole (`path.moveAnchors`), as a drag of
/// an anchor does, with Direct Selection and Group Selection alike. Alt, pressed or released at
/// any time during the drag, turns the copy of the selected objects on and off, and a selection
/// of whole paths only still moves as objects.
#[test]
fn a_drag_from_a_fill_with_anchors_selected_moves_the_anchors() {
    let (d, a, b) = two_squares();
    let p = paint();
    let mut mixed = anchors_of(b, &[(0, 0)]);
    mixed.objects.insert(0, a);
    let mut whole = Selection::default();
    whole.set([a, b]);
    let alt = Mods { alt: true, ..Default::default() };
    for group in [false, true] {
        let c = cx(&d, &mixed, &p);
        let mut t = DirectSelectionTool::new(group);
        assert!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 150.0)).is_empty(), "group: {group}");
        let out = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 171.0, 153.0));
        assert_eq!(
            out,
            vec![Action::Begin("Move".into()), Action::Preview("path.moveAnchors".into(), json!({"dx": 21.0, "dy": 3.0}))],
            "group: {group}"
        );
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 171.0, 153.0)), vec![Action::Commit]);
        // Alt held at the start, released and pressed again: a copy, a move, a copy.
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 150.0).with_mods(alt));
        let out = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 171.0, 153.0).with_mods(alt));
        assert!(matches!(out.last(), Some(Action::Preview(c, v)) if c == "object.transform" && v["copy"] == true), "group: {group} {out:?}");
        assert_eq!(out.first(), Some(&Action::Begin("Move".into())), "the history names it by the last preview");
        let out = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 172.0, 153.0));
        assert_eq!(out, vec![Action::Preview("path.moveAnchors".into(), json!({"dx": 22.0, "dy": 3.0}))], "group: {group}");
        let out = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 173.0, 153.0).with_mods(alt));
        assert!(matches!(out.as_slice(), [Action::Preview(c, v)] if c == "object.transform" && v["copy"] == true), "group: {group} {out:?}");
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 173.0, 153.0).with_mods(alt));
        let c = cx(&d, &whole, &p);
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
        let out = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 171.0, 153.0));
        assert!(matches!(out.last(), Some(Action::Preview(c, _)) if c == "object.transform"), "group: {group} {out:?}");
    }
}

/// A drag from the fill of A, selected whole, while B's top-left anchor is selected snaps the
/// point it holds as a dragged anchor does: near an anchor of B that stays put, it lands on it.
/// With Snap to Grid on, the box around A and B's selected anchor, (100, 100)–(300, 300), lands an
/// edge on the 9 pt grid instead: moved by (21, 3), its right side and bottom (321, 303) are
/// nearest the gridlines 324 and 306, and with Shift the level move by 13 lands the right side on
/// 315.
#[test]
fn a_drag_from_a_fill_with_anchors_selected_snaps() {
    let (d, a, b) = two_squares();
    let p = paint();
    let mut mixed = anchors_of(b, &[(0, 0)]);
    mixed.objects.insert(0, a);
    let none = Mods::default();
    for group in [false, true] {
        let mut t = DirectSelectionTool::new(group);
        let c = cx(&d, &mixed, &p);
        let (v, labels) = drag(&mut t, &c, (150.0, 150.0), (298.0, 401.0), none);
        assert_eq!((v, labels), (json!({"dx": 150.0, "dy": 250.0}), vec!["anchor".to_string()]), "group: {group}");
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 298.0, 401.0));
        let c = ToolContext { snap_to_grid: true, ..cx(&d, &mixed, &p) };
        assert_eq!(drag(&mut t, &c, (150.0, 150.0), (171.0, 153.0), none).0, json!({"dx": 24.0, "dy": 6.0}), "group: {group}");
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 171.0, 153.0));
        let (v, _) = drag(&mut t, &c, (150.0, 150.0), (163.0, 151.0), Mods { shift: true, ..none });
        let (x, y) = (v["dx"].as_f64().unwrap(), v["dy"].as_f64().unwrap());
        assert!((x - 15.0).abs() < 1e-9 && y.abs() < 1e-9, "group: {group}: {v}");
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 163.0, 151.0));
    }
}

/// A path inside a selected group moves whole with the group, even with some of its anchors
/// direct-selected, so a drag from a fill does not snap to its other anchors: here to the
/// bottom-right corner (400, 400) of the path at (300, 300)–(400, 400).
#[test]
fn a_drag_from_a_fill_skips_the_anchors_of_a_path_in_a_selected_group() {
    let (d, _, outside, _, outer) = nested_group_doc();
    let p = paint();
    let mut sel = anchors_of(outside, &[(0, 0)]);
    sel.objects.insert(0, outer);
    let c = cx(&d, &sel, &p);
    let mut t = DirectSelectionTool::new(true);
    let (v, labels) = drag(&mut t, &c, (150.0, 150.0), (398.0, 401.0), Mods::default());
    assert_eq!((v, labels), (json!({"dx": 248.0, "dy": 251.0}), Vec::<String>::new()));
}

/// A dragged anchor lands on another path's anchor or segment, lines up with the other anchors
/// of its own path, and with several selected the one pressed on snaps, the others following.
#[test]
fn dragged_anchors_snap_to_points_segments_and_smart_guides() {
    let (d, _, b) = two_squares();
    let p = paint();
    let none = Mods::default();
    let one = anchors_of(b, &[(0, 0)]);
    let c = cx(&d, &one, &p);
    let mut t = DirectSelectionTool::new(false);
    // B's top-left corner, pressed 1 pt off, onto A's bottom-right one.
    let (v, labels) = drag(&mut t, &c, (301.0, 301.0), (203.0, 199.0), none);
    assert_eq!((v, labels), (json!({"dx": -100.0, "dy": -100.0}), vec!["anchor".to_string()]));
    // Onto A's right side.
    let (v, labels) = drag(&mut t, &c, (300.0, 300.0), (203.0, 150.0), none);
    assert_eq!((v, labels), (json!({"dx": -100.0, "dy": -150.0}), vec!["path".to_string()]));
    // In line with B's own bottom-left corner, which stays put.
    let (v, labels) = drag(&mut t, &c, (300.0, 300.0), (303.0, 330.0), none);
    assert_eq!((v, labels), (json!({"dx": 0.0, "dy": 30.0}), vec!["align".to_string()]));
    // Shift keeps the move at 45° steps and still snaps along that line (#886): on the
    // diagonal from B's corner, onto A's bottom-right corner…
    let shift = Mods { shift: true, ..none };
    let (v, labels) = drag(&mut t, &c, (300.0, 300.0), (203.0, 199.0), shift);
    let near = |v: &serde_json::Value, (x, y): (f64, f64)| (v["dx"].as_f64().unwrap() - x).hypot(v["dy"].as_f64().unwrap() - y) < 1e-9;
    assert!(near(&v, (-100.0, -100.0)) && labels == ["anchor"], "{v} {labels:?}");
    // …and with nothing in reach, at the pointer's distance along its angle.
    let (v, labels) = drag(&mut t, &c, (300.0, 300.0), (380.0, 302.0), shift);
    let s = move_delta(Point::new(300.0, 300.0), Point::new(380.0, 302.0), true, 0.0);
    assert!(near(&v, (s.x, s.y)) && labels.is_empty(), "{v} {labels:?}");
    // Two anchors: the one pressed on (B's top-right) lands on A's corner.
    let two = anchors_of(b, &[(0, 0), (0, 1)]);
    let c = cx(&d, &two, &p);
    let (v, _) = drag(&mut t, &c, (400.0, 300.0), (202.0, 198.0), none);
    assert_eq!(v, json!({"dx": -200.0, "dy": -100.0}));
    // Smart Guides off: Snap to Point pulls within its distance (2 px) only.
    let c = ToolContext { smart_guides: false, ..cx(&d, &one, &p) };
    assert_eq!(drag(&mut t, &c, (300.0, 300.0), (201.0, 199.0), none).0, json!({"dx": -100.0, "dy": -100.0}));
    assert_eq!(drag(&mut t, &c, (300.0, 300.0), (203.0, 199.0), none).0, json!({"dx": -97.0, "dy": -101.0}));
    let c = ToolContext { smart_guides: false, snap_to_point: false, ..cx(&d, &one, &p) };
    assert_eq!(drag(&mut t, &c, (300.0, 300.0), (201.0, 199.0), none).0, json!({"dx": -99.0, "dy": -101.0}));
}

/// #2120: the end of a slanted straight line, dragged roughly along it, stays on its direction
/// (Line Extension) and the guide runs along it from the other end.
#[test]
fn dragged_end_of_a_slanted_line_stays_on_its_direction() {
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let line = vectorcraft_geom::shapes::line(Point::new(300.0, 400.0), Point::new(380.0, 340.0));
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(id, line, vectorcraft_doc::Appearance::default_art())).unwrap();
    let p = paint();
    let end = anchors_of(id, &[(0, 1)]);
    let c = cx(&d, &end, &p);
    let mut t = DirectSelectionTool::new(false);
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 380.0, 340.0));
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 421.5, 312.0));
    let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
    let (dx, dy) = (v["dx"].as_f64().unwrap(), v["dy"].as_f64().unwrap());
    assert!((dx - 40.0).abs() < 1e-9 && (dy + 30.0).abs() < 1e-9, "{v}");
    let guides: Vec<(Point, Point)> =
        t.overlays(&c).into_iter().filter_map(|o| if let Overlay::Line { a, b, .. } = o { Some((a, b)) } else { None }).collect();
    assert!(matches!(guides[..], [(a, b)] if a == Point::new(300.0, 400.0) && (b - Point::new(420.0, 310.0)).hypot() < 1e-9), "{guides:?}");
}

/// Group Selection (and Direct Selection on a whole path) moves objects snapping as the
/// Selection tool does: B's left side lines up with A's right one.
#[test]
fn moved_objects_snap_with_smart_guides() {
    let (d, _, b) = two_squares();
    let p = paint();
    let mut s = Selection::default();
    s.set([b]);
    let c = cx(&d, &s, &p);
    for group in [true, false] {
        let mut t = DirectSelectionTool::new(group);
        let (v, _) = drag(&mut t, &c, (350.0, 350.0), (253.0, 351.0), Mods::default());
        assert_eq!(v["matrix"], json!([1.0, 0.0, 0.0, 1.0, -100.0, 1.0]), "group: {group}");
    }
}

/// #1816: Group Selection and Direct Selection move a whole path with Snap to Grid as the
/// Selection tool does, landing an edge on the grid; the center does not snap. A 45 pt square at
/// 90–135 on the 9 pt grid, dragged right by 22 to 112–157, lands its left edge on the gridline
/// 108 and moves by 18. Its center, at 134.5 after the drag, is nearer the gridline 135 and
/// does not snap.
#[test]
fn a_move_of_objects_lands_an_edge_on_the_grid() {
    let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let sq = vectorcraft_geom::shapes::rectangle(Rect::new(90.0, 90.0, 135.0, 135.0));
    d.insert(Some(l), 0, vectorcraft_doc::Node::path(id, sq, vectorcraft_doc::Appearance::default_art())).unwrap();
    let p = paint();
    let mut s = Selection::default();
    s.set([id]);
    let c = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
    for group in [true, false] {
        let mut t = DirectSelectionTool::new(group);
        let (v, _) = drag(&mut t, &c, (110.0, 110.0), (132.0, 110.0), Mods::default());
        assert_eq!(v["matrix"], json!([1.0, 0.0, 0.0, 1.0, 18.0, 0.0]), "group: {group}");
    }
}

/// Group Selection and Direct Selection on a whole path keep a Shift-constrained move on its
/// line when it snaps, as the Selection tool does: level by 13.04 with Snap to Grid on, B's
/// right side (413.04) lands on the gridline 414, and its top stays at 300, between 297 and
/// 306.
#[test]
fn a_shift_move_of_objects_snaps_along_its_line() {
    let (d, _, b) = two_squares();
    let p = paint();
    let mut s = Selection::default();
    s.set([b]);
    let c = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
    let shift = Mods { shift: true, ..Mods::default() };
    for group in [true, false] {
        let mut t = DirectSelectionTool::new(group);
        let (v, _) = drag(&mut t, &c, (350.0, 350.0), (363.0, 351.0), shift);
        let (x, y) = (v["matrix"][4].as_f64().unwrap(), v["matrix"][5].as_f64().unwrap());
        assert!((x - 14.0).abs() < 1e-9 && y.abs() < 1e-9, "group: {group}: {v}");
    }
}

/// Direct Selection's double-click on type where a layer's clipping mask edge runs through it
/// edits the type, though a press there takes the mask's segment.
#[test]
fn a_double_click_on_type_at_a_layer_masks_edge_edits_it() {
    let (d, _, at) = doc_with_type_on_a_layer_masks_edge();
    let (s, p) = (Selection::default(), paint());
    let acts = DirectSelectionTool::new(false).pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::DoubleClick, at.x, at.y));
    assert_eq!(acts, vec![Action::SwitchTool("type".into())]);
}

#[test]
fn double_clicking_type_switches_to_type_tool() {
    use std::sync::Arc;
    let (mut d, id) = doc_with_area_type();
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let b = d.node(id).unwrap().geometric_bounds().unwrap();
    let mid = b.center();
    // Direct Selection (shortcut A, group == false) switches to the Type tool on double click.
    let mut t = DirectSelectionTool::new(false);
    let acts = t.pointer(&c, &PointerEvent::new(PointerKind::DoubleClick, mid.x, mid.y));
    assert_eq!(acts, vec![Action::SwitchTool("type".into())]);

    // Group Selection (group == true) does not switch to Type tool.
    let mut gt = DirectSelectionTool::new(true);
    let acts_group = gt.pointer(&c, &PointerEvent::new(PointerKind::DoubleClick, mid.x, mid.y));
    assert!(acts_group.is_empty(), "group selection shouldn't switch: {acts_group:?}");

    // Text inside a group also switches to Type tool with Direct Selection.
    let gid = d.alloc_id();
    let text_id = d.alloc_id();
    let text_node = Node::new(
        text_id,
        vectorcraft_doc::NodeKind::Text(Box::new(vectorcraft_doc::TextObject::point(Point::new(50.0, 50.0), "In group", Default::default()))),
    );
    d.insert(Some(d.layers[0].id), 2, Node::group(gid, vec![Arc::new(text_node)])).unwrap();
    let in_group_b = d.node(gid).unwrap().geometric_bounds().unwrap();
    let c2 = cx(&d, &s, &p);
    let acts_group_text = t.pointer(&c2, &PointerEvent::new(PointerKind::DoubleClick, in_group_b.center().x, in_group_b.center().y));
    assert_eq!(acts_group_text, vec![Action::SwitchTool("type".into())]);
}

#[test]
fn direct_selection_click_and_drag_member_of_compound_path() {
    let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let (outer, hole, compound) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    let rect = |id, a: f64, b: f64| {
        std::sync::Arc::new(vectorcraft_doc::Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(a, a, b, b)), Default::default()))
    };
    let mut c = vectorcraft_doc::Node::new(
        compound,
        vectorcraft_doc::NodeKind::Compound {
            children: vec![rect(outer, 0.0, 200.0), rect(hole, 50.0, 150.0)],
            rule: vectorcraft_geom::FillRule::EvenOdd,
        },
    );
    c.appearance = vectorcraft_doc::Appearance::default_art().into();
    d.insert(Some(l), 0, c).unwrap();
    let p = paint();
    let none = Selection::default();
    let mut t = DirectSelectionTool::new(false);
    let ctx = cx(&d, &none, &p);
    let down = t.pointer(&ctx, &PointerEvent::new(PointerKind::Down, 25.0, 100.0));
    assert_eq!(
        down,
        vec![Action::Exec("select.set".into(), json!({"ids": [outer.0]}))],
        "Direct Selection selects child member, not compound parent"
    );
    let mut s = Selection::default();
    s.set([outer]);
    let ctx_sel = cx(&d, &s, &p);
    let drag = t.pointer(&ctx_sel, &PointerEvent::new(PointerKind::Drag, 35.0, 110.0));
    assert!(drag.iter().any(|a| matches!(a, Action::Preview(cmd, _) if cmd == "object.transform")), "drags the selected child member: {drag:?}");
    let up = t.pointer(&ctx_sel, &PointerEvent::new(PointerKind::Up, 35.0, 110.0));
    assert_eq!(up, vec![Action::Commit]);
}

#[test]
fn shift_clicking_anchor_after_segment_pick_toggles_or_adds() {
    let (d, id) = doc_with_rect();
    let p = paint();
    let mut s = Selection::default();
    s.set([id]);
    s.segments.insert(id, [(0, 0)].into());
    s.anchors.insert(id, [(0, 0), (0, 1)].into());
    let cx = cx(&d, &s, &p);
    let mut t = DirectSelectionTool::new(false);
    let shift = Mods { shift: true, ..Default::default() };
    // Shift-click on anchor (0, 0) (top-left at 100, 100) toggles it off
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0).with_mods(shift));
    assert_eq!(a, vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 0]], "mode": "toggle"}))]);
    // Shift-click on anchor (0, 2) (bottom-right at 200, 200) toggles it on (adds it)
    let a2 = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.0, 200.0).with_mods(shift));
    assert_eq!(a2, vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[0, 2]], "mode": "toggle"}))]);
}

#[test]
fn shift_clicking_segment_toggles_segment() {
    let (d, id) = doc_with_rect();
    let p = paint();
    let s = Selection::default();
    let cx = cx(&d, &s, &p);
    let mut t = DirectSelectionTool::new(false);
    let shift = Mods { shift: true, ..Default::default() };
    // The rectangle is at 100..200. Segment 0 is the top edge (y=100, x between 100 and 200).
    // Shift-click at (150, 100) selects segment 0 with mode "toggle".
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 100.0).with_mods(shift));
    assert_eq!(a, vec![Action::Exec("select.segments".into(), json!({"id": id.0, "segments": [[0, 0]], "mode": "toggle"}))]);
}

/// In normal viewmode, clicking the fill of an open path in front selects the open path's shape
/// and anchors, and NEVER clicks through to select or grab a path behind it (even if the path
/// behind has an anchor directly under the click).
#[test]
fn clicking_fill_of_open_path_in_front_never_selects_path_behind() {
    use vectorcraft_geom::{Anchor, SubPath};
    let (mut d, _behind) = doc_with_rect();
    // `behind` has an anchor at (100.0, 100.0).
    // Create an open path in front (id `front`), with a fill, covering (100.0, 100.0).
    let front = d.alloc_id();
    let l = d.layers[0].id;
    // Triangle from (50, 50) to (150, 50) to (150, 150), open (implicit closing edge from (150, 150) to (50, 50)).
    // (100.0, 100.0) is inside this filled triangle, but `front` has NO anchor at (100.0, 100.0).
    let anchors = vec![Anchor::corner(Point::new(50.0, 50.0)), Anchor::corner(Point::new(150.0, 50.0)), Anchor::corner(Point::new(150.0, 150.0))];
    let open_path = PathData::single(SubPath::new(anchors, false));
    let ap = vectorcraft_doc::Appearance::default_art(); // has solid white fill
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(front, open_path, ap)).unwrap();

    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);

    // Hovering over (100.0, 100.0) does NOT hover the hidden anchor of `behind`.
    assert_eq!(hovered_anchor(&c, Point::new(100.0, 100.0)), None);

    // Clicking at (100.0, 100.0) selects `front` (the open path in front), NOT `behind`.
    let mut t = DirectSelectionTool::new(false);
    let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
    assert_eq!(acts, vec![Action::Exec("select.set".into(), json!({"ids": [front.0]}))]);
}

/// A selected path's Bézier direction handle positioned over a background filled rectangle
/// can be hit and dragged without being occluded by the background fill.
#[test]
fn selected_path_handle_over_background_rect_is_draggable() {
    use vectorcraft_geom::{Anchor, SubPath};
    let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    // Background rectangle covering 0..300 with default white fill
    let bg = d.alloc_id();
    let bg_rect = vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 300.0, 300.0));
    d.insert(Some(l), 0, vectorcraft_doc::Node::path(bg, bg_rect, vectorcraft_doc::Appearance::default_art())).unwrap();

    // Selected path with a Bézier curve whose out-handle is at (150, 150)
    let fg = d.alloc_id();
    let mut a0 = Anchor::corner(Point::new(100.0, 150.0));
    a0.h_out = Point::new(150.0, 150.0); // handle over the background rect
    let a1 = Anchor::corner(Point::new(200.0, 150.0));
    let curve = PathData::single(SubPath::new(vec![a0, a1], false));
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(fg, curve, vectorcraft_doc::Appearance::default_art())).unwrap();

    let mut s = Selection::default();
    s.set([fg]);
    s.anchors.insert(fg, [(0, 0)].into());
    let p = paint();
    let c = cx(&d, &s, &p);

    // hit_handle on the handle at (150, 150) must find the handle of `fg`
    assert_eq!(hit_handle(&c, Point::new(150.0, 150.0), c.point_tol()), Some((fg, 0, 0, true)));

    // Direct Selection tool clicking on the handle initiates Reshape
    let mut t = DirectSelectionTool::new(false);
    let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
    assert_eq!(acts, vec![Action::Begin("Reshape".into())]);
}
