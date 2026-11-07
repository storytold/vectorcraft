use super::*;
use crate::testutil::*;
use crate::{Mods, PointerKind};
use vectorcraft_doc::{Appearance, Node};

/// A live 100 × 100 rectangle at (100, 100) with corner radius `r`.
fn live_rect(r: f64, xf: Affine) -> (Document, NodeId) {
    live_shape(100.0, 100.0, r, xf)
}

/// A live `w` × `h` rectangle with corner radius `r`.
fn live_shape(w: f64, h: f64, r: f64, xf: Affine) -> (Document, NodeId) {
    let live = LiveShape::Rectangle { w, h, radii: [r; 4], kinds: Default::default(), radii_relative: false, xf };
    doc_with(live.to_path(), Some(live))
}

/// A document holding one path with its live shape.
fn doc_with(path: PathData, live: Option<LiveShape>) -> (Document, NodeId) {
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let mut n = Node::path(id, path, Appearance::default_art());
    if let NodeKind::Path { live: slot, .. } = &mut n.kind {
        *slot = live.map(Box::new);
    }
    d.insert(Some(l), 0, n).unwrap();
    (d, id)
}

/// A five-pointed star at (200, 200) of radii 60 and 30 (#511): a plain path.
fn star() -> (Document, NodeId) {
    doc_with(vectorcraft_geom::shapes::star(Point::new(200.0, 200.0), 60.0, 30.0, 5, 0.0), None)
}

fn selected(id: NodeId) -> Selection {
    let mut s = Selection::default();
    s.add(id);
    s
}

fn pts(w: &CornerWidgets) -> Vec<Point> {
    w.visible().collect()
}

fn radius(a: &Action) -> f64 {
    let Action::Preview(c, v) = a else { panic!("not a preview: {a:?}") };
    assert_eq!(c, "object.setLiveShape");
    v["radius"].as_f64().unwrap()
}

#[test]
fn widgets_sit_inside_each_corner() {
    let (d, id) = live_rect(0.0, Affine::translate((100.0, 100.0)));
    let w = CornerWidgets::of(&d, &selected(id), 1.0, false).unwrap();
    assert_eq!(pts(&w), [Point::new(110.0, 110.0), Point::new(190.0, 110.0), Point::new(190.0, 190.0), Point::new(110.0, 190.0)]);
    // A larger radius moves them to the arc centres; zooming in keeps a 10 px minimum.
    let (d, id) = live_rect(20.0, Affine::translate((100.0, 100.0)));
    assert_eq!(pts(&CornerWidgets::of(&d, &selected(id), 1.0, false).unwrap())[0], Point::new(120.0, 120.0));
    assert_eq!(pts(&CornerWidgets::of(&d, &selected(id), 4.0, false).unwrap())[2], Point::new(180.0, 180.0));
    // Hidden when the shape is tiny on screen and without a single selection; a plain path
    // shows them with Direct Selection only.
    assert!(CornerWidgets::of(&d, &selected(id), 0.2, true).is_none());
    let (d, id) = doc_with_rect();
    assert!(CornerWidgets::of(&d, &selected(id), 1.0, false).is_none());
    assert_eq!(pts(&CornerWidgets::of(&d, &selected(id), 1.0, true).unwrap())[0], Point::new(110.0, 110.0));
    assert!(CornerWidgets::of(&d, &Selection::default(), 1.0, true).is_none());
}

#[test]
fn dragging_a_widget_inward_rounds_all_corners_in_one_step() {
    let (d, id) = live_rect(0.0, Affine::translate((100.0, 100.0)));
    let s = selected(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    for mut t in [crate::create("selection"), crate::create("directSelection")] {
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 190.0, 191.0)).is_empty());
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 175.0, 176.0));
        assert_eq!(a[0], Action::Begin("Corner Radius".into()));
        assert!((radius(&a[1]) - 15.0).abs() < 1e-9);
        assert!(matches!(&a[1], Action::Preview(_, v) if v["id"] == id.0));
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == "Radius: 15.00 pt")));
        let quiet = ToolContext { measurement_labels: false, ..crate::testutil::cx(&d, &s, &p) };
        assert!(!t.overlays(&quiet).iter().any(|o| matches!(o, Overlay::Measure { .. })));
        // Past the middle the corner is fully round; back outside it is square.
        assert_eq!(radius(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 100.0, 100.0))[0]), 50.0);
        assert_eq!(radius(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 260.0, 260.0))[0]), 0.0);
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 260.0, 260.0)), vec![Action::Commit]);
    }
}

/// #938: with two rectangles selected, both show their widgets, and a drag on either rounds
/// the corners of both in one step; with Direct Selection, each its shown corners.
#[test]
fn widgets_of_several_selected_paths_drag_together() {
    let (mut d, a) = live_rect(0.0, Affine::translate((100.0, 100.0)));
    let live = LiveShape::Rectangle {
        w: 100.0,
        h: 100.0,
        radii: [0.0; 4],
        kinds: Default::default(),
        radii_relative: false,
        xf: Affine::translate((300.0, 100.0)),
    };
    let b = d.alloc_id();
    let mut n = Node::path(b, live.to_path(), Appearance::default_art());
    if let NodeKind::Path { live: slot, .. } = &mut n.kind {
        *slot = Some(Box::new(live));
    }
    let l = d.layers[0].id;
    d.insert(Some(l), 1, n).unwrap();
    let mut s = selected(a);
    s.add(b);
    assert_eq!(pts(&CornerWidgets::of(&d, &s, 1.0, false).unwrap()).len(), 8);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = crate::create("selection");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 390.0, 191.0));
    let preview = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 375.0, 176.0));
    assert!((radius(&preview[1]) - 15.0).abs() < 1e-9);
    let Action::Preview(_, v) = &preview[1] else { panic!() };
    assert_eq!(v["items"], json!([{"id": a.0}, {"id": b.0}]));
    // Direct Selection with one anchor of the first: its corner, and the whole second.
    s.anchors.insert(a, [(0, 2)].into());
    let w = CornerWidgets::of(&d, &s, 1.0, true).unwrap();
    assert_eq!(w.corners(), [2, 0, 1, 2, 3]);
    assert_eq!(w.command("radius", json!(5.0)), json!({"items": [{"id": a.0, "corners": [2]}, {"id": b.0}], "radius": 5.0}));
}

/// #442: on a 200 × 80 rectangle the widgets sit at the centres of the corners as drawn, also
/// with a radius past half the shorter side; a drag past fully round stops every corner there
/// and outlines them in red.
#[test]
fn widgets_of_a_non_square_rectangle_follow_the_drawn_corners() {
    let (d, id) = live_shape(200.0, 80.0, 60.0, Affine::translate((100.0, 100.0)));
    let s = selected(id);
    let w = CornerWidgets::of(&d, &s, 1.0, false).unwrap();
    let centres = [Point::new(140.0, 140.0), Point::new(260.0, 140.0), Point::new(260.0, 140.0), Point::new(140.0, 140.0)];
    assert_eq!(pts(&w), centres);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let red = |t: &dyn crate::Tool| {
        t.overlays(&cx).into_iter().find_map(|o| match o {
            Overlay::Path { path, color, .. } if color == crate::builder::HIGHLIGHT_RED => Some(path),
            _ => None,
        })
    };
    let mut t = crate::create("selection");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 140.0, 140.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 200.0));
    assert_eq!(radius(&a[1]), 40.0);
    // The four arcs, each a quarter circle of radius 40 starting on a side.
    let arcs = red(t.as_ref()).expect("the limit shows");
    let starts: Vec<_> = arcs.elements().iter().filter_map(|e| if let vectorcraft_geom::PathEl::MoveTo(p) = e { Some(*p) } else { None }).collect();
    assert_eq!(starts, [Point::new(260.0, 100.0), Point::new(300.0, 140.0), Point::new(140.0, 180.0), Point::new(100.0, 140.0)]);
    // Back below the limit, the outline goes.
    assert_eq!(radius(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 130.0, 130.0))[0]), 30.0);
    assert!(red(t.as_ref()).is_none());
}

/// #442: a rectangle saved stretched (an uneven scale in its transform, before #291) shows its
/// widgets and drags its radius in document units, as `object.setLiveShape` sets it.
#[test]
fn a_stretched_rectangle_drags_its_radius_in_document_units() {
    // 50 × 80 with 10 pt corners stretched 4 × 1: drawn 200 × 80, a mean radius of 20.
    let (d, id) = live_shape(50.0, 80.0, 10.0, Affine::translate((100.0, 100.0)) * Affine::scale_non_uniform(4.0, 1.0));
    let s = selected(id);
    let w = CornerWidgets::of(&d, &s, 1.0, false).unwrap();
    assert_eq!(pts(&w)[0], Point::new(120.0, 120.0));
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = crate::create("directSelection");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 120.0, 120.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 110.0, 110.0));
    assert!((radius(&a[1]) - 10.0).abs() < 1e-9, "{a:?}");
}

#[test]
fn drag_follows_a_rotated_shape_and_a_click_does_nothing() {
    // Rotated 90°: the shape's top-left corner is at document (200, 100).
    let xf = Affine::translate((200.0, 100.0)) * Affine::rotate(std::f64::consts::FRAC_PI_2);
    let (d, id) = live_rect(10.0, xf);
    let s = selected(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let w = CornerWidgets::of(&d, &s, 1.0, false).unwrap();
    assert!(pts(&w)[0].distance(Point::new(190.0, 110.0)) < 1e-9);
    let mut t = crate::create("directSelection");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 190.0, 110.0));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 180.0, 120.0));
    assert!((radius(&a[1]) - 20.0).abs() < 1e-9);
    // A click without a drag leaves the shape (and the undo history) alone.
    let mut t = crate::create("selection");
    assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 190.0, 110.0)).is_empty());
    assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 190.0, 110.0)).is_empty());
}

/// Direct Selection: a click on a widget (no drag) selects its corner, as a click on the
/// corner's anchor does, so the next drag on that widget rounds that corner alone; Shift
/// toggles the corner. The Selection tool's click still does nothing.
#[test]
fn a_click_on_a_widget_with_direct_selection_selects_its_corner() {
    let (d, id) = live_rect(10.0, Affine::translate((100.0, 100.0)));
    let s = selected(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let w = pts(&CornerWidgets::of(&d, &s, 1.0, true).unwrap())[2];
    // The bottom-right corner of a rounded rectangle is drawn with two anchors.
    let anchors: Vec<(usize, usize)> = LiveCorners::of(d.node(id).unwrap()).unwrap().anchors_of(&[2].into()).into_iter().collect();
    assert_eq!(anchors.len(), 2);
    let mut t = crate::create("directSelection");
    assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, w.x, w.y)).is_empty());
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, w.x, w.y));
    assert_eq!(a, vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": anchors, "mode": "set"}))]);
    let shift = Mods { shift: true, ..Mods::default() };
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, w.x, w.y).with_mods(shift));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, w.x, w.y).with_mods(shift));
    assert_eq!(a, vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": anchors, "mode": "toggle"}))]);
    // With that corner selected, its widget alone shows, and a drag rounds it alone
    // (`direct_selected_corners_show_their_widgets_alone_and_round_alone`).
    let mut part = selected(id);
    part.anchors.insert(id, anchors.iter().copied().collect());
    assert_eq!(CornerWidgets::of(&d, &part, 1.0, true).unwrap().corners(), [2]);
    let mut t = crate::create("selection");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, w.x, w.y));
    assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, w.x, w.y)).is_empty());
}

#[test]
fn direct_selected_corners_show_their_widgets_alone_and_round_alone() {
    let (d, id) = live_rect(0.0, Affine::translate((100.0, 100.0)));
    // Direct Selection picked the bottom-right anchor.
    let mut s = selected(id);
    s.anchors.insert(id, [(0, 2)].into());
    let p = paint();
    let cx = cx(&d, &s, &p);
    let w = CornerWidgets::of(&d, &s, 1.0, true).unwrap();
    assert_eq!(w.corners(), [2]);
    assert_eq!(pts(&w), [Point::new(190.0, 190.0)]);
    assert!(!over_widget(&cx, Point::new(110.0, 110.0), true), "the other corners hide theirs");
    let mut t = crate::create("directSelection");
    assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 190.0, 191.0)).is_empty());
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 175.0, 176.0));
    assert_eq!(a[0], Action::Begin("Corner Radius".into()));
    assert_eq!(a[1], Action::Preview("object.setLiveShape".into(), json!({"id": id.0, "radius": 15.0, "corners": [2]})));
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 175.0, 176.0)), vec![Action::Commit]);
}

/// #2332: one corner rounded alone goes past half the side, until its cut reaches the
/// neighbouring anchors (the whole side of a square), or a rounded neighbour's cut.
#[test]
fn a_corner_rounded_alone_rounds_up_to_its_neighbours() {
    for (top_right, max) in [(0.0, 100.0), (30.0, 70.0)] {
        let live = LiveShape::Rectangle {
            w: 100.0,
            h: 100.0,
            radii: [0.0, top_right, 0.0, 0.0],
            kinds: Default::default(),
            radii_relative: false,
            xf: Affine::translate((100.0, 100.0)),
        };
        let (d, id) = doc_with(live.to_path(), Some(live));
        // Direct Selection picked the bottom-right anchor.
        let mut s = selected(id);
        s.anchors.insert(id, [(0, if top_right > 0.0 { 3 } else { 2 })].into());
        let p = paint();
        let cx = cx(&d, &s, &p);
        assert_eq!(CornerWidgets::of(&d, &s, 1.0, true).unwrap().corners(), [2]);
        let mut t = crate::create("directSelection");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 190.0, 190.0));
        assert!((radius(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 130.0, 130.0))[1]) - 60.0).abs() < 1e-9);
        assert!(!t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Path { .. })), "60 is below the limit");
        assert_eq!(radius(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 0.0, 0.0))[0]), max);
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Path { .. })), "the limit shows");
    }
}

#[test]
fn alt_click_cycles_the_kind_and_double_click_opens_the_dialog() {
    let (d, id) = live_rect(10.0, Affine::translate((100.0, 100.0)));
    let p = paint();
    let alt = Mods { alt: true, ..Mods::default() };
    let whole = selected(id);
    let mut part = selected(id);
    part.anchors.insert(id, [(0, 0), (0, 7)].into());
    for (s, extra) in [(&whole, json!({})), (&part, json!({"corners": [0]}))] {
        let cx = cx(&d, s, &p);
        let mut want = json!({"id": id.0, "kind": "invertedRound"});
        want.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        for tool in ["selection", "directSelection"] {
            let mut t = crate::create(tool);
            assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 110.0, 110.0).with_mods(alt)).is_empty());
            let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 110.0, 110.0).with_mods(alt));
            assert_eq!(a, vec![Action::Exec("object.setLiveShape".into(), want.clone())], "{tool}");
        }
    }
    // A double-click on a widget opens Corners for the shown corners; elsewhere it doesn't.
    let cx1 = cx(&d, &part, &p);
    let a = crate::create("directSelection").pointer(&cx1, &PointerEvent::new(PointerKind::DoubleClick, 110.0, 110.0));
    assert_eq!(a, vec![Action::Dialog(DIALOG.into(), json!({"id": id.0, "corners": [0]}))]);
    let cx2 = cx(&d, &whole, &p);
    let a = crate::create("selection").pointer(&cx2, &PointerEvent::new(PointerKind::DoubleClick, 190.0, 110.0));
    assert_eq!(a, vec![Action::Dialog(DIALOG.into(), json!({"id": id.0, "corners": [0, 1, 2, 3]}))]);
    // Off the widgets, the Selection tool's double-click isolates the shape (Double Click To
    // Isolate) instead.
    let a = crate::create("selection").pointer(&cx2, &PointerEvent::new(PointerKind::DoubleClick, 150.0, 150.0));
    assert_eq!(a, vec![Action::Exec("object.isolate".into(), json!({"id": id.0}))]);
}

/// Hide Corner Widget for angles greater than (#394): a rectangle's right angles hide below
/// 90°; sheared, only its acute corners keep their widgets.
#[test]
fn corners_wider_than_the_preference_hide_their_widgets() {
    let (d, id) = live_rect(0.0, Affine::translate((100.0, 100.0)));
    let w = CornerWidgets::of(&d, &selected(id), 1.0, false).unwrap();
    assert_eq!(w.clone().within_angle(177.0).map(|w| w.corners()), Some(vec![0, 1, 2, 3]));
    assert_eq!(w.clone().within_angle(90.0).map(|w| w.corners()), Some(vec![0, 1, 2, 3]));
    assert!(w.within_angle(89.0).is_none());
    // Sheared by 30°: 60° at the top-left and bottom-right corners, 120° at the others.
    let shear = Affine::translate((100.0, 100.0)) * Affine::new([1.0, 0.0, 30f64.to_radians().tan(), 1.0, 0.0, 0.0]);
    let (d, id) = live_rect(0.0, shear);
    let w = CornerWidgets::of(&d, &selected(id), 1.0, false).unwrap();
    assert_eq!(w.clone().within_angle(100.0).map(|w| w.corners()), Some(vec![0, 2]));
    let s = selected(id);
    let p = paint();
    let c = ToolContext { corner_widget_max_angle: 100.0, ..cx(&d, &s, &p) };
    assert!(over_widget(&c, pts(&w)[0], false));
    assert!(!over_widget(&c, pts(&w)[1], false), "the 120° corner's widget is hidden");
}

#[test]
fn widgets_off_or_elsewhere_keep_the_tools_behaviour() {
    let (d, id) = live_rect(0.0, Affine::translate((100.0, 100.0)));
    let s = selected(id);
    let p = paint();
    let mut c = cx(&d, &s, &p);
    assert!(over_widget(&c, Point::new(110.0, 110.0), false));
    assert!(!over_widget(&c, Point::new(150.0, 150.0), false));
    c.corner_widgets = false;
    assert!(!over_widget(&c, Point::new(110.0, 110.0), true));
    // With the widgets hidden, a drag from the same spot moves the object.
    let mut t = crate::create("selection");
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, 110.0, 110.0));
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 130.0, 110.0));
    assert_eq!(a[0], Action::Begin("Move".into()));
}

/// #511: Direct Selection shows a widget in each of a star's ten corners (the Selection tool
/// none: a star isn't a live shape); dragging a tip's inward rounds them all as one step.
#[test]
fn a_star_shows_widgets_in_every_corner_with_direct_selection() {
    let (d, id) = star();
    let s = selected(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    assert!(CornerWidgets::for_tool(&cx, false).is_none());
    let w = CornerWidgets::for_tool(&cx, true).expect("widgets on a plain path");
    assert_eq!(w.corners(), (0..10).collect::<Vec<_>>());
    // The top tip's widget sits 10 √2 px down its bisector.
    let tip = pts(&w)[0];
    assert!(tip.distance(Point::new(200.0, 140.0 + 10.0 * 2f64.sqrt())) < 1e-9, "{tip:?}");
    let mut t = crate::create("directSelection");
    assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, tip.x, tip.y)).is_empty());
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, tip.x, tip.y + 10.0));
    assert_eq!(a[0], Action::Begin("Corner Radius".into()));
    // The widget follows the arc's centre: r / sin(angle / 2) along the bisector.
    let c = vectorcraft_geom::corners::path_corners(&star_path(&d, id))[0];
    let want = 10.0 * (c.angle_deg() / 2.0).to_radians().sin();
    assert!((radius(&a[1]) - want).abs() < 1e-9, "{a:?} want {want}");
    assert!(matches!(&a[1], Action::Preview(_, v) if v["id"] == id.0 && v.get("corners").is_none()), "every corner: {a:?}");
    // The Selection tool drags the star instead.
    let mut t = crate::create("selection");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, tip.x, tip.y));
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, tip.x, tip.y + 10.0))[0], Action::Begin("Move".into()));
}

fn star_path(d: &Document, id: NodeId) -> PathData {
    let Some(NodeKind::Path { path, .. }) = d.node(id).map(|n| &n.kind) else { panic!("not a path") };
    path.clone()
}

/// #511: a Direct-Selected anchor of the star shows its corner's widget alone, and the
/// preference hides the star's wide inner corners (about 125°) below their angle.
#[test]
fn star_corners_follow_the_anchors_and_the_angle_preference() {
    let (d, id) = star();
    let mut s = selected(id);
    s.anchors.insert(id, [(0, 3)].into());
    let p = paint();
    let c = cx(&d, &s, &p);
    let w = CornerWidgets::for_tool(&c, true).unwrap();
    assert_eq!(w.corners(), [3]);
    let mut t = crate::create("directSelection");
    let at = pts(&w)[0];
    t.pointer(&c, &PointerEvent::new(PointerKind::Down, at.x, at.y));
    let a = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, at.x + 20.0, at.y + 20.0));
    assert!(matches!(&a[1], Action::Preview(_, v) if v["corners"] == json!([3])), "{a:?}");
    // Corners wider than 90° hide: the five tips keep theirs.
    let whole = selected(id);
    let c = ToolContext { corner_widget_max_angle: 90.0, ..cx(&d, &whole, &p) };
    let w = CornerWidgets::for_tool(&c, true).unwrap();
    assert_eq!(w.corners(), [0, 2, 4, 6, 8]);
    assert_eq!(w.command("radius", json!(4.0))["corners"], json!([0, 2, 4, 6, 8]));
}

/// A live polygon shows its widgets to the Selection tool too; past every corner's limit the
/// cuts outline in red.
#[test]
fn a_live_polygon_rounds_with_either_tool() {
    let live =
        LiveShape::Polygon { radius: 80.0, sides: 6, xf: Affine::translate((200.0, 200.0)), radii: vec![], kinds: vec![], radii_relative: false };
    let (d, id) = doc_with(live.to_path(), Some(live));
    let s = selected(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    for any_path in [false, true] {
        assert_eq!(CornerWidgets::for_tool(&cx, any_path).unwrap().corners().len(), 6);
    }
    let tip = pts(&CornerWidgets::for_tool(&cx, false).unwrap())[0];
    let mut t = crate::create("selection");
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, tip.x, tip.y));
    let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 300.0));
    // A hexagon's 120° corners stop where their cuts meet: 40 · tan 60°.
    assert!((radius(&a[1]) - 40.0 * 3f64.sqrt()).abs() < 1e-9, "{a:?}");
    let red = t
        .overlays(&cx)
        .into_iter()
        .any(|o| matches!(o, Overlay::Path { color, path, .. } if color == crate::builder::HIGHLIGHT_RED && path.elements().len() == 12));
    assert!(red, "six arcs at the limit");
}

/// A pen path's corners show widgets, curved ones too (#1144); its ends don't.
#[test]
fn a_pen_path_shows_widgets_on_its_corners_only() {
    let mut sp = vectorcraft_geom::SubPath::polyline(
        &[Point::new(100.0, 100.0), Point::new(300.0, 100.0), Point::new(300.0, 300.0), Point::new(100.0, 300.0)],
        false,
    );
    sp.anchors[2].h_out = Point::new(250.0, 350.0);
    let (d, id) = doc_with(PathData::single(sp), None);
    let s = selected(id);
    let p = paint();
    let w = CornerWidgets::for_tool(&cx(&d, &s, &p), true).unwrap();
    assert_eq!(w.corners(), [1, 2]);
    assert_eq!(pts(&w)[0], Point::new(290.0, 110.0));
}
