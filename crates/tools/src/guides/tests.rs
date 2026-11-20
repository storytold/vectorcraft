use super::*;
use crate::testutil::*;

#[test]
fn artboard_bleed_is_a_target() {
    let (mut d, _) = doc_with_rect();
    let t = Targets::collect(&d, &[], None);
    assert_eq!(t.snap_point(Point::new(512.0, 300.0), 4.0).0.x, 512.0, "no bleed, nothing near");
    d.setup.bleed = [10.0; 4];
    let t = Targets::collect(&d, &[], None);
    assert_eq!(t.snap_point(Point::new(512.0, 300.0), 4.0).0.x, 510.0);
}

#[test]
fn snaps_to_anchor_and_alignment() {
    let (d, _) = doc_with_rect();
    let t = Targets::collect(&d, &[], None);
    let (p, ov) = t.snap_point(Point::new(102.0, 99.0), 4.0);
    assert_eq!(p, Point::new(100.0, 100.0));
    assert!(matches!(&ov[0], Overlay::Label { text, .. } if text == "anchor"));
    let (p, _) = t.snap_point(Point::new(301.0, 199.0), 4.0);
    assert_eq!(p.y, 200.0);
}

/// #4312: a Shift move slides the selection along its line onto a ruler guide, but not far to
/// meet one a nearly parallel line only reaches a long way off.
#[test]
fn a_slide_along_a_line_lands_the_bounds_on_a_ruler_guide_within_reach() {
    let t = Targets { rulers: vec![Ruler { vertical: true, pos: 300.0, span: None }], ..Targets::default() };
    let r = Rect::new(248.0, 10.0, 298.0, 40.0);
    let p = Point::new(270.0, 25.0);
    // Level: the right edge is 2 short of the guide, which is the way along the line.
    assert_eq!(t.snap_moved_along_to_rulers(p, r, Vec2::new(1.0, 0.0), 4.0).0, Vec2::new(2.0, 0.0));
    // On the diagonal it is 2.8 along.
    let (d, ov) = t.snap_moved_along_to_rulers(p, r, Vec2::new(1.0, 1.0), 4.0);
    assert!((d - Vec2::new(2.0, 2.0)).hypot() < 1e-9 && !ov.is_empty(), "{d:?}");
    // Nearly upright, it would take about 118 along the line to meet the guide.
    assert_eq!(t.snap_moved_along_to_rulers(p, r, Vec2::new(0.017, 1.0), 4.0).0, Vec2::ZERO);
    // A guide of an artboard needs the selection to run past it.
    let bounded = Targets { rulers: vec![Ruler { vertical: true, pos: 300.0, span: Some((100.0, 200.0)) }], ..Targets::default() };
    assert_eq!(bounded.snap_moved_along_to_rulers(p, r, Vec2::new(1.0, 0.0), 4.0).0, Vec2::ZERO);
}

#[test]
fn rect_snap_aligns_edges() {
    let (d, id) = doc_with_rect();
    let t = Targets::collect(&d, &[id], None);
    // Artboard is 0..500; a rect at x0=2 snaps to the artboard's left edge.
    let (dv, ov) = t.snap_rect(Rect::new(2.0, 50.0, 52.0, 90.0), 4.0);
    assert_eq!(dv.x, -2.0);
    assert!(!ov.is_empty());
}

#[test]
fn scale_snap_lands_the_moved_edges_on_targets() {
    use crate::bbox::scale_for_drag;
    let (d, _) = doc_with_rect();
    // The document's rect is 100..200: a box beside it is resized against it.
    let t = Targets::collect(&d, &[], None);
    let r = Rect::new(250.0, 100.0, 300.0, 150.0);
    let snap = |h: Handle, p: Point, shift: bool, alt: bool| {
        let (a, ov) = t.snap_scale(&OrientedBox::aligned(r), h, scale_for_drag(r, h, p, shift, alt), shift, alt, 4.0);
        (a.transform_rect_bbox(r), ov)
    };
    // The bottom edge lands on the neighbour's bottom: same height.
    let lines = |ov: &[Overlay]| ov.iter().filter(|o| matches!(o, Overlay::Line { .. })).count();
    let align = |ov: &[Overlay]| ov.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "align"));
    let (nr, ov) = snap(Handle::Bottom, Point::new(275.0, 197.0), false, false);
    assert_eq!(nr, Rect::new(250.0, 100.0, 300.0, 200.0));
    assert_eq!(lines(&ov), 1);
    assert!(align(&ov), "{ov:?}");
    // A corner snaps each axis on its own: y to the neighbour, x stays free.
    let (nr, ov) = snap(Handle::BottomRight, Point::new(330.0, 202.0), false, false);
    assert_eq!(nr, Rect::new(250.0, 100.0, 330.0, 200.0));
    assert_eq!(lines(&ov), 1);
    // Proportional: the snapped axis carries the other one.
    let (nr, _) = snap(Handle::BottomRight, Point::new(340.0, 198.0), true, false);
    assert_eq!(nr, Rect::new(250.0, 100.0, 350.0, 200.0));
    // From the centre the dragged edge still lands on the target.
    let (nr, _) = snap(Handle::Bottom, Point::new(275.0, 198.0), false, true);
    assert_eq!(nr, Rect::new(250.0, 50.0, 300.0, 200.0));
    // Out of reach, and the fixed edge's own target: untouched.
    let (nr, ov) = snap(Handle::Bottom, Point::new(275.0, 180.0), false, false);
    assert_eq!(nr, Rect::new(250.0, 100.0, 300.0, 180.0));
    assert!(ov.is_empty());
    let (nr, ov) = snap(Handle::Bottom, Point::new(275.0, 102.0), false, false);
    assert_eq!(nr, Rect::new(250.0, 100.0, 300.0, 102.0));
    assert!(ov.is_empty());
}

/// A resize handle lands on another object's anchor point (#482), not only in line with its
/// edges and centre, and says so; a side handle stops where its way passes the anchor.
#[test]
fn scale_snap_lands_the_handle_on_anchors() {
    use crate::bbox::scale_for_drag;
    use vectorcraft_doc::{Appearance, Node};
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    // A triangle whose apex (320, 400) lines up with none of the edges or centres on x.
    let tri =
        vectorcraft_geom::PathData::single(SubPath::polyline(&[Point::new(300.0, 300.0), Point::new(380.0, 300.0), Point::new(320.0, 400.0)], true));
    d.insert(Some(l), 1, Node::path(id, tri, Appearance::default_art())).unwrap();
    let t = Targets::collect(&d, &[], None);
    let snap = |r: Rect, h: Handle, p: Point, shift: bool| {
        let (a, ov) = t.snap_scale(&OrientedBox::aligned(r), h, scale_for_drag(r, h, p, shift, false), shift, false, 4.0);
        (a.transform_rect_bbox(r), ov)
    };
    let anchor = |ov: &[Overlay]| matches!(ov, [Overlay::Label { text, p, .. }] if text == "anchor" && *p == Point::new(320.0, 400.0));
    // A free corner takes the anchor's x and y.
    let (nr, ov) = snap(Rect::new(250.0, 350.0, 300.0, 390.0), Handle::BottomRight, Point::new(322.0, 398.0), false);
    assert_eq!(nr, Rect::new(250.0, 350.0, 320.0, 400.0));
    assert!(anchor(&ov), "{ov:?}");
    // The right side, at the apex's height: its x lands on the anchor.
    let (nr, ov) = snap(Rect::new(250.0, 380.0, 300.0, 420.0), Handle::Right, Point::new(318.0, 401.0), false);
    assert_eq!(nr, Rect::new(250.0, 380.0, 320.0, 420.0));
    assert!(anchor(&ov), "{ov:?}");
    // A proportional corner slides along its diagonal onto it.
    let (nr, ov) = snap(Rect::new(240.0, 320.0, 300.0, 380.0), Handle::BottomRight, Point::new(318.0, 398.0), true);
    assert_eq!(nr, Rect::new(240.0, 320.0, 320.0, 400.0));
    assert!(anchor(&ov), "{ov:?}");
}

/// Smart Guides › Spacing Guides (#394): a moved selection lands as far from its neighbour as
/// two other objects of its row are apart, or halfway between its two neighbours, and the
/// equal gaps are marked; up and down the same in a column. Off, it stays where it is dragged.
#[test]
fn spacing_guides_space_a_moved_selection_evenly() {
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    // A and B 20 apart in a row at y 300–340, C and D 30 apart in a column at x 400–440.
    for r in [
        Rect::new(20.0, 300.0, 60.0, 340.0),
        Rect::new(80.0, 300.0, 120.0, 340.0),
        Rect::new(400.0, 20.0, 440.0, 50.0),
        Rect::new(400.0, 80.0, 440.0, 110.0),
    ] {
        let id = d.alloc_id();
        d.insert(Some(l), 0, Node::path(id, vectorcraft_geom::shapes::rectangle(r), Default::default())).unwrap();
    }
    let (s, p) = (Selection::default(), paint());
    let lines = |ov: &[Overlay]| ov.iter().filter(|o| matches!(o, Overlay::Line { color, .. } if *color == MAGENTA)).count();
    let t = Targets::for_move(&cx(&d, &s, &p), &s.objects);
    // 21 after B: 20, as A and B, with both gaps marked (a line and two ticks each).
    let (dv, ov) = t.snap_rect(Rect::new(141.0, 305.0, 181.0, 345.0), 4.0);
    assert_eq!(dv, Vec2::new(-1.0, 0.0));
    assert_eq!(lines(&ov), 6, "{ov:?}");
    assert!(ov.iter().any(|o| matches!(o, Overlay::Line { a, b, .. } if (a.x, b.x) == (120.0, 140.0) && a.y == b.y)), "{ov:?}");
    assert!(ov.iter().any(|o| matches!(o, Overlay::Line { a, b, .. } if (a.x, b.x) == (60.0, 80.0) && a.y == b.y)), "{ov:?}");
    // Halfway between B and a box at 220–260: 50 either side.
    let mut wide = d.clone();
    let id = wide.alloc_id();
    wide.insert(Some(l), 0, Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(220.0, 300.0, 260.0, 340.0)), Default::default())).unwrap();
    let (dv, ov) = Targets::for_move(&cx(&wide, &s, &p), &s.objects).snap_rect(Rect::new(148.0, 305.0, 188.0, 345.0), 4.0);
    assert_eq!(dv, Vec2::new(2.0, 0.0));
    assert!(lines(&ov) >= 6, "{ov:?}");
    // A column: 28 under D, as C and D are 30 apart.
    let (dv, ov) = t.snap_rect(Rect::new(405.0, 138.0, 445.0, 168.0), 4.0);
    assert_eq!(dv, Vec2::new(0.0, 2.0));
    assert!(ov.iter().any(|o| matches!(o, Overlay::Line { a, b, .. } if (a.y, b.y) == (110.0, 140.0) && a.x == b.x)), "{ov:?}");
    // Above C, 32 over it: 30.
    let (dv, _) = t.snap_rect(Rect::new(405.0, -42.0, 445.0, -12.0), 4.0);
    assert_eq!(dv, Vec2::new(0.0, 2.0));
    // Off: nowhere to go, and no marks.
    let off = ToolContext { spacing_guides: false, ..cx(&d, &s, &p) };
    let (dv, ov) = Targets::for_move(&off, &s.objects).snap_rect(Rect::new(141.0, 305.0, 181.0, 345.0), 4.0);
    assert_eq!((dv, lines(&ov)), (Vec2::ZERO, 0));
}

/// Smart Guides › Spacing Guides (#394): resizing by a side handle snaps the moving edge to the
/// same gap as the neighbours in its row, with the gaps marked.
#[test]
fn spacing_guides_resize_snaps_the_moving_edge_to_equal_gaps() {
    use crate::bbox::scale_for_drag;
    use vectorcraft_doc::{Appearance, Node};
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    for r in [Rect::new(20.0, 300.0, 60.0, 340.0), Rect::new(80.0, 300.0, 120.0, 340.0)] {
        let id = d.alloc_id();
        d.insert(Some(l), 0, Node::path(id, vectorcraft_geom::shapes::rectangle(r), Appearance::default_art())).unwrap();
    }
    let r = Rect::new(141.0, 305.0, 181.0, 345.0);
    let id = d.alloc_id();
    d.insert(Some(l), 0, Node::path(id, vectorcraft_geom::shapes::rectangle(r), Appearance::default_art())).unwrap();
    let s = Selection { objects: vec![id], ..Default::default() };
    let t = Targets::for_scale(&cx(&d, &s, &paint()));
    let (a, ov) = t.snap_scale(
        &OrientedBox::aligned(r),
        Handle::Left,
        scale_for_drag(r, Handle::Left, Point::new(139.0, 325.0), false, false),
        false,
        false,
        4.0,
    );
    assert_eq!(a.transform_rect_bbox(r).x0, 140.0, "20 pt from the second square");
    let magenta = |o: &Overlay| matches!(o, Overlay::Line { color, .. } if *color == MAGENTA);
    assert!(ov.iter().any(magenta), "{ov:?}");
}

/// Preferences › Smart Guides › Display Options (#394): the colour, and Alignment Guides and
/// Anchor/Path Labels off, change what shows and never where a point lands.
#[test]
fn display_options_colour_and_hide_the_guides_but_keep_the_snap() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let green = [0x00, 0xff, 0x00];
    let c = ToolContext { smart_guide_color: green, ..cx(&d, &s, &p) };
    let t = Targets::collect(&d, &[], None).styled(&c);
    let (q, ov) = t.snap_point(Point::new(102.0, 99.0), 4.0);
    assert_eq!(q, Point::new(100.0, 100.0));
    assert!(matches!(&ov[0], Overlay::Label { text, color, .. } if text == "anchor" && *color == green), "{ov:?}");
    let (q, ov) = t.snap_point(Point::new(301.0, 199.0), 4.0);
    assert_eq!(q.y, 200.0);
    assert!(ov.iter().all(|o| matches!(o, Overlay::Line { color, .. } | Overlay::Label { color, .. } if *color == green)), "{ov:?}");
    assert!(ov.iter().any(|o| matches!(o, Overlay::Line { .. })) && ov.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "align")));
    // Alignment Guides off: the point still lines up and says so, without the line.
    let c = ToolContext { alignment_guides: false, ..cx(&d, &s, &p) };
    let t = Targets::collect(&d, &[], None).styled(&c);
    let (q, ov) = t.snap_point(Point::new(301.0, 199.0), 4.0);
    assert_eq!(q.y, 200.0);
    assert!(!ov.iter().any(|o| matches!(o, Overlay::Line { .. })), "{ov:?}");
    assert!(ov.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "align")), "{ov:?}");
    let (dv, ov) = t.snap_rect(Rect::new(2.0, 50.0, 52.0, 90.0), 4.0);
    assert_eq!((dv.x, ov.len()), (-2.0, 0));
    // A handle already in line moves nowhere, yet is still "align".
    let (q, ov) = t.snap_point_with(Point::new(400.0, 200.0), &[], 4.0);
    assert_eq!(q, Point::new(400.0, 200.0));
    assert!(matches!(&ov[..], [Overlay::Label { text, .. }] if text == "align"), "{ov:?}");
    // Anchor/Path Labels off: on the anchor, without saying so.
    let c = ToolContext { anchor_path_labels: false, ..cx(&d, &s, &p) };
    let t = Targets::collect(&d, &[], None).styled(&c);
    let (q, ov) = t.snap_point(Point::new(102.0, 99.0), 4.0);
    assert_eq!((q, ov.len()), (Point::new(100.0, 100.0), 0));
    let (q, ov) = t.snap_point(Point::new(301.0, 199.0), 4.0);
    assert_eq!(q.y, 200.0);
    assert!(ov.iter().all(|o| matches!(o, Overlay::Line { .. })) && !ov.is_empty(), "{ov:?}");
    assert_eq!(t.snap_guide(false, 199.0, 4.0), (200.0, vec![]));
}

/// Dragging an anchor of a path: its other anchors and the segments not touching it stay
/// targets, but neither it nor the path's bounds (which move) are.
#[test]
fn anchor_drag_targets_leave_out_what_moves() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.set([id]);
    s.anchors.insert(id, [(0, 0)].into_iter().collect());
    let t = Targets::for_anchor_drag(&d, &s);
    let anchors: Vec<Point> = t.points.iter().filter(|(_, k)| *k == Kind::Anchor).map(|(p, _)| *p).collect();
    assert_eq!(anchors, [Point::new(100.0, 200.0), Point::new(200.0, 100.0), Point::new(200.0, 200.0)], "by x");
    assert_eq!(t.segments.len(), 2);
    assert!(!t.points.iter().any(|(p, k)| *k == Kind::Center && *p == Point::new(150.0, 150.0)), "the rect's centre moves");
    assert_eq!(t.snap_point(Point::new(101.0, 99.0), 4.0).0, Point::new(100.0, 100.0), "in line with its neighbours, not on itself");
}

/// A Shaper Group's original shapes, which are not shown, are no targets: not their
/// anchors, edges or segments, not their bounds as an object a moved face spaces itself evenly
/// with, and not the anchors a transform tool's reference point snaps to on the selected group.
/// Selected, or under the pointer in Construction mode, an original shape still gives its anchors.
#[test]
fn shaper_sources_are_no_targets() {
    let (d, g, sources, face) = doc_with_shaper_group();
    let b = d.node(sources).and_then(|n| n.children()).unwrap()[1].id;
    // The top right corner of the second source square, which no shown art has.
    let corner = Point::new(150.0, 300.0);
    let gives_corner = |t: &Targets| {
        t.points.iter().any(|(p, _)| *p == corner)
            || t.xs.iter().chain(&t.ys).any(|(_, p, _)| *p == corner)
            || t.segments.iter().any(|(_, s)| s.start() == corner || s.end() == corner)
    };
    assert!(!gives_corner(&Targets::collect(&d, &[], None)));
    assert!(!gives_corner(&Targets::for_drawing(&d, &[], None, None)));
    assert!(!gives_corner(&Targets::for_anchor_drag(&d, &Selection::default())));
    assert!(!gives_corner(&Targets::for_handle(&d, face)));
    assert!(!object_boxes(&d, &[face]).contains(&Rect::new(0.0, 300.0, 150.0, 400.0)), "the source shapes' bounds");
    let near = Rect::new(146.0, 296.0, 154.0, 304.0);
    assert!(!gives_corner(&Targets::points_near(&d, &[g], near)));
    assert!(gives_corner(&Targets::points_near(&d, &[b], near)));
}

/// Art inside a hidden group or layer is no target, even while its own visibility is on: not
/// for drawing, the Knife's straight cut, a move or a transform tool's reference point on the
/// shown group around it.
#[test]
fn art_in_hidden_groups_and_layers_is_no_target() {
    use std::sync::Arc;
    use vectorcraft_doc::{Appearance, Node};
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let square = Node::path(d.alloc_id(), vectorcraft_geom::shapes::rectangle(Rect::new(300.0, 300.0, 400.0, 400.0)), Appearance::default_art());
    let mut inner = Node::group(d.alloc_id(), vec![Arc::new(square)]);
    inner.visible = false;
    let outer = Node::group(d.alloc_id(), vec![Arc::new(inner)]);
    let g = outer.id;
    d.insert(Some(l), 1, outer).unwrap();
    let corner = Point::new(300.0, 300.0);
    let gives = |t: &Targets, at: Point| {
        t.points.iter().any(|(p, _)| *p == at)
            || t.xs.iter().chain(&t.ys).any(|(_, p, _)| *p == at)
            || t.segments.iter().any(|(_, s)| s.start() == at || s.end() == at)
    };
    assert!(!gives(&Targets::collect(&d, &[], None), corner));
    assert!(!gives(&Targets::for_drawing(&d, &[], None, None), corner));
    assert!(!gives(&Targets::for_drawing(&d, &[], None, None).anchors_and_paths(), corner));
    assert!(!gives(&Targets::points_near(&d, &[g], Rect::new(296.0, 296.0, 304.0, 304.0)), corner));
    // The shown rectangle on the same layer still is one, until the layer is hidden.
    let shown = Point::new(100.0, 100.0);
    assert!(gives(&Targets::for_drawing(&d, &[], None, None).anchors_and_paths(), shown));
    d.node_mut(l).unwrap().visible = false;
    assert!(!gives(&Targets::collect(&d, &[], None), shown));
    assert!(!gives(&Targets::for_drawing(&d, &[], None, None).anchors_and_paths(), shown));
}

/// #2120: the dragged end of a straight segment at any angle lands on the segment's own
/// extension (Line Extension), past its other end or short of it, the guide drawn from that
/// end; with two anchors moving there is none to keep to.
#[test]
fn dragged_end_of_a_slanted_line_keeps_to_its_extension() {
    use vectorcraft_doc::{Appearance, Node};
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    // From (300, 400) up to (380, 340): along (4, -3), about 37° up.
    d.insert(
        Some(l),
        1,
        Node::path(id, vectorcraft_geom::shapes::line(Point::new(300.0, 400.0), Point::new(380.0, 340.0)), Appearance::default_art()),
    )
    .unwrap();
    let (mut s, p) = (Selection::default(), paint());
    s.set([id]);
    s.anchors.insert(id, [(0, 1)].into_iter().collect());
    let t = Targets::for_anchor_drag(&d, &s).styled(&cx(&d, &s, &p));
    // 150 along it (2.5 off), and 50 along it (shorter).
    for (pointer, on) in [(Point::new(421.5, 312.0), Point::new(420.0, 310.0)), (Point::new(341.5, 372.0), Point::new(340.0, 370.0))] {
        let (q, ov) = t.snap_point(pointer, 4.0);
        assert!((q - on).hypot() < 1e-9, "{q:?}");
        assert_eq!(lines(&ov), vec![(Point::new(300.0, 400.0), q)]);
    }
    // Two anchors moving, the middle one staying put: no telling which one the pointer drags,
    // so neither keeps to the extension of its segment.
    let id = d.alloc_id();
    let bent =
        vectorcraft_geom::PathData::single(SubPath::polyline(&[Point::new(250.0, 450.0), Point::new(300.0, 400.0), Point::new(380.0, 340.0)], false));
    d.insert(Some(l), 1, Node::path(id, bent, Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.set([id]);
    s.anchors.insert(id, [(0, 0), (0, 2)].into_iter().collect());
    let t = Targets::for_anchor_drag(&d, &s);
    assert_eq!(t.snap_point(Point::new(421.5, 312.0), 4.0), (Point::new(421.5, 312.0), vec![]));
}

/// A segment nearly level (0.3°) is still straightened by dragging its end into line with
/// the other one; along its own direction, farther from level, it keeps to its extension.
#[test]
fn a_nearly_level_segment_still_lines_up_with_its_other_end() {
    use vectorcraft_doc::{Appearance, Node};
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    d.insert(
        Some(l),
        1,
        Node::path(id, vectorcraft_geom::shapes::line(Point::new(100.0, 300.0), Point::new(300.0, 301.0)), Appearance::default_art()),
    )
    .unwrap();
    let (mut s, p) = (Selection::default(), paint());
    s.set([id]);
    s.anchors.insert(id, [(0, 1)].into_iter().collect());
    let t = Targets::for_anchor_drag(&d, &s).styled(&cx(&d, &s, &p));
    let (q, ov) = t.snap_point(Point::new(300.0, 300.0), 4.0);
    assert_eq!((q, labels(&ov)), (Point::new(300.0, 300.0), vec!["align"]));
    let (q, ov) = t.snap_point(Point::new(400.0, 301.5), 4.0);
    assert!((q - Point::new(400.0, 301.5)).hypot() < 1e-9, "{q:?}");
    assert_eq!(lines(&ov), vec![(Point::new(100.0, 300.0), q)]);
}

fn labels(ov: &[Overlay]) -> Vec<&str> {
    ov.iter().filter_map(|o| if let Overlay::Label { text, .. } = o { Some(text.as_str()) } else { None }).collect()
}

fn lines(ov: &[Overlay]) -> Vec<(Point, Point)> {
    ov.iter().filter_map(|o| if let Overlay::Line { a, b, .. } = o { Some((*a, *b)) } else { None }).collect()
}

/// A drawn point (#506): on another object's anchors, in line with them too (not only with its
/// bounds), onto its paths; Shift slides it along its 45° step into line; near a construction
/// guide through the point it leaves from it lands on the guide, where a target's line crosses
/// it if one does nearby.
#[test]
fn drawn_points_land_on_anchors_paths_alignments_and_construction_guides() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let t = Targets::for_drawing(&d, &[], None, None).styled(&c);
    // The rect is 100..200: its corner, a point on its side, and in line with an anchor.
    let (q, ov) = t.snap_from(Point::new(202.0, 198.0), None, 4.0);
    assert_eq!((q, labels(&ov)), (Point::new(200.0, 200.0), vec!["anchor"]));
    let (q, ov) = t.snap_from(Point::new(202.0, 160.0), None, 4.0);
    assert_eq!((q, labels(&ov)), (Point::new(200.0, 160.0), vec!["path"]));
    let (q, ov) = t.snap_from(Point::new(330.0, 102.0), None, 4.0);
    assert_eq!((q, labels(&ov)), (Point::new(330.0, 100.0), vec!["align"]));
    // Shift from (300, 300): the pointer at 2° keeps to 0°, then slides onto x = 200.
    let shift = Leave::segment(&c, Point::new(300.0, 300.0), true);
    let (q, ov) = t.snap_from(Point::new(203.0, 303.0), Some(&shift), 4.0);
    assert!((q - Point::new(200.0, 300.0)).hypot() < 1e-9, "{q:?}");
    assert_eq!(labels(&ov), ["align"]);
    assert_eq!(lines(&ov).len(), 1, "{ov:?}");
    // Nothing in reach: on its angle, as far as the pointer.
    let (q, ov) = t.snap_from(Point::new(300.0, 380.0), Some(&shift), 4.0);
    assert_eq!((q, ov.len()), (Point::new(300.0, 380.0), 0));
    // Construction guides through (300, 320): onto the 135° one…
    let from = Leave::segment(&c, Point::new(300.0, 320.0), false);
    let (q, ov) = t.snap_from(Point::new(352.0, 373.0), Some(&from), 4.0);
    assert!((q - Point::new(352.5, 372.5)).hypot() < 1e-9, "{q:?}");
    assert_eq!(lines(&ov), vec![(Point::new(300.0, 320.0), q)]);
    // …where x = 200 crosses it.
    let (q, ov) = t.snap_from(Point::new(203.0, 222.5), Some(&from), 4.0);
    assert!((q - Point::new(200.0, 220.0)).hypot() < 1e-9, "{q:?}");
    assert_eq!((lines(&ov).len(), labels(&ov)), (2, vec!["align"]));
    // Construction Guides off (or Smart Guides off): in line, not on a guide.
    let off = ToolContext { construction_angles: &[], ..cx(&d, &s, &p) };
    let (q, _) = t.snap_from(Point::new(352.0, 373.0), Some(&Leave::segment(&off, Point::new(300.0, 320.0), false)), 4.0);
    assert_eq!(q, Point::new(352.0, 373.0));
    // A box's corner keeps to a diagonal with Shift, the farther axis giving its size.
    let (q, _) = t.snap_from(Point::new(340.0, 310.0), Some(&Leave::diagonal(Point::new(300.0, 300.0), true)), 4.0);
    assert_eq!(q, Point::new(340.0, 340.0));
}

/// #1602: a drawn point lands where two paths cross, where a path crosses a ruler guide and
/// where two ruler guides cross, labelled "intersect"; an anchor nearby still wins.
#[test]
fn drawn_points_land_on_intersections() {
    use vectorcraft_doc::{Appearance, Node};
    use vectorcraft_geom::shapes;
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    // A slanted line through the rect's top side (y = 100) at x = 150, and a circle across its
    // right side (x = 200).
    for path in [shapes::line(Point::new(145.0, 90.0), Point::new(165.0, 130.0)), shapes::ellipse(Rect::new(175.0, 130.0, 215.0, 170.0))] {
        let id = d.alloc_id();
        d.insert(Some(l), 1, Node::path(id, path, Appearance::default_art())).unwrap();
    }
    d.add_ruler_guide(&vectorcraft_doc::Guide::new(true, 400.0));
    d.add_ruler_guide(&vectorcraft_doc::Guide::new(false, 300.0));
    d.add_ruler_guide(&vectorcraft_doc::Guide::new(false, 150.0));
    let (s, p) = (Selection::default(), paint());
    let c = ToolContext { guides: true, ..cx(&d, &s, &p) };
    let t = Targets::for_drawing(&d, &[], None, None).styled(&c).with_rulers(&c);
    let (q, ov) = t.snap_from(Point::new(152.0, 101.5), None, 4.0);
    assert!((q - Point::new(150.0, 100.0)).hypot() < 1e-9, "{q:?}");
    assert_eq!(labels(&ov), ["intersect"]);
    // The circle (curves, centre (195, 150), radius 20) crosses the rect's right side.
    let (q, ov) = t.snap_from(Point::new(201.5, 131.5), None, 4.0);
    assert!((q - Point::new(200.0, 150.0 - 375f64.sqrt())).hypot() < 0.02, "{q:?}");
    assert_eq!(labels(&ov), ["intersect"]);
    // A ruler guide across a path, and two ruler guides.
    let (q, ov) = t.snap_from(Point::new(101.0, 151.0), None, 4.0);
    assert_eq!((q, labels(&ov)), (Point::new(100.0, 150.0), vec!["intersect"]));
    let (q, ov) = t.snap_from(Point::new(402.0, 298.0), None, 4.0);
    assert_eq!((q, labels(&ov)), (Point::new(400.0, 300.0), vec!["intersect"]));
    // An anchor in reach wins, and a path alone is still "path".
    assert_eq!(labels(&t.snap_from(Point::new(146.0, 91.0), None, 4.0).1), ["anchor"]);
    assert_eq!(labels(&t.snap_from(Point::new(170.0, 201.0), None, 4.0).1), ["path"]);
}

#[test]
fn perpendicular_snapping_rejects_a_curved_target() {
    let (mut d, _) = doc_with_rect();
    let layer = d.layers[0].id;
    let id = d.alloc_id();
    d.insert(
        Some(layer),
        1,
        vectorcraft_doc::Node::path(
            id,
            vectorcraft_geom::shapes::ellipse(Rect::new(450.0, 450.0, 550.0, 550.0)),
            vectorcraft_doc::Appearance::default_art(),
        ),
    )
    .unwrap();
    let targets = Targets { tangent: false, perpendicular: true, ..Targets::for_drawing(&d, &[], None, None) };
    assert!(targets.touching(Point::new(400.0, 500.0), Point::new(450.0, 500.0), 4.0).is_none());
    assert!(targets.touching(Point::new(300.0, 160.0), Point::new(200.0, 160.0), 4.0).is_some());
}

/// A line drawn from an anchor lands square to a segment (the foot of its perpendicular) and
/// just grazing a curve, with a guide and a label; the options switch each off, and the
/// result is the same each time and survives non-finite input.
#[test]
fn drawn_lines_land_perpendicular_to_segments_and_tangent_to_curves() {
    use vectorcraft_doc::{Appearance, Node};
    use vectorcraft_geom::shapes;
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    d.insert(Some(l), 1, Node::path(id, shapes::ellipse(Rect::new(450.0, 450.0, 550.0, 550.0)), Appearance::default_art())).unwrap();
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let snap = |c: &ToolContext, from: Point, to: Point| {
        let leave = Leave::segment(c, from, false);
        PointSnap::new(c, || Targets::for_drawing(&d, &[], None, None)).snap_from(c, to, Some(&leave))
    };
    // From (300, 160) the rect's right side (x = 200) is square at (200, 160).
    let (q, ov) = snap(&c, Point::new(300.0, 160.0), Point::new(202.0, 163.0));
    assert!((q - Point::new(200.0, 160.0)).hypot() < 1e-6, "{q:?}");
    assert_eq!(labels(&ov), ["perpendicular"]);
    assert_eq!(lines(&ov), vec![(Point::new(300.0, 160.0), q)]);
    assert!(ov.iter().any(|o| matches!(o, Overlay::Path { .. })), "the square corner mark: {ov:?}");
    assert_eq!(snap(&c, Point::new(300.0, 160.0), Point::new(202.0, 163.0)).0, q, "deterministic");
    // Off, the point only lands on the path, where the pointer is.
    let curved = Leave::segment(&c, Point::new(300.0, 160.0), false).without_contact();
    let (_, ov) = PointSnap::new(&c, || Targets::for_drawing(&d, &[], None, None)).snap_from(&c, Point::new(202.0, 163.0), Some(&curved));
    assert!(!labels(&ov).contains(&"perpendicular"));
    let off = ToolContext { snap_perpendicular: false, ..cx(&d, &s, &p) };
    let (q, ov) = snap(&off, Point::new(300.0, 160.0), Point::new(202.0, 163.0));
    assert_eq!((q, labels(&ov)), (Point::new(200.0, 163.0), vec!["path"]));
    // The circle (centre (500, 500), radius 50) seen from (400, 500): the tangent points are 60°
    // round from the way back, at (475, 456.7) and (475, 543.3).
    let (q, ov) = snap(&c, Point::new(400.0, 500.0), Point::new(477.0, 459.0));
    assert_eq!(labels(&ov), ["tangent"]);
    assert!((q - Point::new(475.0, 500.0 - 1875f64.sqrt())).hypot() < 0.1, "{q:?}");
    assert!(((q - Point::new(400.0, 500.0)).dot(q - Point::new(500.0, 500.0))).abs() < 20.0, "{q:?}");
    let none = ToolContext { snap_tangent: false, ..cx(&d, &s, &p) };
    assert!(!labels(&snap(&none, Point::new(400.0, 500.0), Point::new(477.0, 459.0)).1).contains(&"tangent"));
    // Out of reach, or with nothing finite to go on: left alone, no panic.
    assert_eq!(snap(&c, Point::new(300.0, 160.0), Point::new(250.0, 175.0)).0, Point::new(250.0, 175.0));
    let t = Targets::for_drawing(&d, &[], None, None);
    let t = Targets { tangent: true, perpendicular: true, ..t };
    for bad in [f64::NAN, f64::INFINITY] {
        assert!(t.touching(Point::new(bad, 1.0), Point::new(200.0, 160.0), 4.0).is_none());
        assert!(t.touching(Point::new(300.0, 160.0), Point::new(bad, 1.0), 4.0).is_none());
        assert!(t.touching(Point::new(300.0, 160.0), Point::new(200.0, 160.0), bad).is_none());
    }
}

/// A touch point that falls exactly on one of the samples `touch_params` takes (t = k / 24) is
/// found, once: the foot of the perpendicular from above a segment's middle, and the tangent
/// point of a line drawn from a free point to a circle (#7846). A line to an oval lands tangent
/// between samples, as before.
#[test]
fn touch_points_exactly_on_a_sample_are_found() {
    use super::snap::touch_params;
    use vectorcraft_doc::{Appearance, Node};
    use vectorcraft_geom::kurbo::{CubicBez, Line, ParamCurve};
    use vectorcraft_geom::shapes;
    // (50, 40) above the segment (0, 0) to (100, 0): the foot is (50, 0), at t = 0.5 = 12 / 24.
    let flat = PathSeg::Line(Line::new((0.0, 0.0), (100.0, 0.0)));
    assert_eq!(touch_params(&flat, Point::new(50.0, 40.0), true), [0.5]);
    assert_eq!(touch_params(&flat, Point::new(30.0, 40.0), true).len(), 1, "between samples, as before");
    assert!(touch_params(&flat, Point::new(0.0, 40.0), true).is_empty(), "the ends stay out");
    assert!(touch_params(&flat, Point::new(100.0, 40.0), true).is_empty(), "the ends stay out");
    // From a point on the segment itself there is no line to be square.
    assert!(touch_params(&flat, Point::new(50.0, 0.0), true).is_empty());
    // A straight cubic seen from its own line runs along that line everywhere: no touch point.
    let straight = PathSeg::Cubic(CubicBez::new((0.0, 0.0), (30.0, 0.0), (70.0, 0.0), (100.0, 0.0)));
    assert!(touch_params(&straight, Point::new(-50.0, 0.0), false).is_empty());
    // y = 400 (t - 0.5)^3 has its inflection at t = 0.5, and the tangent there (the x axis)
    // touches zero without a change of sign; seen from (-100, 0) it is still found.
    let wave = PathSeg::Cubic(CubicBez::new((0.0, -50.0), (100.0 / 3.0, 50.0), (200.0 / 3.0, -50.0), (100.0, 50.0)));
    assert_eq!(touch_params(&wave, Point::new(-100.0, 0.0), false), [0.5]);
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let circle = shapes::ellipse(Rect::new(450.0, 450.0, 550.0, 550.0));
    // The middle of the lower right quarter, and a point on the circle's tangent there.
    let quarter = PathSeg::Cubic(circle.subpaths[0].segment(2));
    let mid = quarter.eval(0.5);
    let from = mid + Vec2::new(-64.0, 64.0);
    assert!(((mid - Point::new(500.0, 500.0)).hypot() - 50.0).abs() < 0.1, "{mid:?}");
    assert_eq!(touch_params(&quarter, from, false), [0.5]);
    d.insert(Some(l), 1, Node::path(id, circle, Appearance::default_art())).unwrap();
    // An oval (centre (500, 300), 100 by 50) seen from (300, 300): the tangent points are at
    // x = 450, y = 300 -/+ 25 * sqrt(3), between samples.
    let oval = d.alloc_id();
    d.insert(Some(l), 2, Node::path(oval, shapes::ellipse(Rect::new(400.0, 250.0, 600.0, 350.0)), Appearance::default_art())).unwrap();
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let leave = Leave::segment(&c, from, false);
    let (q, ov) = PointSnap::new(&c, || Targets::for_drawing(&d, &[], None, None)).snap_from(&c, mid + Vec2::new(1.5, 1.0), Some(&leave));
    assert_eq!(labels(&ov), ["tangent"]);
    assert!((q - mid).hypot() < 1e-9, "{q:?} {mid:?}");
    assert_eq!(lines(&ov), vec![(from, q)]);
    let start = Point::new(300.0, 300.0);
    let leave = Leave::segment(&c, start, false);
    let (q, ov) = PointSnap::new(&c, || Targets::for_drawing(&d, &[], None, None)).snap_from(&c, Point::new(452.0, 259.0), Some(&leave));
    assert_eq!(labels(&ov), ["tangent"]);
    assert!((q - Point::new(450.0, 300.0 - 25.0 * 3f64.sqrt())).hypot() < 0.2, "{q:?}");
}

/// A box dragged from a corner (Rectangle, Ellipse, ...) never snaps square to a segment, and
/// a busy spot does not hide the contact: every segment in reach is tried.
#[test]
fn contact_snaps_only_for_segments_and_in_busy_spots() {
    use vectorcraft_geom::kurbo::Line;
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let snap = |t: &Targets, leave: &Leave| t.snap_from(Point::new(202.0, 163.0), Some(leave), 4.0);
    let t = Targets { tangent: true, perpendicular: true, ..Targets::for_drawing(&d, &[], None, None) };
    let (_, ov) = snap(&t, &Leave::segment(&c, Point::new(300.0, 160.0), false));
    assert_eq!(labels(&ov), ["perpendicular"]);
    let (_, ov) = snap(&t, &Leave::diagonal(Point::new(300.0, 160.0), false));
    assert!(!labels(&ov).contains(&"perpendicular"), "{ov:?}");
    // 70 short segments near the pointer, none square to the leave point, then the rect's side.
    let mut busy = t;
    let mut segments: Vec<(Rect, PathSeg)> = (0..70)
        .map(|i| {
            let y = 160.0 + f64::from(i) * 0.01;
            (Rect::new(199.0, y, 205.0, y + 0.001), PathSeg::Line(Line::new((199.0, y), (205.0, y + 0.001))))
        })
        .collect();
    segments.extend(busy.segments.iter().cloned());
    busy.segments = segments;
    let (_, ov) = snap(&busy, &Leave::segment(&c, Point::new(300.0, 160.0), false));
    assert_eq!(labels(&ov), ["perpendicular"]);
}

/// The angle sets of Preferences › Smart Guides › Construction Guides.
#[test]
fn construction_angle_sets() {
    assert_eq!(construction_angles(DEFAULT_CONSTRUCTION_ANGLES), [0.0, 45.0, 90.0, 135.0]);
    assert_eq!(construction_angles("90° Angles"), [0.0, 90.0]);
    assert_eq!(construction_angles("30° Angles"), [30.0, 60.0, 120.0, 150.0]);
    assert_eq!(construction_angles("90° & 45° & 30° Angles").len(), 8);
    assert_eq!(construction_angles("nonsense"), construction_angles(DEFAULT_CONSTRUCTION_ANGLES));
}

/// The sorted targets find by bisection just what a scan of all of them finds, whatever
/// gathered them.
#[test]
fn bisection_finds_what_a_scan_finds() {
    use vectorcraft_doc::{Appearance, Node};
    let (mut d, id) = doc_with_rect();
    let l = d.layers[0].id;
    for i in 0..60 {
        let (x, y) = ((i * 37 % 450) as f64, (i * 53 % 430) as f64);
        let tri = vectorcraft_geom::PathData::single(SubPath::polyline(
            &[Point::new(x, y), Point::new(x + 13.0, y + 3.0), Point::new(x + 5.0, y + 17.0)],
            true,
        ));
        let n = d.alloc_id();
        d.insert(Some(l), 1, Node::path(n, tri, Appearance::default_art())).unwrap();
    }
    let mut s = Selection::default();
    s.set([id]);
    s.anchors.insert(id, [(0, 1)].into_iter().collect());
    for t in [Targets::collect(&d, &[], None), Targets::for_drawing(&d, &[id], None, None), Targets::for_anchor_drag(&d, &s)] {
        for q in (0..40).flat_map(|i| (0..40).map(move |j| Point::new(i as f64 * 12.7, j as f64 * 11.3))) {
            let scan = t.points.iter().copied().filter(|(a, _)| a.distance(q) <= 6.0).min_by(|a, b| a.0.distance(q).total_cmp(&b.0.distance(q)));
            assert_eq!(t.nearest_point(q, 6.0).map(|h| h.0.distance(q)), scan.map(|h| h.0.distance(q)), "{q:?}");
            for (lines, v) in [(&t.xs, q.x), (&t.ys, q.y)] {
                let mut near: Vec<f64> = lines_near(lines, v, 6.0).iter().map(|l| l.0).collect();
                let mut all: Vec<f64> = lines.iter().map(|l| l.0).filter(|w| (w - v).abs() <= 6.0).collect();
                near.sort_by(f64::total_cmp);
                all.sort_by(f64::total_cmp);
                assert_eq!(near, all);
            }
        }
    }
}

/// #812: with Smart Guides on, ruler guides pull a drawn point too (Snap to Point), horizontal
/// and vertical alike, also while it slides along a construction guide or Shift's angle.
#[test]
fn drawn_points_snap_to_ruler_guides_with_smart_guides_on() {
    let (mut d, _) = doc_with_rect();
    d.add_ruler_guide(&vectorcraft_doc::Guide::new(false, 313.0));
    d.add_ruler_guide(&vectorcraft_doc::Guide::new(true, 331.0));
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let mut snap = DrawSnap::default();
    assert_eq!(snap.press(&c, Point::new(260.0, 315.0), &[], None), Point::new(260.0, 313.0), "the horizontal guide");
    assert_eq!(snap.press(&c, Point::new(329.0, 260.0), &[], None), Point::new(331.0, 260.0), "the vertical guide");
    // On the 45° construction guide from (210, 210), where the horizontal guide crosses it.
    let from = Leave::segment(&c, Point::new(210.0, 210.0), false);
    let q = snap.press(&c, Point::new(312.0, 311.0), &[], Some(&from));
    assert!((q - Point::new(313.0, 313.0)).hypot() < 1e-9, "{q:?}");
    // Snap to Point off: Smart Guides alone don't use the ruler guides.
    let off = ToolContext { snap_to_point: false, ..cx(&d, &s, &p) };
    assert_eq!(snap.press(&off, Point::new(260.0, 315.0), &[], None), Point::new(260.0, 315.0));
}

/// A drawing tool's snapper reuses its targets while hovering over the same document state, and
/// gathers them again when it changes, and on every press.
#[test]
fn draw_snap_gathers_once_per_document_state() {
    let (d, _) = doc_with_rect();
    let empty = vectorcraft_doc::Document::new(500.0, 500.0);
    let (s, p) = (Selection::default(), paint());
    let at = Point::new(202.0, 198.0);
    let mut snap = DrawSnap::default();
    let c = ToolContext { revision: (1, 1), ..cx(&d, &s, &p) };
    assert_eq!(snap.hover(&c, at, &[], None), Point::new(200.0, 200.0));
    assert_eq!(labels(snap.guides()), ["anchor"]);
    // The same revision: the targets are the ones gathered (the document isn't read again).
    let c = ToolContext { revision: (1, 1), ..cx(&empty, &s, &p) };
    assert_eq!(snap.hover(&c, at, &[], None), Point::new(200.0, 200.0));
    // A new revision, or a press, reads it.
    let c = ToolContext { revision: (1, 2), ..cx(&empty, &s, &p) };
    assert_eq!(snap.hover(&c, at, &[], None), at);
    assert!(snap.guides().is_empty());
    let c = ToolContext { revision: (1, 2), ..cx(&d, &s, &p) };
    assert_eq!(snap.press(&c, at, &[], None), Point::new(200.0, 200.0));
    // The drag keeps what the press gathered, though the document now shows its preview.
    let c = ToolContext { revision: (1, 3), ..cx(&empty, &s, &p) };
    assert_eq!(snap.drag(&c, at, None), Point::new(200.0, 200.0));
    snap.clear();
    assert!(snap.guides().is_empty());
    // Snap to Grid wins over Smart Guides, Shift still keeping the angle.
    let c = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
    let from = Leave::segment(&c, Point::ZERO, true);
    let q = snap.press(&c, Point::new(31.0, 2.0), &[], Some(&from));
    assert!((q.y - 0.0).abs() < 1e-9, "{q:?}");
}

/// A document holding point type `text` at (100, 300), 60 pt, turned `angle` radians about
/// where it starts, and the type.
fn glyph_doc(text: &str, angle: f64) -> (vectorcraft_doc::Document, vectorcraft_doc::TextObject) {
    glyph_doc_with(text, angle, vectorcraft_doc::CharStyle { size: 60.0, ..Default::default() })
}

/// [`glyph_doc`] with the type in `style`.
fn glyph_doc_with(text: &str, angle: f64, style: vectorcraft_doc::CharStyle) -> (vectorcraft_doc::Document, vectorcraft_doc::TextObject) {
    let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let at = Point::new(100.0, 300.0);
    let mut t = vectorcraft_doc::TextObject::point(at, text, style);
    t.xf = Affine::rotate_about(angle, at) * t.xf;
    d.insert(Some(l), 0, Node::new(id, NodeKind::Text(Box::new(t.clone())))).unwrap();
    (d, t)
}

/// Snap to Glyph: a Pen click near an anchor point of a glyph's outline lands on it, and not with
/// Snap to Glyph off.
#[test]
fn pen_click_near_a_glyph_anchor_lands_on_it() {
    use crate::{Action, PointerEvent, PointerKind, Tool};
    let (d, t) = glyph_doc("H", 0.0);
    let lay = layout_text(FontDb::global(), &t);
    let g = lay.glyphs.iter().find(|g| !g.outline.elements().is_empty()).expect("a glyph with an outline");
    // An anchor inside the glyph's bounds (a corner where the crossbar meets a stem), which no
    // glyph bounds line passes through.
    let bb = g.outline.bounding_box();
    let inside = |p: Point| p.x > bb.x0 + 1.0 && p.x < bb.x1 - 1.0 && p.y > bb.y0 + 1.0 && p.y < bb.y1 - 1.0;
    let anchor = t.xf * g.outline.segments().map(|s| s.start()).find(|p| inside(*p)).expect("an anchor inside the glyph");
    let (s, p) = (Selection::default(), paint());
    let click = anchor + Vec2::new(1.5, -1.0);
    let first_anchor = |glyphs: bool, anchors: bool| {
        let c = ToolContext { snap_to_glyph: glyphs, glyph_snap: GlyphSnap { anchors, ..GlyphSnap::default() }, ..cx(&d, &s, &p) };
        let mut pen = crate::pen::PenTool::default();
        let acts = pen.pointer(&c, &PointerEvent::new(PointerKind::Down, click.x, click.y));
        let Some(Action::Preview(_, v)) = acts.get(1) else { panic!("{acts:?}") };
        Point::new(v["anchors"][0]["x"].as_f64().unwrap(), v["anchors"][0]["y"].as_f64().unwrap())
    };
    assert!(first_anchor(true, true).distance(anchor) < 1e-9, "{:?} vs {anchor:?}", first_anchor(true, true));
    assert!(first_anchor(false, true).distance(anchor) > 1e-3);
    // The Character panel's Snap to Glyph › Anchor Point off: no glyph anchors (#4232).
    assert!(first_anchor(true, false).distance(anchor) > 1e-3);
    // Snap to Glyph works with Smart Guides on: Snap to Point alone does not land on glyph anchors.
    let c = ToolContext { snap_to_glyph: true, smart_guides: false, snap_to_point: true, ..cx(&d, &s, &p) };
    let mut pen = crate::pen::PenTool::default();
    let acts = pen.pointer(&c, &PointerEvent::new(PointerKind::Down, click.x, click.y));
    let Some(Action::Preview(_, v)) = acts.get(1) else { panic!("{acts:?}") };
    let q = Point::new(v["anchors"][0]["x"].as_f64().unwrap(), v["anchors"][0]["y"].as_f64().unwrap());
    assert!(q.distance(anchor) > 1e-3, "{q:?} landed on the glyph anchor with Smart Guides off");
}

/// Snap to Glyph: the Parametric Shape tool's center (a drag and a double-click) and the point
/// dragged out from it, and the Slice tool's corners, land on a glyph's anchor point as a Pen
/// click does, with the guides shown while hovering, and not with Snap to Glyph off.
#[test]
fn parametric_shape_and_slice_points_land_on_a_glyph_anchor() {
    use crate::addons::parametric_shape::ParametricShapeTool;
    use crate::slice::SliceTool;
    use crate::{Action, PointerEvent, PointerKind, Tool};
    let (d, t) = glyph_doc("H", 0.0);
    let lay = layout_text(FontDb::global(), &t);
    let g = lay.glyphs.iter().find(|g| !g.outline.elements().is_empty()).expect("a glyph with an outline");
    // An anchor inside the glyph's bounds, which no glyph bounds line passes through.
    let bb = g.outline.bounding_box();
    let inside = |p: Point| p.x > bb.x0 + 1.0 && p.x < bb.x1 - 1.0 && p.y > bb.y0 + 1.0 && p.y < bb.y1 - 1.0;
    let anchor = t.xf * g.outline.segments().map(|s| s.start()).find(|p| inside(*p)).expect("an anchor inside the glyph");
    let (s, p) = (Selection::default(), paint());
    let click = anchor + Vec2::new(1.5, -1.0);
    let ev = |kind, q: Point| PointerEvent::new(kind, q.x, q.y);
    let center = |v: &serde_json::Value| Point::new(v["center"][0].as_f64().unwrap(), v["center"][1].as_f64().unwrap());
    // A press away from the type, for drags that end at the click.
    let away = anchor + Vec2::new(-83.3, -61.7);
    for glyphs in [true, false] {
        let c = ToolContext { snap_to_glyph: glyphs, ..cx(&d, &s, &p) };
        let lands = |q: Point| q.distance(anchor) < 1e-9;
        // A Parametric Shape drawn out from the click.
        let mut tool = ParametricShapeTool::default();
        tool.pointer(&c, &ev(PointerKind::Down, click));
        let acts = tool.pointer(&c, &ev(PointerKind::Drag, click + Vec2::new(80.0, 60.0)));
        let Some(Action::Preview(_, v)) = acts.first() else { panic!("{acts:?}") };
        assert_eq!(lands(center(v)), glyphs, "drag, Snap to Glyph {glyphs}: {:?} vs {anchor:?}", center(v));
        // A Parametric Shape dragged out to the click is sized and turned as one dragged out to
        // the anchor itself.
        let drawn_to = |q: Point| {
            let mut tool = ParametricShapeTool::default();
            tool.pointer(&c, &ev(PointerKind::Down, away));
            tool.pointer(&c, &ev(PointerKind::Drag, q))
        };
        assert_eq!(drawn_to(click) == drawn_to(anchor), glyphs, "dragged point, Snap to Glyph {glyphs}");
        // A click 3 px from the anchor draws nothing, though its center snaps onto the anchor.
        let near = anchor + Vec2::new(2.5, -1.5);
        let mut tool = ParametricShapeTool::default();
        tool.pointer(&c, &ev(PointerKind::Down, near));
        let acts = tool.pointer(&c, &ev(PointerKind::Drag, near));
        let Some(Action::Preview(_, v)) = acts.first() else { panic!("{acts:?}") };
        assert_eq!(lands(center(v)), glyphs, "click, Snap to Glyph {glyphs}: {:?} vs {anchor:?}", center(v));
        assert_eq!(tool.pointer(&c, &ev(PointerKind::Up, near)), vec![Action::Cancel], "click, Snap to Glyph {glyphs}");
        // One placed with a double-click.
        let mut tool = ParametricShapeTool::default();
        let acts = tool.pointer(&c, &ev(PointerKind::DoubleClick, click));
        let Some(Action::Exec(_, v)) = acts.first() else { panic!("{acts:?}") };
        assert_eq!(lands(center(v)), glyphs, "double-click, Snap to Glyph {glyphs}: {:?} vs {anchor:?}", center(v));
        // The pointer hovering at the click shows the guides of the anchor it would land on.
        let mut tool = ParametricShapeTool::default();
        tool.pointer(&c, &ev(PointerKind::Move, click));
        assert_eq!(!tool.overlays(&c).is_empty(), glyphs, "hover, Snap to Glyph {glyphs}");
        // A slice dragged from the click down and to the right starts at its top-left corner.
        let mut tool = SliceTool::default();
        tool.pointer(&c, &ev(PointerKind::Move, click));
        assert_eq!(!tool.overlays(&c).is_empty(), glyphs, "slice hover, Snap to Glyph {glyphs}");
        tool.pointer(&c, &ev(PointerKind::Down, click));
        let acts = tool.pointer(&c, &ev(PointerKind::Drag, click + Vec2::new(80.0, 60.0)));
        let Some(Action::Preview(_, v)) = acts.get(1) else { panic!("{acts:?}") };
        let corner = Point::new(v["x"].as_f64().unwrap(), v["y"].as_f64().unwrap());
        assert_eq!(lands(corner), glyphs, "slice, Snap to Glyph {glyphs}: {corner:?} vs {anchor:?}");
        // One dragged from up and to the left ends at its bottom-right corner.
        let mut tool = SliceTool::default();
        tool.pointer(&c, &ev(PointerKind::Down, away));
        let acts = tool.pointer(&c, &ev(PointerKind::Drag, click));
        let Some(Action::Preview(_, v)) = acts.get(1) else { panic!("{acts:?}") };
        let f = |k: &str| v[k].as_f64().unwrap();
        let end = Point::new(f("x") + f("width"), f("y") + f("height"));
        assert_eq!(end.distance(anchor) < 1e-6, glyphs, "slice end, Snap to Glyph {glyphs}: {end:?} vs {anchor:?}");
    }
}

/// Snap to Glyph: a single line of type has an x-height guide; a paragraph shows glyph bounds only.
#[test]
fn drawn_points_line_up_with_the_x_height_of_a_single_line() {
    let (d, t) = glyph_doc("H", 0.0);
    let lay = layout_text(FontDb::global(), &t);
    let g = &lay.glyphs[0];
    let face = FontDb::global().face_by_id(g.font_id).unwrap();
    let x_height = (t.xf * g.xf * Point::new(0.0, -face.x_height())).y;
    let p = Point::new(400.0, x_height + 2.0);
    let (q, _) = Targets::for_drawing(&d, &[], None, Some(GlyphSnap::default())).snap_point(p, 4.0);
    assert!((q.y - x_height).abs() < 1e-9, "{q:?}, x-height {x_height}");
    let (q, _) = Targets::for_drawing(&d, &[], None, None).snap_point(p, 4.0);
    assert_eq!(q.y, p.y);
    let (d, _) = glyph_doc("H\nH", 0.0);
    let (q, _) = Targets::for_drawing(&d, &[], None, Some(GlyphSnap::default())).snap_point(p, 4.0);
    assert_eq!(q.y, p.y, "a paragraph has no x-height guide");
    // Baseline Shift raises the x-height with the glyphs.
    let (d, _) = glyph_doc_with("H", 0.0, vectorcraft_doc::CharStyle { size: 60.0, baseline_shift: 10.0, ..Default::default() });
    let (q, _) = Targets::for_drawing(&d, &[], None, Some(GlyphSnap::default())).snap_point(Point::new(400.0, x_height - 10.0 + 2.0), 4.0);
    assert!((q.y - (x_height - 10.0)).abs() < 1e-9, "{q:?}, shifted x-height {}", x_height - 10.0);
    // Character Rotation turns the glyph, not its baseline shift.
    let style = vectorcraft_doc::CharStyle { size: 60.0, baseline_shift: 10.0, rotation: 30.0, ..Default::default() };
    let (d, _) = glyph_doc_with("H", 0.0, style);
    let (q, _) = Targets::for_drawing(&d, &[], None, Some(GlyphSnap::default())).snap_point(Point::new(400.0, x_height - 10.0 + 2.0), 4.0);
    assert!((q.y - (x_height - 10.0)).abs() < 1e-9, "{q:?}, shifted x-height {}", x_height - 10.0);
    // Glyph Scaling widens a rotated glyph without raising its x-height.
    let mut t =
        vectorcraft_doc::TextObject::point(Point::ZERO, "HH HH", vectorcraft_doc::CharStyle { size: 60.0, rotation: 90.0, ..Default::default() });
    let frame = kurbo::Shape::to_path(&kurbo::Rect::new(50.0, 100.0, 450.0, 400.0), 0.1);
    t.kind = vectorcraft_doc::TextKind::Area { frame: vectorcraft_geom::PathData::from_bezpath(&frame) };
    t.xf = Affine::IDENTITY;
    t.para.justify = vectorcraft_doc::Justify::JustifyAll;
    t.para.justification.glyph_scaling = [100.0, 100.0, 200.0];
    let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
    let (l, id) = (d.layers[0].id, d.alloc_id());
    d.insert(Some(l), 0, Node::new(id, NodeKind::Text(Box::new(t.clone())))).unwrap();
    let lay = layout_text(FontDb::global(), &t);
    assert_eq!(lay.lines.len(), 1);
    let g = lay.glyphs.iter().find(|g| !g.outline.elements().is_empty()).expect("a glyph with an outline");
    assert!(g.scale > 1.2, "Glyph Scaling widens the glyph: {}", g.scale);
    let expected = (t.xf * g.origin).y - (300.0 - x_height);
    let (q, _) = Targets::for_drawing(&d, &[], None, Some(GlyphSnap::default())).snap_point(Point::new(30.0, expected + 2.0), 4.0);
    assert!((q.y - expected).abs() < 1e-9, "{q:?}, x-height {expected}");
}

/// Snap to Glyph: the x-height guide of type skewed along a slanted path runs through the glyph's
/// x-height.
#[test]
fn the_x_height_guide_of_skewed_path_type_runs_through_the_glyphs_x_height() {
    let mut path = kurbo::BezPath::new();
    path.move_to((100.0, 400.0));
    path.line_to((300.0, 54.0));
    let mut t = vectorcraft_doc::TextObject::point(Point::ZERO, "H", vectorcraft_doc::CharStyle { size: 60.0, ..Default::default() });
    t.kind = vectorcraft_doc::TextKind::OnPath { path: vectorcraft_geom::PathData::from_bezpath(&path), start: 0.0, end: None };
    t.xf = Affine::IDENTITY;
    t.path_effect = vectorcraft_doc::PathEffect::Skew;
    let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
    let (l, id) = (d.layers[0].id, d.alloc_id());
    d.insert(Some(l), 0, Node::new(id, NodeKind::Text(Box::new(t.clone())))).unwrap();
    let lay = layout_text(FontDb::global(), &t);
    let g = lay.glyphs.iter().find(|g| !g.outline.elements().is_empty()).expect("a glyph with an outline");
    let face = FontDb::global().face_by_id(g.font_id).unwrap();
    let top = t.xf * g.xf * Point::new(0.0, -face.x_height());
    let ahead = Vec2::new(g.angle.cos(), g.angle.sin());
    let normal = Vec2::new(ahead.y, -ahead.x);
    // Beside the guide's line, past the glyph's end along it.
    let p = top + ahead * 80.0 + normal * 1.5;
    let (q, _) = Targets::for_drawing(&d, &[], None, Some(GlyphSnap::default())).snap_point(p, 4.0);
    assert!((q - top).cross(ahead).abs() < 1e-6, "{q:?} is off the x-height line through {top:?}");
}

/// Snap to Glyph › Proximity Guides: the edges of each character's advance width are guides.
#[test]
fn drawn_points_line_up_with_the_edge_of_a_characters_width() {
    let (d, t) = glyph_doc("HH", 0.0);
    let lay = layout_text(FontDb::global(), &t);
    let edge = (t.xf * lay.glyphs[1].origin).x;
    let p = Point::new(edge + 0.3, 450.0);
    let (q, _) = Targets::for_drawing(&d, &[], None, Some(GlyphSnap::default())).snap_point(p, 4.0);
    assert!((q.x - edge).abs() < 1e-9, "{q:?}, edge {edge}");
    let (q, _) = Targets::for_drawing(&d, &[], None, None).snap_point(p, 4.0);
    assert_eq!(q.x, p.x);
    // Character Rotation turns the glyphs, not their widths along the baseline.
    let (d, t) = glyph_doc_with("HH", 0.0, vectorcraft_doc::CharStyle { size: 60.0, rotation: 30.0, ..Default::default() });
    let lay = layout_text(FontDb::global(), &t);
    let edge = (t.xf * lay.glyphs[1].origin).x;
    let p = Point::new(edge + 0.3, 450.0);
    let (q, _) = Targets::for_drawing(&d, &[], None, Some(GlyphSnap::default())).snap_point(p, 4.0);
    assert!((q.x - edge).abs() < 1e-9, "{q:?}, edge {edge}");
}

/// Snap to Glyph › Angular Guides: in a rotated glyph a straight upright stroke is a guide at its
/// angle, which a drawn point lands on beyond the glyph.
#[test]
fn a_rotated_glyph_has_angular_guides_along_its_strokes() {
    let angle = 20f64.to_radians();
    let (d, t) = glyph_doc("H", angle);
    let lay = layout_text(FontDb::global(), &t);
    let g = lay.glyphs.iter().find(|g| !g.outline.elements().is_empty()).expect("a glyph with an outline");
    let stem = g
        .outline
        .segments()
        .find_map(|s| match s {
            PathSeg::Line(l) if (l.p1.x - l.p0.x).abs() < 1e-6 && (l.p1.y - l.p0.y).abs() > 10.0 => Some(l),
            _ => None,
        })
        .expect("an upright stroke");
    let (a, b) = (t.xf * stem.p0, t.xf * stem.p1);
    let dir = (b - a) / (b - a).hypot();
    // Past the far end of the stroke, a little beside its line.
    let far = if (a - t.xf * Point::new(0.0, 0.0)).hypot() > (b - t.xf * Point::new(0.0, 0.0)).hypot() { a } else { b };
    let out = if far == a { -dir } else { dir };
    let p = far + out * 30.0 + Vec2::new(-dir.y, dir.x) * 1.5;
    let on_line = |q: Point| (q - a).cross(dir).abs();
    let (q, ov) = Targets::for_drawing(&d, &[], None, Some(GlyphSnap::default())).snap_point(p, 4.0);
    assert!(on_line(q) < 1e-9, "{q:?} is {} off the stroke's line", on_line(q));
    assert!(!lines(&ov).is_empty());
    let (q, _) = Targets::for_drawing(&d, &[], None, None).snap_point(p, 4.0);
    assert!(on_line(q) > 1e-3, "no angular guide with Snap to Glyph off");
}

/// The Character panel's Snap to Glyph options (#4232): each one off takes away its own guides and
/// leaves the others.
#[test]
fn each_snap_to_glyph_option_takes_away_only_its_own_guides() {
    // The glyph guides: lines across the page (ys), down it (xs), at an angle, and anchor points.
    let count = |d: &vectorcraft_doc::Document, o: GlyphSnap| {
        let t = Targets::for_drawing(d, &[], None, Some(o));
        let glyph = |v: &[(f64, Point, Kind)]| v.iter().filter(|e| e.2 == Kind::Glyph).count();
        [glyph(&t.ys), glyph(&t.xs), t.angled.len(), t.points.iter().filter(|e| e.1 == Kind::Anchor).count()]
    };
    let all = GlyphSnap::default();
    let (d, t) = glyph_doc("H", 0.0);
    let on = count(&d, all);
    let lay = layout_text(FontDb::global(), &t);
    let g = &lay.glyphs[0];
    let anchors = g.outline.segments().count();
    assert!(anchors > 0);
    // One line, one glyph: a baseline, an x-height, two width edges, four corners and its anchors.
    for (off, less) in [
        (GlyphSnap { baseline: false, ..all }, [1, 0, 0, 0]),
        (GlyphSnap { x_height: false, ..all }, [1, 0, 0, 0]),
        (GlyphSnap { bounds: false, ..all }, [2, 2, 0, 0]),
        (GlyphSnap { proximity: false, ..all }, [0, 2, 0, 0]),
        (GlyphSnap { anchors: false, ..all }, [0, 0, 0, anchors]),
        (GlyphSnap { angular: false, ..all }, [0, 0, 0, 0]),
    ] {
        let got = count(&d, off);
        assert_eq!(std::array::from_fn::<usize, 4, _>(|i| on[i] - got[i]), less, "{off:?}: {got:?} of {on:?}");
    }
    // A point near the x-height lands on it only with x-height on.
    let face = FontDb::global().face_by_id(g.font_id).unwrap();
    let x_height = (t.xf * g.xf * Point::new(0.0, -face.x_height())).y;
    let p = Point::new(400.0, x_height + 2.0);
    assert!((Targets::for_drawing(&d, &[], None, Some(all)).snap_point(p, 4.0).0.y - x_height).abs() < 1e-9);
    assert_eq!(Targets::for_drawing(&d, &[], None, Some(GlyphSnap { x_height: false, ..all })).snap_point(p, 4.0).0.y, p.y);
    // Rotated type: its baseline, x-height, width edges, glyph bounds and strokes run at an angle,
    // all Angular Guides (the corners of its glyph bounds stay points to land on).
    let (d, _) = glyph_doc("H", 20f64.to_radians());
    let (on, off) = (count(&d, all), count(&d, GlyphSnap { angular: false, ..all }));
    assert!(on[2] > 0, "{on:?}");
    assert_eq!(off, [on[0], on[1], 0, on[3]]);
}

/// Snap to Glyph: the baseline of type turned slightly runs at its angle, and a point near it
/// lands on it even close to where it starts, far nearer the horizontal through that point.
#[test]
fn a_slightly_rotated_baseline_pulls_points_near_its_start() {
    let (d, t) = glyph_doc("H", 2f64.to_radians());
    let lay = layout_text(FontDb::global(), &t);
    let m = t.xf * lay.line_xf;
    let start = m * Point::new(lay.lines[0].x0, lay.lines[0].baseline);
    let dir = (m * Point::new(1.0, 0.0)) - (m * Point::ZERO);
    let dir = dir / dir.hypot();
    let p = start - dir * 10.0 + Vec2::new(-dir.y, dir.x) * 1.0;
    let (q, _) = Targets::for_drawing(&d, &[], None, Some(GlyphSnap::default())).snap_point(p, 4.0);
    assert!((q - start).cross(dir).abs() < 1e-9, "{q:?} is off the baseline");
}

/// Snap to Glyph: a segment Shift keeps level slides along to where it crosses the baseline of
/// rotated type.
#[test]
fn a_shift_constrained_point_slides_onto_a_rotated_baseline() {
    let (d, t) = glyph_doc("H", 20f64.to_radians());
    let lay = layout_text(FontDb::global(), &t);
    let m = t.xf * lay.line_xf;
    let start = m * Point::new(lay.lines[0].x0, lay.lines[0].baseline);
    let dir = (m * Point::new(1.0, 0.0)) - (m * Point::ZERO);
    let dir = dir / dir.hypot();
    let crossing = start - dir * 40.0;
    let (s, p) = (Selection::default(), paint());
    let c = cx(&d, &s, &p);
    let from = Leave::segment(&c, crossing - Vec2::new(60.0, 0.0), true);
    let (q, _) = Targets::for_drawing(&d, &[], None, Some(GlyphSnap::default())).snap_from(crossing + Vec2::new(1.5, 0.3), Some(&from), 4.0);
    assert!(q.distance(crossing) < 1e-9, "{q:?} vs {crossing:?}");
}

/// The corners of a glyph's bounds, from its outline in its own frame placed as the glyph is.
fn glyph_corners(t: &vectorcraft_doc::TextObject) -> [Point; 4] {
    let lay = layout_text(FontDb::global(), t);
    let g = lay.glyphs.iter().find(|g| !g.outline.elements().is_empty()).expect("a glyph with an outline");
    let r = (g.xf.inverse() * g.outline.clone()).bounding_box();
    [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)].map(|p| t.xf * g.xf * p)
}

/// Snap to Glyph › Glyph Bounds (#5112): each corner of a glyph's bounds is a point a drawn point
/// lands on, also where its outline curves (the right of `B`, where no anchor is), and the corners
/// turn with rotated type. Glyph Bounds off, they are gone.
#[test]
fn a_drawn_point_lands_on_a_corner_of_a_glyphs_bounds() {
    let style = vectorcraft_doc::CharStyle { size: 200.0, ..Default::default() };
    for angle in [0.0, 30f64.to_radians()] {
        let (d, t) = glyph_doc_with("B", angle, style.clone());
        let corners = glyph_corners(&t);
        for (i, c) in corners.into_iter().enumerate() {
            let p = c + Vec2::new(2.0, -1.5);
            let (q, ov) = Targets::for_drawing(&d, &[], None, Some(GlyphSnap::default())).snap_point(p, 4.0);
            assert!(q.distance(c) < 1e-9, "angle {angle}: {q:?} vs corner {c:?}");
            // The left corners of `B` are anchors of its stem too; the right ones, on its curved
            // side, come from its bounds alone.
            if i == 0 || i == 3 {
                continue;
            }
            assert!(ov.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "glyph")), "{ov:?}");
            let off = GlyphSnap { bounds: false, ..GlyphSnap::default() };
            let (q, _) = Targets::for_drawing(&d, &[], None, Some(off)).snap_point(p, 4.0);
            assert!(q.distance(c) > 1e-3, "Glyph Bounds off: {q:?} still on {c:?}");
        }
        // The right edge of the rotated glyph runs at its angle: a point beside it, far from the
        // corners, lands on it.
        let (tr, br) = (corners[1], corners[2]);
        let dir = (br - tr) / (br - tr).hypot();
        let p = tr.midpoint(br) + Vec2::new(-dir.y, dir.x) * 1.5;
        let (q, _) =
            Targets::for_drawing(&d, &[], None, Some(GlyphSnap { anchors: false, proximity: false, ..GlyphSnap::default() })).snap_point(p, 4.0);
        assert!((q - tr).cross(dir).abs() < 1e-6, "angle {angle}: {q:?} is off the right edge");
    }
}

/// Snap to Glyph: both ends of a gradient vector drawn on the art with the Gradient tool, a click
/// that applies one, and its dragged start and end handles, land on a glyph's anchor point as a
/// Pen click does, with the guides shown while hovering, and not with Snap to Glyph off. A drawn
/// end also lands on a slanted path's segment, which no bounds line passes along, and on a
/// construction guide through its start, and with Shift on an anchor along its 45° step. With nothing
/// selected, hovering off the art shows no guides. A press that snaps still starts a drag only
/// after the pointer moves 3 px from where it was pressed.
#[test]
fn gradient_vector_ends_land_on_a_glyph_anchor() {
    use crate::xform::GradientTool;
    use crate::{Action, PointerEvent, PointerKind, Tool};
    use vectorcraft_color::{Gradient, GradientGeom, GradientPaint, Paint};
    use vectorcraft_doc::Appearance;
    use vectorcraft_geom::shapes;
    let (mut d, t) = glyph_doc("H", 0.0);
    let lay = layout_text(FontDb::global(), &t);
    let g = lay.glyphs.iter().find(|g| !g.outline.elements().is_empty()).expect("a glyph with an outline");
    // An anchor inside the glyph's bounds, which no glyph bounds line passes through.
    let bb = g.outline.bounding_box();
    let inside = |p: Point| p.x > bb.x0 + 1.0 && p.x < bb.x1 - 1.0 && p.y > bb.y0 + 1.0 && p.y < bb.y1 - 1.0;
    let anchor = t.xf * g.outline.segments().map(|s| s.start()).find(|p| inside(*p)).expect("an anchor inside the glyph");
    let type_id = d.children(Some(d.layers[0].id)).unwrap()[0].id;
    // A rectangle away from the type, with a gradient from (300, 100) to (400, 100).
    let (l, rect) = (d.layers[0].id, d.alloc_id());
    d.insert(Some(l), 0, Node::path(rect, shapes::rectangle(Rect::new(300.0, 50.0, 400.0, 150.0)), Appearance::default_art())).unwrap();
    let mut gp = GradientPaint::new(Gradient::default());
    gp.geom = Some(GradientGeom { start: Point::new(300.0, 100.0), end: Point::new(400.0, 100.0), aspect: 1.0, focal: None });
    d.node_mut(rect).unwrap().appearance.set_fill(Paint::Gradient(Box::new(gp)));
    // A slanted line, its middle far from its bounds' edges.
    let (a0, a1) = (Point::new(200.0, 420.0), Point::new(280.0, 480.0));
    let line = d.alloc_id();
    d.insert(Some(l), 0, Node::path(line, shapes::line(a0, a1), Appearance::default_art())).unwrap();
    let mut on_type = Selection::default();
    on_type.add(type_id);
    let mut on_rect = Selection::default();
    on_rect.add(rect);
    let p = paint();
    let click = anchor + Vec2::new(1.5, -1.0);
    let away = anchor + Vec2::new(-83.3, -61.7);
    let ev = |kind, q: Point| PointerEvent::new(kind, q.x, q.y);
    let preview = |acts: &[Action]| {
        acts.iter().find_map(|a| if let Action::Preview(_, v) = a { Some(v.clone()) } else { None }).unwrap_or_else(|| panic!("{acts:?}"))
    };
    let at = |v: &serde_json::Value| Point::new(v[0].as_f64().unwrap(), v[1].as_f64().unwrap());
    for glyphs in [true, false] {
        let c = ToolContext { snap_to_glyph: glyphs, ..cx(&d, &on_type, &p) };
        let lands = |q: Point| q.distance(anchor) < 1e-9;
        // The pointer hovering at the click shows the guides of the anchor a vector would start on.
        let mut tool = GradientTool::default();
        tool.pointer(&c, &ev(PointerKind::Move, click));
        assert_eq!(!tool.overlays(&c).is_empty(), glyphs, "hover, Snap to Glyph {glyphs}");
        // A vector drawn from the click starts on the anchor.
        tool.pointer(&c, &ev(PointerKind::Down, click));
        let v = preview(&tool.pointer(&c, &ev(PointerKind::Drag, click + Vec2::new(80.0, 60.0))));
        assert_eq!(lands(at(&v["start"])), glyphs, "start, Snap to Glyph {glyphs}: {v}");
        tool.pointer(&c, &ev(PointerKind::Up, click + Vec2::new(80.0, 60.0)));
        // One drawn to beside the slanted line, a quarter of the way along (15 pt or more from its
        // bounds' edges, centre and anchors), ends on the line.
        let dir = (a1 - a0) / (a1 - a0).hypot();
        let quarter = a0 + (a1 - a0) * 0.25;
        tool.pointer(&c, &ev(PointerKind::Down, away));
        let v = preview(&tool.pointer(&c, &ev(PointerKind::Drag, quarter + Vec2::new(-dir.y, dir.x) * 2.0)));
        assert!((at(&v["end"]) - a0).cross(dir).abs() < 1e-6, "end on the line: {v}");
        tool.pointer(&c, &ev(PointerKind::Up, quarter));
        // One drawn to the click ends on it.
        tool.pointer(&c, &ev(PointerKind::Down, away));
        let v = preview(&tool.pointer(&c, &ev(PointerKind::Drag, click)));
        assert_eq!(lands(at(&v["end"])), glyphs, "end, Snap to Glyph {glyphs}: {v}");
        tool.pointer(&c, &ev(PointerKind::Up, click));
        // A click at the click applies a gradient that starts on the anchor.
        tool.pointer(&c, &ev(PointerKind::Down, click));
        let acts = tool.pointer(&c, &ev(PointerKind::Up, click));
        let v = acts.iter().find_map(|a| if let Action::Exec(_, v) = a { Some(v.clone()) } else { None }).unwrap_or_else(|| panic!("{acts:?}"));
        assert_eq!(lands(at(&v["start"])), glyphs, "click, Snap to Glyph {glyphs}: {v}");
        // One drawn to 2 px below the horizontal through its start, far from any target, ends on
        // that construction guide, as a Line Segment's end does.
        tool.pointer(&c, &ev(PointerKind::Down, away));
        let v = preview(&tool.pointer(&c, &ev(PointerKind::Drag, away + Vec2::new(150.0, 2.0))));
        assert!((at(&v["end"]).y - away.y).abs() < 1e-9, "end on the construction guide: {v}");
        tool.pointer(&c, &ev(PointerKind::Up, away + Vec2::new(150.0, 2.0)));
        // With Shift, one drawn at 45° toward the line's first anchor slides along that step onto it.
        let shift = |kind, q: Point| PointerEvent { mods: crate::Mods { shift: true, ..Default::default() }, ..ev(kind, q) };
        tool.pointer(&c, &shift(PointerKind::Down, a0 + Vec2::new(60.0, -60.0)));
        let v = preview(&tool.pointer(&c, &shift(PointerKind::Drag, a0 + Vec2::new(-1.5, 0.5))));
        assert!(at(&v["end"]).distance(a0) < 1e-9, "Shift end on the anchor: {v}");
        tool.pointer(&c, &shift(PointerKind::Up, a0));
        // A press 3 px from the anchor, then a move of 2 px away from it: no drag yet.
        let near = anchor + Vec2::new(2.5, -1.5);
        tool.pointer(&c, &ev(PointerKind::Down, near));
        let acts = tool.pointer(&c, &ev(PointerKind::Drag, near + Vec2::new(1.5, -1.0)));
        assert!(acts.is_empty(), "drag start, Snap to Glyph {glyphs}: {acts:?}");
        tool.pointer(&c, &ev(PointerKind::Up, near + Vec2::new(1.5, -1.0)));
        // The end handle of the rectangle's gradient Alt-dragged to the click lands on the anchor.
        let c = ToolContext { snap_to_glyph: glyphs, ..cx(&d, &on_rect, &p) };
        let mut tool = GradientTool::default();
        // Hovering over the handle (on the rectangle's edge) shows no guides.
        tool.pointer(&c, &ev(PointerKind::Move, Point::new(400.0, 100.0)));
        assert!(!tool.overlays(&c).iter().any(|o| matches!(o, Overlay::Label { .. })), "hover over the handle");
        let alt = |kind, q: Point| PointerEvent { mods: crate::Mods { alt: true, ..Default::default() }, ..ev(kind, q) };
        tool.pointer(&c, &alt(PointerKind::Down, Point::new(400.0, 100.0)));
        let v = preview(&tool.pointer(&c, &alt(PointerKind::Drag, click)));
        assert_eq!(lands(at(&v["end"])), glyphs, "end handle, Snap to Glyph {glyphs}: {v}");
        tool.pointer(&c, &ev(PointerKind::Up, click));
        // The start handle dragged to the click lands on the anchor.
        tool.pointer(&c, &ev(PointerKind::Down, Point::new(300.0, 100.0)));
        let v = preview(&tool.pointer(&c, &ev(PointerKind::Drag, click)));
        assert_eq!(lands(at(&v["start"])), glyphs, "start handle, Snap to Glyph {glyphs}: {v}");
        tool.pointer(&c, &ev(PointerKind::Up, click));
        // With nothing selected, hovering beside the rectangle (off the art and its 1 pt stroke,
        // where a press draws nothing, but within the snapping tolerance of its edge) shows no
        // guides.
        let none = Selection::default();
        let c = ToolContext { snap_to_glyph: glyphs, ..cx(&d, &none, &p) };
        let mut tool = GradientTool::default();
        tool.pointer(&c, &ev(PointerKind::Move, Point::new(296.2, 75.0)));
        assert!(tool.overlays(&c).is_empty(), "hover off the art: {:?}", tool.overlays(&c));
    }
}

/// With Smart Guides off, View → Snap to Point still pulls a drawn point onto an anchor, but
/// draws no Smart Guides: no "anchor" label, no lines.
#[test]
fn snap_to_point_without_smart_guides_shows_no_guides() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    // Within Snap to Point's 2 px of the anchor at (200, 200).
    let at = Point::new(201.0, 199.0);
    let off = ToolContext { smart_guides: false, snap_to_point: true, ..cx(&d, &s, &p) };
    let mut snap = DrawSnap::default();
    assert_eq!(snap.hover(&off, at, &[], None), Point::new(200.0, 200.0));
    assert!(snap.guides().is_empty(), "{:?}", snap.guides());
    assert_eq!(snap.press(&off, at, &[], None), Point::new(200.0, 200.0));
    assert!(snap.guides().is_empty(), "{:?}", snap.guides());
    // A dragged selection's grabbed point too.
    let (q, ov) = Targets::snap_to_point(&off, &[]).unwrap().snap_point(at, 4.0);
    assert_eq!((q, ov.len()), (Point::new(200.0, 200.0), 0), "{ov:?}");
    // Smart Guides on: the same point says "anchor".
    let mut snap = DrawSnap::default();
    assert_eq!(snap.hover(&cx(&d, &s, &p), at, &[], None), Point::new(200.0, 200.0));
    assert_eq!(labels(snap.guides()), ["anchor"]);
}

/// A reference hover must not bake the whole document again on every move.
/// The actual root/document/option key refreshes the observable stroke contact.
#[test]
fn pick_snap_reuses_contours_until_document_roots_or_option_change() {
    fn width(mut d: Document, value: f64) -> Document {
        let layer = Arc::make_mut(&mut d.layers[0]);
        let node = Arc::make_mut(&mut layer.children_mut().unwrap()[0]);
        for item in &mut node.appearance.items {
            if let vectorcraft_doc::AppearanceItem::Stroke(stroke) = item {
                stroke.width = value;
            }
        }
        d
    }
    let (d, id) = doc_with_rect();
    let d = width(d, 10.0);
    let empty = Document::new(500.0, 500.0);
    let s = Selection { objects: vec![id], ..Default::default() };
    let p = paint();
    let at = Point::new(207.0, 150.0);
    let mut snap = PickSnap::default();
    let c = ToolContext { revision: (1, 1), smart_guides: true, snap_to_stroke_edges: true, snapping_tolerance: 4.0, ..cx(&d, &s, &p) };
    assert_eq!(snap.snap(&c, at).0, Point::new(205.0, 150.0));
    // As in the existing DrawSnap cache test, a deliberately changed document
    // behind the SAME authoritative revision proves targets are reused.
    let c = ToolContext { doc: &empty, ..c };
    assert_eq!(snap.snap(&c, at).0, Point::new(205.0, 150.0));
    let c = ToolContext { revision: (1, 2), ..c };
    assert_eq!(snap.snap(&c, at).0, at);
    // A different document needs a new authoritative revision; the separate
    // root-transition test keeps one real document while changing hit roots.
    let mut d2 = Document::new(500.0, 500.0);
    let _padding_id = d2.alloc_id();
    let id2 = d2.alloc_id();
    let layer2 = d2.layers[0].id;
    d2.insert(
        Some(layer2),
        0,
        Node::path(id2, vectorcraft_geom::shapes::rectangle(Rect::new(100.0, 100.0, 200.0, 200.0)), vectorcraft_doc::Appearance::default_art()),
    )
    .unwrap();
    let d2 = width(d2, 20.0);
    let s2 = Selection { objects: vec![id2], ..Default::default() };
    let c2 = ToolContext { doc: &d2, selection: &s2, revision: (1, 3), ..c };
    assert_eq!(snap.snap(&c2, at).0, Point::new(210.0, 150.0));
    let d3 = width(d2.clone(), 10.0);
    let off = ToolContext { doc: &d3, snap_to_stroke_edges: false, ..c2 };
    assert_eq!(snap.snap(&off, at).0, at);
    let on = ToolContext { snap_to_stroke_edges: true, ..off };
    assert_eq!(snap.snap(&on, at).0, Point::new(205.0, 150.0));
}

/// Default-OFF reference picking retains the old selection/hit root order:
/// equal-distance anchors with equal x keep their stable gather priority.
#[test]
fn pick_snap_off_preserves_legacy_root_order_for_equal_distance_points() {
    let mut d = Document::new(500.0, 500.0);
    let layer = d.layers[0].id;
    let mut ids = vec![];
    for y in [100.0, 104.0] {
        let mut path = BezPath::new();
        path.move_to((100.0, y));
        path.line_to((110.0, y));
        let id = d.alloc_id();
        d.insert(Some(layer), 0, Node::path(id, vectorcraft_geom::PathData::from_bezpath(&path), vectorcraft_doc::Appearance::default_art()))
            .unwrap();
        ids.push(id);
    }
    let p = paint();
    let at = Point::new(100.0, 102.0);
    let mut pick = PickSnap::default();
    for (roots, expected_y) in [(vec![ids[0], ids[1]], 100.0), (vec![ids[1], ids[0]], 104.0)] {
        let selection = Selection { objects: roots.clone(), ..Default::default() };
        let c = ToolContext { smart_guides: true, snap_to_stroke_edges: false, snapping_tolerance: 4.0, ..cx(&d, &selection, &p) };
        let tol = c.snap_tol();
        let near = Rect::new(at.x - tol, at.y - tol, at.x + tol, at.y + tol);
        // Neither short line's centre is the probe; all possible hit roots
        // are already in this selected pair, so an appended hit cannot precede it.
        let legacy = Targets::points_near(&d, &roots, near).styled(&c).snap_point(at, tol).0;
        assert_eq!(legacy, Point::new(100.0, expected_y), "the original stable tie must follow selected root order");
        assert_eq!(pick.snap(&c, at).0, legacy, "stroke edges OFF must not sort legacy roots");
    }
}

/// A carried bleed point contacts the continuous stroke channel on the
/// handle's permitted trajectory, without replacing legacy anchor priority.
#[test]
fn stroke_handle_contact_keeps_bleed_offset_constraint_and_legacy_priority() {
    let mut d = Document::new(500.0, 500.0);
    let layer = d.layers[0].id;
    let id = d.alloc_id();
    let mut appearance = vectorcraft_doc::Appearance::default_art();
    appearance.set_fill(vectorcraft_color::Paint::None);
    appearance.stroke_mut().unwrap().width = 10.0;
    let line = vectorcraft_geom::shapes::line(Point::new(100.0, 100.0), Point::new(200.0, 200.0));
    d.insert(Some(layer), 0, Node::path(id, line, appearance)).unwrap();
    let (selection, paint) = (Selection::default(), paint());
    let c = ToolContext { snap_to_stroke_edges: true, snapping_tolerance: 4.0, ..cx(&d, &selection, &paint) };
    let targets = Targets::collect(&d, &[], None).styled(&c).with_stroke_edges(&c, &[], None);
    let at = Point::new(162.0, 150.0);
    let along = Some(Vec2::new(1.0, 0.0));
    // The ordinary handle is nearly 5 pt from the horizontal intersection;
    // only the bleed point 5 pt to its left is in reach.
    assert!(targets.snap_stroke_point_with(at, at, &[], along, 4.0).is_none());
    let offset = Vec2::new(-5.0, 0.0);
    let (q, ov) = targets.snap_stroke_point_with(at, at, &[offset], along, 4.0).unwrap();
    let band = 5.0 * std::f64::consts::SQRT_2;
    assert!((q.x - (155.0 + band)).abs() < 1e-8, "{q:?}");
    assert_eq!(q.y, at.y);
    assert!(ov.iter().any(|o| matches!(o, Overlay::Label { p, text, .. } if text == "stroke edge" && p.distance(q + offset) < 1e-8)));
    // A free corner may move on both axes; a parallel permitted trajectory
    // cannot replace its missing intersection with a projected free snap.
    let free = Point::new(159.0, 150.0);
    let (q, _) = targets.snap_stroke_point_with(free, free, &[], None, 4.0).unwrap();
    assert!((q.x - q.y - band).abs() < 1e-8);
    assert!(q.distance(free) <= 4.0);
    assert!(targets.snap_stroke_point_with(free, free, &[], Some(Vec2::new(1.0, 1.0)), 4.0).is_none());
    // Legacy priority is checked at the original input, not its constrained
    // projection. Here that input is the actual line's original anchor.
    assert!(targets.snap_stroke_point_with(Point::new(100.0, 100.0), at, &[offset], along, 4.0).is_none());
    let off = ToolContext { snap_to_stroke_edges: false, ..c };
    let targets = Targets::collect(&d, &[], None).with_stroke_edges(&off, &[], None);
    assert!(targets.snap_stroke_point_with(at, at, &[offset], along, 4.0).is_none());
}

/// A compound SHAPE keeps its own stroke targets when an unpainted parent
/// materializes it as a Pathfinder member; ordinary rendered art stays the same.
#[test]
fn stroke_review_nested_compound_origin() {
    use vectorcraft_doc::color::{Color, Paint};
    use vectorcraft_doc::{Appearance, Effect};

    let probe = Point::new(207.0, 140.0);
    for add_enabled in [false, true] {
        let mut d = Document::new(500.0, 500.0);
        let layer = d.layers[0].id;
        let (back, front, inner, outer) = (d.alloc_id(), d.alloc_id(), d.alloc_id(), d.alloc_id());
        let rectangle = |id, rect| Arc::new(Node::path(id, vectorcraft_geom::shapes::rectangle(rect), Appearance::default()));
        let mut compound =
            Node::group(inner, vec![rectangle(back, Rect::new(100.0, 100.0, 175.0, 200.0)), rectangle(front, Rect::new(150.0, 100.0, 200.0, 200.0))]);
        compound.extras_mut().compound_shape = true;
        compound.appearance = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 10.0).into();
        let mut parent = Node::group(outer, vec![Arc::new(compound)]);
        parent.appearance.effects.push(Effect { id: "pathfinder.add".into(), params: serde_json::json!({}), visible: add_enabled });
        d.insert(Some(layer), 0, parent).unwrap();
        assert!(d.node(inner).unwrap().is_compound_shape(), "use the compound-shape materializer, not a compound path");
        assert!(d.node(outer).unwrap().appearance.items.is_empty(), "the parent must not replace the child's own paint");
        assert_eq!(vectorcraft_effects::has_pathfinder(d.node(outer).unwrap()), add_enabled);
        let original = d.clone();
        if add_enabled {
            // The ordinary Pathfinder result is already painted before the fix;
            // missing query metadata must not be mistaken for missing artwork.
            let art = vectorcraft_effects::pathfinder_children(Some(&d), d.node(outer).unwrap(), None).unwrap();
            assert_eq!(art.len(), 1);
            assert_eq!(art[0].geometric_bounds(), Some(Rect::new(100.0, 100.0, 200.0, 200.0)));
            assert_eq!(art[0].appearance.stroke().unwrap().width, 10.0);
            assert!(art[0].extras().query_origin.0.is_none(), "ordinary evaluation does not invent a query origin");
        }
        let (selection, paint) = (Selection::default(), paint());
        let off = ToolContext { snap_to_stroke_edges: false, snapping_tolerance: 4.0, ..cx(&d, &selection, &paint) };
        let off_targets = Targets::collect(&d, &[], None).with_stroke_edges(&off, &[], None);
        assert_eq!(off_targets.snap_point(probe, 4.0).0, probe, "legacy anchors/alignment do not explain the stroke contact");
        let on = ToolContext { snap_to_stroke_edges: true, snapping_tolerance: 4.0, ..cx(&d, &selection, &paint) };
        let excluded = Targets::collect(&d, &[inner], None).with_stroke_edges(&on, &[inner], None);
        assert_eq!(excluded.snap_point(probe, 4.0).0, probe, "excluding the inner paint owner must remove its outer stroke edge");
        let targets = Targets::collect(&d, &[], None).with_stroke_edges(&on, &[], None);
        let snapped = targets.snap_point(probe, 4.0).0;
        assert_eq!(d, original, "query evaluation must leave the actual source document intact");
        for layer in &d.layers {
            layer.walk(&mut |n| assert!(n.extras().query_origin.0.is_none(), "source nodes keep no transient query metadata"));
        }
        assert_eq!(snapped, Point::new(205.0, 140.0), "outer Pathfinder Add enabled={add_enabled}");
    }
}

/// Incoming non-similarity type outlines materialize character strokes as
/// filled paths. Their real painted bands remain stroke targets, unlike glyph fills.
#[test]
fn stroke_review_nonuniform_type_stroke_origin() {
    use vectorcraft_doc::color::{Color, Paint};
    use vectorcraft_doc::{CharStyle, QueryPaintKind, TextObject};
    use vectorcraft_geom::{FillRule, PathData};

    fn nearest(path: &BezPath, p: Point) -> Point {
        path.segments()
            .map(|s| s.eval(s.nearest(p, 1e-6).t))
            .min_by(|a, b| a.distance_squared(p).total_cmp(&b.distance_squared(p)))
            .expect("the real glyph or stroke band must have an outline")
    }
    // Select one ordinary straight part of this fixed glyph's actual outline,
    // away from its corners and every legacy target. This is not a font corpus.
    fn witness(path: &BezPath, legacy: &Targets, avoid: Option<&BezPath>, tol: f64) -> (Point, Point) {
        for segment in path.segments() {
            let PathSeg::Line(line) = segment else { continue };
            let tangent = line.p1 - line.p0;
            let length = tangent.hypot();
            if length <= 12.0 {
                continue;
            }
            let at = line.p0.midpoint(line.p1);
            let probe = at + Vec2::new(-tangent.y, tangent.x) * (0.75 / length);
            let on = nearest(path, probe);
            if on.distance(at) < 1e-6
                && on.distance(probe) > 0.25
                && on.distance(probe) <= tol
                && legacy.snap_point(probe, tol).0 == probe
                && avoid.is_none_or(|p| nearest(p, on).distance(on) > tol)
            {
                return (probe, on);
            }
        }
        panic!("the fixed Source Sans 3 H must provide an isolated outline contact");
    }
    fn document(style: CharStyle, xf: Affine) -> (Document, NodeId, TextObject) {
        let mut d = Document::new(800.0, 600.0);
        let layer = d.layers[0].id;
        let id = d.alloc_id();
        let mut text = TextObject::point(Point::ZERO, "H", style);
        text.xf = xf;
        d.insert(Some(layer), 0, Node::new(id, NodeKind::Text(Box::new(text.clone())))).unwrap();
        (d, id, text)
    }
    let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let style = CharStyle {
        font_family: "Source Sans 3".into(),
        size: 160.0,
        fill: Paint::solid(Color::BLACK),
        stroke: red.clone(),
        stroke_width: 8.0,
        ..CharStyle::default()
    };
    let tol = 1.5;
    // Similarity first is a real passing control for the existing live-stroke
    // route; the second transform requires the new filled-stroke materializer.
    for (filled, xf) in [(false, Affine::new([1.5, 0.0, 0.0, 1.5, 150.0, 300.0])), (true, Affine::new([2.0, 0.0, 0.35, 1.0, 150.0, 300.0]))] {
        let (d, id, text) = document(style.clone(), xf);
        let original = d.clone();
        let layout = layout_text(FontDb::global(), &text);
        assert_eq!(layout.glyphs.len(), 1, "the original fixture is one actual H glyph");
        let glyph = &layout.glyphs[0].outline;
        assert!(!glyph.elements().is_empty(), "the pinned document font must supply real outlines");
        let ink = xf * glyph.clone();
        // Independent geometry oracle: stroke the laid-out glyph in text space,
        // then transform the resulting band, matching the canvas's operation order.
        let band = vectorcraft_effects::stroke::outline_region(&PathData::from_bezpath(glyph), FillRule::NonZero, &style.stroke_layer())
            .transformed(xf)
            .to_bezpath();
        let (selection, paint) = (Selection::default(), paint());
        let off = ToolContext { snap_to_stroke_edges: false, snapping_tolerance: tol, ..cx(&d, &selection, &paint) };
        let legacy = Targets::collect(&d, &[], None).with_stroke_edges(&off, &[], None);
        let (probe, expected) = witness(&band, &legacy, Some(&ink), tol);
        assert_eq!(legacy.snap_point(probe, tol).0, probe);
        let ordinary = vectorcraft_effects::outline_text(d.node(id).unwrap()).unwrap();
        ordinary.walk(&mut |n| assert!(n.extras().query_origin.0.is_none(), "ordinary outlines remain untagged"));
        if filled {
            let painted_band = ordinary.children().unwrap().iter().find(|n| n.appearance.fill_paint() == red).expect("actual filled red stroke");
            assert!(painted_band.appearance.stroke().is_none());
            let actual_band = painted_band.path_data().unwrap().to_bezpath();
            assert!(nearest(&actual_band, probe).distance(expected) < 1e-6, "ordinary materialization keeps the independently outlined band");
        } else {
            assert!(ordinary.children().unwrap().iter().any(|n| n.appearance.stroke().is_some()), "similarity retains the live character stroke");
        }
        // A real fill-only H must not become a stroke target, even near an ink
        // contour that the old anchors/centres/alignment cannot explain.
        let (fill_doc, _, _) = document(CharStyle { stroke: Paint::None, ..style.clone() }, xf);
        let fill_original = fill_doc.clone();
        let fill_legacy = Targets::collect(&fill_doc, &[], None);
        let (fill_probe, _) = witness(&ink, &fill_legacy, None, tol);
        let fill_cx = ToolContext { snap_to_stroke_edges: true, snapping_tolerance: tol, ..cx(&fill_doc, &selection, &paint) };
        let fill_targets = Targets::collect(&fill_doc, &[], None).with_stroke_edges(&fill_cx, &[], None);
        assert_eq!(fill_targets.snap_point(fill_probe, tol).0, fill_probe, "original glyph fill is not a stroke edge");
        assert_eq!(fill_doc, fill_original);
        let on = ToolContext { snap_to_stroke_edges: true, snapping_tolerance: tol, ..cx(&d, &selection, &paint) };
        let targets = Targets::collect(&d, &[], None).with_stroke_edges(&on, &[], None);
        let snapped = targets.snap_point(probe, tol).0;
        assert!(
            snapped.distance(expected) < 1e-6 && snapped.distance(probe) > 0.25,
            "filled={filled}: {probe:?} -> {snapped:?}, expected {expected:?}"
        );
        if filled {
            let view = vectorcraft_effects::stroke_paint_view(&d);
            let query_outline = vectorcraft_effects::outline_text(view.node(id).unwrap()).expect("the real tagged text's outlines");
            let (mut geometry_fill, mut stroke_fill) = (false, false);
            query_outline.walk(&mut |n| {
                if n.path_data().is_some() && n.appearance.stroke().is_none() && !n.appearance.fill_paint().is_none() {
                    match n.extras().query_origin.0.map(|o| o.kind) {
                        Some(QueryPaintKind::Geometry) => geometry_fill = true,
                        Some(QueryPaintKind::Stroke) => stroke_fill = true,
                        _ => {}
                    }
                }
            });
            assert!(geometry_fill && stroke_fill, "real query view keeps original glyph fill and materialized stroke distinct");
        }
        assert_eq!(d, original, "actual text and paints stay unchanged");
        for layer in d.layers.iter().chain(&fill_doc.layers) {
            layer.walk(&mut |n| assert!(n.extras().query_origin.0.is_none(), "original documents retain no transient origin"));
        }
    }
}

// Additive to crates/tools/src/guides/tests.rs on exact c39.
// These cases use only APIs already present in that Before source.
fn review_597185_path(d: &mut Document, points: &[Point], width: Option<f64>) {
    let mut path = BezPath::new();
    path.move_to(points[0]);
    for point in &points[1..] {
        path.line_to(*point);
    }
    let mut appearance = vectorcraft_doc::Appearance::default_art();
    if let Some(width) = width {
        appearance.set_fill(vectorcraft_color::Paint::None);
        appearance.stroke_mut().unwrap().width = width;
    } else {
        path.close_path();
        appearance.set_stroke(vectorcraft_color::Paint::None);
    }
    let id = d.alloc_id();
    d.insert(Some(d.layers[0].id), 0, Node::path(id, vectorcraft_geom::PathData::from_bezpath(&path), appearance)).unwrap();
}

fn review_597185_vertical(d: &mut Document, x: f64) {
    review_597185_path(d, &[Point::new(x, -1000.0), Point::new(x, 2000.0)], Some(0.5));
}

fn review_597185_anchor(d: &mut Document, at: Point) {
    review_597185_path(d, &[at, Point::new(-20.0, 50.0), Point::new(-20.0, 60.0)], None);
}

fn review_597185_scale(d: &Document, on: bool, h: Handle, raw: Point) -> (Point, Vec<Overlay>) {
    let (selection, paint) = (Selection::default(), paint());
    let c = ToolContext { smart_guides: true, snap_to_stroke_edges: on, snapping_tolerance: 4.0, ..cx(d, &selection, &paint) };
    let r = Rect::new(0.0, 0.0, 10.0, if h == Handle::Right { 20.0 } else { 10.0 });
    let a = crate::bbox::scale_for_drag(r, h, raw, false, false);
    let (a, guides) = Targets::for_scale(&c).snap_scale(&OrientedBox::aligned(r), h, a, false, false, 4.0);
    (a * h.pos(r), guides)
}

fn review_597185_stroke_label(guides: &[Overlay]) -> bool {
    guides.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "stroke edge"))
}

#[test]
fn review_597185_distant_stroke_does_not_enable_plain_constrained_path() {
    let run = |triangle: bool, stroke: Option<f64>, on: bool| {
        let mut d = Document::new(2000.0, 1200.0);
        if triangle {
            review_597185_path(&mut d, &[Point::new(480.0, 200.0), Point::new(540.0, 500.0), Point::new(700.0, 200.0)], None);
        }
        if let Some(x) = stroke {
            review_597185_path(&mut d, &[Point::new(x, 100.0), Point::new(x, 400.0)], Some(20.0));
        }
        let (selection, paint) = (Selection::default(), paint());
        let c = ToolContext { smart_guides: true, snap_to_stroke_edges: on, snapping_tolerance: 4.0, ..cx(&d, &selection, &paint) };
        let leave = Leave::segment(&c, Point::new(400.0, 300.0), true);
        Targets::for_drawing(&d, &[], None, None).styled(&c).with_stroke_edges(&c, &[], None).snap_from(Point::new(498.0, 300.0), Some(&leave), 4.0)
    };
    // Ordinary OFF behavior, an empty painted channel, and a genuine in-reach painted edge.
    assert_eq!(run(true, Some(1100.0), false).0, Point::new(498.0, 300.0));
    assert_eq!(run(true, None, true).0, Point::new(498.0, 300.0));
    let (q, guides) = run(false, Some(510.0), true);
    assert!(q.distance(Point::new(500.0, 300.0)) < 1e-7 && review_597185_stroke_label(&guides), "{q:?}: {guides:?}");
    let (q, guides) = run(true, Some(1100.0), true);
    assert_eq!(q, Point::new(498.0, 300.0), "an unrelated painted stroke must not enable a plain-path intersection: {guides:?}");
}

#[test]
fn review_597185_bbox_constrained_tries_farther_feasible_stroke() {
    let raw = Point::new(0.5, 10.0);
    let mut valid = Document::new(2000.0, 1200.0);
    review_597185_vertical(&mut valid, 2.75);
    let (q, guides) = review_597185_scale(&valid, true, Handle::Right, raw);
    assert!((q.x - 2.5).abs() < 1e-7 && review_597185_stroke_label(&guides), "valid painted control: {q:?} {guides:?}");
    let mut anchor = valid.clone();
    review_597185_anchor(&mut anchor, Point::new(3.0, 10.0));
    let (q, guides) = review_597185_scale(&anchor, true, Handle::Right, raw);
    assert!((q.x - 3.0).abs() < 1e-7 && !review_597185_stroke_label(&guides), "valid anchor keeps priority: {q:?} {guides:?}");
    let mut pair = valid.clone();
    review_597185_vertical(&mut pair, -0.5);
    assert_eq!(review_597185_scale(&pair, false, Handle::Right, raw).0, raw, "OFF control");
    let (q, guides) = review_597185_scale(&pair, true, Handle::Right, raw);
    assert!(
        (q.x - 2.5).abs() < 1e-7 && review_597185_stroke_label(&guides),
        "the nearer flipping stroke must not hide a feasible contact: {q:?} {guides:?}"
    );
}

#[test]
fn review_597185_bbox_rejected_anchor_still_tries_stroke() {
    let raw = Point::new(0.5, 10.0);
    let mut valid = Document::new(2000.0, 1200.0);
    review_597185_vertical(&mut valid, 2.25);
    let (q, guides) = review_597185_scale(&valid, true, Handle::BottomRight, raw);
    assert!((q.x - 2.0).abs() < 1e-7 && review_597185_stroke_label(&guides), "valid painted control: {q:?} {guides:?}");
    let mut anchor = valid.clone();
    review_597185_anchor(&mut anchor, Point::new(3.0, 10.0));
    let (q, guides) = review_597185_scale(&anchor, true, Handle::BottomRight, raw);
    assert!((q.x - 3.0).abs() < 1e-7 && !review_597185_stroke_label(&guides), "valid anchor keeps priority: {q:?} {guides:?}");
    let mut pair = valid.clone();
    review_597185_anchor(&mut pair, Point::new(-0.5, 10.0));
    assert_eq!(review_597185_scale(&pair, false, Handle::BottomRight, raw).0, raw, "OFF control");
    let (q, guides) = review_597185_scale(&pair, true, Handle::BottomRight, raw);
    assert!(
        (q.x - 2.0).abs() < 1e-7 && review_597185_stroke_label(&guides),
        "a sign-invalid anchor must not suppress the painted channel: {q:?} {guides:?}"
    );
}

#[test]
fn review_597185_bbox_free_domain_uses_same_original_segment() {
    let mut d = Document::new(2000.0, 1200.0);
    let normal = Vec2::new(-2.0, 1.0) / 5.0f64.sqrt();
    // One actual painted edge is y=2x+11. Its unrestricted nearest point is
    // sign-invalid; the feasible interval belongs to that SAME original edge.
    review_597185_path(&mut d, &[Point::new(-10.0, -9.0) + normal * 20.0, Point::new(10.0, 31.0) + normal * 20.0], Some(40.0));
    let raw = Point::new(0.5, 10.0);
    assert_eq!(review_597185_scale(&d, false, Handle::BottomRight, raw).0, raw, "OFF control");
    let (q, guides) = review_597185_scale(&d, true, Handle::BottomRight, Point::new(1.0, 13.5));
    assert!(
        q.x > 0.0 && (q.y - (2.0 * q.x + 11.0)).abs() < 1e-6 && review_597185_stroke_label(&guides),
        "already-feasible painted control: {q:?} {guides:?}"
    );
    let (q, guides) = review_597185_scale(&d, true, Handle::BottomRight, raw);
    assert!(q.x > 0.0 && q.x < 5e-4 && (q.y - 11.0).abs() < 1e-3, "OPEN sign boundary without a new minimum-scale floor: {q:?} {guides:?}");
    assert!(
        (q.y - (2.0 * q.x + 11.0)).abs() < 1e-6 && review_597185_stroke_label(&guides),
        "contact must stay on the original painted edge: {q:?} {guides:?}"
    );
}

fn review_ruler_597368_nearest(path: &BezPath, p: Point) -> Point {
    path.segments()
        .map(|segment| segment.eval(segment.nearest(p, 1e-6).t))
        .min_by(|a, b| a.distance_squared(p).total_cmp(&b.distance_squared(p)))
        .expect("CONTROL: real painted glyph geometry exists")
}

fn review_ruler_597368_move_document(text: Option<&vectorcraft_doc::TextObject>, crossing: Option<Point>, legacy: bool) -> (Document, Selection) {
    use vectorcraft_doc::color::Paint;
    let mut d = Document::new(800.0, 600.0);
    let layer = d.layers[0].id;
    if let Some(text) = text {
        let id = d.alloc_id();
        d.insert(Some(layer), 0, Node::new(id, NodeKind::Text(Box::new(text.clone())))).unwrap();
    }
    if let Some(r) = crossing {
        d.add_ruler_guide(&vectorcraft_doc::Guide::new(true, r.x));
        d.add_ruler_guide(&vectorcraft_doc::Guide::new(false, r.y));
        if legacy {
            let id = d.alloc_id();
            let path = vectorcraft_geom::shapes::rectangle(Rect::new(r.x - 100.0, r.y - 0.5, r.x + 300.0, r.y + 100.0));
            d.insert(Some(layer), usize::MAX, Node::path(id, path, vectorcraft_doc::Appearance::basic(Paint::None, Paint::None, 0.0))).unwrap();
        }
    }
    let moving = d.alloc_id();
    let path = vectorcraft_geom::shapes::rectangle(Rect::new(50.0, 50.0, 70.0, 70.0));
    d.insert(Some(layer), usize::MAX, Node::path(moving, path, vectorcraft_doc::Appearance::default())).unwrap();
    let mut selection = Selection::default();
    selection.set([moving]);
    (d, selection)
}

fn review_ruler_597368_probe(band: &BezPath, ink: &BezPath, text: &vectorcraft_doc::TextObject, tol: f64) -> (Point, Point) {
    let (d, selected) = review_ruler_597368_move_document(Some(text), None, false);
    let legacy = Targets::collect(&d, &selected.objects, None);
    for segment in band.segments() {
        let PathSeg::Line(line) = segment else { continue };
        let tangent = line.p1 - line.p0;
        let length = tangent.hypot();
        if length <= 12.0 {
            continue;
        }
        let q = line.p0.midpoint(line.p1);
        let p = q + Vec2::new(-tangent.y, tangent.x) * (0.75 / length);
        if review_ruler_597368_nearest(band, p).distance(q) < 1e-6
            && legacy.snap_point(p, tol).0 == p
            && review_ruler_597368_nearest(ink, q).distance(q) > tol
        {
            return (p, q);
        }
    }
    panic!("CONTROL: fixed Source Sans 3 H supplies an isolated genuine stroke contact");
}

fn review_ruler_597368_move(
    text: Option<&vectorcraft_doc::TextObject>,
    on: bool,
    probe: Point,
    crossing: Option<Point>,
    legacy: bool,
) -> (Point, Vec<Overlay>) {
    let (d, selected) = review_ruler_597368_move_document(text, crossing, legacy);
    let paint = paint();
    let c = ToolContext {
        smart_guides: true,
        guides: true,
        snap_to_point: true,
        snap_to_grid: false,
        snap_to_pixel: false,
        snap_to_glyph: false,
        snap_to_last_location: false,
        spacing_guides: false,
        snap_to_stroke_edges: on,
        snapping_tolerance: 1.5,
        ..cx(&d, &selected, &paint)
    };
    let targets = Targets::for_move_by_anchor(&c, &selected.objects);
    assert_eq!(targets.segments.is_empty(), !legacy, "CONTROL: only actual legacy paths supply segments");
    assert!(!targets.has_point_near(probe, 1.5), "CONTROL: no old anchor/center preempts this contact");
    let original = d.clone();
    let start = Point::new(50.0, 50.0);
    let mut snap = crate::select::MoveSnap::by_anchor(&c);
    let (delta, guides) = snap.snap(&c, start, probe - start, false);
    assert_eq!(d, original, "CONTROL: real MoveSnap queries do not change source art");
    (start + delta, guides)
}

#[test]
fn review_597368_anchor_held_text_stroke_does_not_enable_a_ruler_only_crossing() {
    use vectorcraft_doc::color::{Color, Paint};
    use vectorcraft_doc::{CharStyle, TextObject};
    use vectorcraft_geom::{FillRule, PathData};
    let style = CharStyle {
        font_family: "Source Sans 3".into(),
        size: 160.0,
        fill: Paint::solid(Color::BLACK),
        stroke: Paint::solid(Color::rgb(1.0, 0.0, 0.0)),
        stroke_width: 8.0,
        ..CharStyle::default()
    };
    let mut text = TextObject::point(Point::ZERO, "H", style.clone());
    text.xf = Affine::new([1.5, 0.0, 0.0, 1.5, 150.0, 300.0]);
    let layout = layout_text(FontDb::global(), &text);
    assert_eq!(layout.glyphs.len(), 1, "CONTROL: one actual H glyph");
    let glyph = &layout.glyphs[0].outline;
    let ink = text.xf * glyph.clone();
    let band = vectorcraft_effects::stroke::outline_region(&PathData::from_bezpath(glyph), FillRule::NonZero, &style.stroke_layer())
        .transformed(text.xf)
        .to_bezpath();
    let (probe, contact) = review_ruler_597368_probe(&band, &ink, &text, 1.5);
    assert!((probe.distance(contact) - 0.75).abs() < 1e-6, "CONTROL: genuine nearby stroke");
    let crossing = probe + Vec2::new(1.0, 1.0);
    assert!(probe.distance(crossing) < 1.5 && contact.distance(crossing) > 0.25);
    let ordinary = review_ruler_597368_move(Some(&text), false, probe, Some(crossing), false);
    assert!(ordinary.0.distance(crossing) < 1e-8, "CONTROL: OFF keeps ordinary ruler snapping");
    let empty = review_ruler_597368_move(None, true, probe, Some(crossing), false);
    assert!(empty.0.distance(crossing) < 1e-8, "CONTROL: empty painted targets keep ordinary ruler snapping");
    let mut distant = text.clone();
    distant.xf = Affine::translate((300.0, 0.0)) * distant.xf;
    let remote = review_ruler_597368_move(Some(&distant), true, probe, Some(crossing), false);
    assert!(remote.0.distance(crossing) < 1e-8, "CONTROL: a distant stroke alone has the same ordinary result");
    let painted = review_ruler_597368_move(Some(&text), true, probe, None, false);
    assert!(painted.0.distance(contact) < 1e-6, "CONTROL: actual MoveSnap reaches the independent stroke contact");
    assert!(painted.1.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "stroke edge")));
    let paths = review_ruler_597368_move(Some(&text), true, probe, Some(crossing), true);
    let path_crossing = Point::new(crossing.x, crossing.y - 0.5);
    assert!(paths.0.distance(path_crossing) < 1e-8, "CONTROL: real legacy segments retain crossing priority");
    assert!(paths.0.distance(ordinary.0) > 0.25, "the actual path/ruler crossing differs from the ruler-only fallback");
    assert_eq!(labels(&paths.1), ["intersect"], "the real path/ruler crossing owns this independent control");
    let actual = review_ruler_597368_move(Some(&text), true, probe, Some(crossing), false);
    assert!(
        actual.0.distance(contact) < 1e-6 && actual.1.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "stroke edge")),
        "REGRESSION: a genuine text stroke, not a ruler-only crossing, must take the painted contact guard: {:?}, expected {contact:?}",
        actual
    );
}

fn review_597569_draw_document(edge: f64) -> Document {
    let mut d = Document::new(2000.0, 1200.0);
    review_597185_path(&mut d, &[Point::new(edge + 5.0, -1000.0), Point::new(edge + 5.0, 2000.0)], Some(10.0));
    d
}

fn review_597569_rectangle_drag(d: &Document, on: bool, raw: Point) -> serde_json::Value {
    use crate::{Action, Mods, PointerEvent, PointerKind, Tool};

    let original = d.clone();
    let (selection, paint) = (Selection::default(), paint());
    let c = ToolContext {
        smart_guides: true,
        snap_to_stroke_edges: on,
        snapping_tolerance: 4.0,
        zoom: 1.0,
        snap_to_grid: false,
        snap_to_pixel: false,
        constrain_angle: 0.0,
        ..cx(d, &selection, &paint)
    };
    let mut tool = crate::shape::ShapeTool::new("rectangle");
    let shift = Mods { shift: true, ..Mods::default() };
    tool.pointer(&c, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
    let actions = tool.pointer(&c, &PointerEvent::new(PointerKind::Drag, raw.x, raw.y).with_mods(shift));
    assert!(
        actions.iter().any(|action| matches!(action, Action::Begin(label) if label == "Rectangle")),
        "a real nonzero Shift drag must begin instead of snapping back below the click threshold: {actions:?}"
    );
    let value = actions
        .iter()
        .find_map(|action| match action {
            Action::Preview(id, value) if id == "shape.rectangle" => Some(value.clone()),
            _ => None,
        })
        .expect("the public Rectangle tool previews the dragged square");
    let (width, height) = (value["width"].as_f64().unwrap(), value["height"].as_f64().unwrap());
    assert!(width > 0.0 && height > 0.0 && (width - height).abs() < 1e-9, "a nonzero square preview: {value}");
    assert_eq!(value["x"], 100.0);
    assert_eq!(value["y"], 100.0);
    let released = tool.pointer(&c, &PointerEvent::new(PointerKind::Up, raw.x, raw.y).with_mods(shift));
    assert!(matches!(released.as_slice(), [Action::Commit]), "the actual drag commits once instead of opening its click-size dialog: {released:?}");
    assert!(!tool.busy());
    assert_eq!(d, &original, "the tool emits actions without mutating the source paint, geometry or resources");
    value
}

#[test]
fn review_597569_forward_stroke_contact_cannot_turn_a_shift_drag_into_a_click() {
    let d = review_597569_draw_document(100.0);
    let raw = Point::new(102.0, 102.0);
    // Ordinary drawing and a genuine forward painted contact must work first.
    review_597569_rectangle_drag(&d, false, raw);
    let forward = review_597569_draw_document(110.0);
    let value = review_597569_rectangle_drag(&forward, true, Point::new(108.0, 108.0));
    assert_eq!(value["width"], 10.0, "CONTROL: the positive-travel painted contact is110,110");
    assert_eq!(value["height"], 10.0);

    let (selection, paint) = (Selection::default(), paint());
    let c = ToolContext { smart_guides: true, snap_to_stroke_edges: true, snapping_tolerance: 4.0, ..cx(&d, &selection, &paint) };
    let targets = Targets::for_drawing(&d, &[], None, None).styled(&c).with_stroke_edges(&c, &[], None);
    let start = Point::new(100.0, 100.0);
    assert!(
        targets.nearest_point(start, 4.0).is_none() && targets.nearest_point(raw, 4.0).is_none(),
        "CONTROL: no anchor or center explains the painted hit"
    );
    let (same, guides) = targets.snap_point_along(start, Vec2::new(1.0, 1.0), 4.0);
    assert!(same.distance(start) < 1e-9, "CONTROL: a non-ray trajectory may retain its zero-travel contact");
    assert!(review_597185_stroke_label(&guides), "CONTROL: that zero contact belongs to the real painted edge");

    // Before accepts the ray's t=0 stroke contact and never emits Begin/Preview;
    // the public tool then treats Up as a click and opens its size dialog.
    review_597569_rectangle_drag(&d, true, raw);
}

fn review_597569_bbox_document(stroke: Option<f64>, anchors: &[Point]) -> Document {
    let mut d = Document::new(2000.0, 1200.0);
    if let Some(edge) = stroke {
        review_597185_vertical(&mut d, edge + 0.25);
    }
    for &at in anchors {
        review_597185_anchor(&mut d, at);
    }
    d
}

#[test]
fn review_597569_bbox_keeps_the_next_feasible_anchor_ahead_of_stroke() {
    let raw = Point::new(0.5, 10.0);
    let rejected = Point::new(-0.5, 10.0);
    let feasible = Point::new(2.0, 10.0);
    let both = [rejected, feasible];
    // Every existing priority/fallback control precedes either new regression.
    for handle in [Handle::BottomRight, Handle::Right] {
        let stroke = review_597569_bbox_document(Some(3.0), &[]);
        let (q, guides) = review_597185_scale(&stroke, true, handle, raw);
        assert!(
            q.distance(Point::new(3.0, 10.0)) < 1e-7 && review_597185_stroke_label(&guides),
            "CONTROL: genuine painted contact: {q:?} {guides:?}"
        );
        let valid = review_597569_bbox_document(Some(3.0), &[feasible]);
        let (q, guides) = review_597185_scale(&valid, true, handle, raw);
        assert!(
            q.distance(feasible) < 1e-7 && !review_597185_stroke_label(&guides),
            "CONTROL: a valid nearest anchor keeps priority: {q:?} {guides:?}"
        );
        let invalid = review_597569_bbox_document(Some(3.0), &[rejected]);
        let (q, guides) = review_597185_scale(&invalid, true, handle, raw);
        assert!(
            q.distance(Point::new(3.0, 10.0)) < 1e-7 && review_597185_stroke_label(&guides),
            "CONTROL: an invalid sole anchor still permits paint: {q:?} {guides:?}"
        );

        // Empty, distant-only and sign-invalid painted channels must retain the
        // original unrestricted legacy fallback, not broaden ordinary snapping.
        for edge in [None, Some(1000.0), Some(-2.0)] {
            let d = review_597569_bbox_document(edge, &both);
            let off = review_597185_scale(&d, false, handle, raw);
            let on = review_597185_scale(&d, true, handle, raw);
            assert_eq!(on.0, off.0, "CONTROL: no eligible paint preserves OFF geometry: {edge:?} {handle:?}");
            assert_eq!(labels(&on.1), labels(&off.1), "CONTROL: no eligible paint preserves the ordinary labels");
            assert!(!review_597185_stroke_label(&on.1));
        }
    }
    for handle in [Handle::BottomRight, Handle::Right] {
        let d = review_597569_bbox_document(Some(3.0), &both);
        let (q, guides) = review_597185_scale(&d, true, handle, raw);
        assert!(
            q.distance(feasible) < 1e-7,
            "REGRESSION: a rejected nearest anchor cannot hide the next feasible anchor behind stroke: {handle:?} {q:?} {guides:?}"
        );
        assert_eq!(labels(&guides), ["anchor"], "the real feasible anchor, not a stroke or axis alignment, owns the contact");
    }
}

fn review_597702_ray_document(edge: Option<f64>, anchors: &[Point]) -> Document {
    let mut d = Document::new(2000.0, 1200.0);
    if let Some(edge) = edge {
        review_597185_path(&mut d, &[Point::new(edge + 10.0, -1000.0), Point::new(edge + 10.0, 2000.0)], Some(20.0));
    }
    for &at in anchors {
        review_597185_anchor(&mut d, at);
    }
    d
}

fn review_597702_ray(d: &Document, on: bool, anchor: Option<Point>) -> (Point, Vec<Overlay>) {
    let (selection, paint) = (Selection::default(), paint());
    let c = ToolContext {
        smart_guides: true,
        snap_to_stroke_edges: on,
        snap_to_grid: false,
        snap_to_pixel: false,
        snapping_tolerance: 4.0,
        constrain_angle: 0.0,
        ..cx(d, &selection, &paint)
    };
    let targets = Targets::for_drawing(d, &[], None, None).styled(&c).with_stroke_edges(&c, &[], None);
    let raw = Point::new(102.0, 100.0);
    assert_eq!(
        targets.nearest_point(raw, 4.0).map(|(q, _)| q),
        anchor,
        "CONTROL: the actual document supplies only the intended nearby legacy point"
    );
    let leave = Leave::segment(&c, Point::new(100.0, 100.0), true);
    let before = d.clone();
    let result = targets.snap_from(raw, Some(&leave), 4.0);
    assert_eq!(d, &before, "drawing-target queries do not edit the source artwork");
    result
}

#[test]
fn review_597702_forward_paint_survives_an_unreachable_nearby_anchor() {
    let raw = Point::new(102.0, 100.0);
    let rejected = Point::new(99.0, 101.0);
    let forward = Point::new(103.0, 100.0);
    // The same ordinary forward constraint and valid legacy priority work first.
    for edge in [None, Some(104.0), Some(1000.0)] {
        let d = review_597702_ray_document(edge, &[rejected]);
        let off = review_597702_ray(&d, false, Some(rejected));
        assert_eq!(off.0, raw, "CONTROL: an unreachable anchor cannot supply the ordinary horizontal forward snap");
        if edge != Some(104.0) {
            let on = review_597702_ray(&d, true, Some(rejected));
            assert_eq!(on.0, off.0, "CONTROL: empty or distant-only paint keeps ordinary geometry");
            assert_eq!(labels(&on.1), labels(&off.1));
        }
    }
    let paint = review_597702_ray_document(Some(104.0), &[]);
    let painted = review_597702_ray(&paint, true, None);
    assert!(
        painted.0.distance(Point::new(104.0, 100.0)) < 1e-7 && review_597185_stroke_label(&painted.1),
        "CONTROL: real paint supplies the positive forward contact: {painted:?}"
    );
    let valid = review_597702_ray_document(Some(104.0), &[forward]);
    let ordinary = review_597702_ray(&valid, true, Some(forward));
    assert_eq!(ordinary.0, forward, "CONTROL: a reachable forward anchor keeps its existing priority");
    assert!(!review_597185_stroke_label(&ordinary.1));

    let d = review_597702_ray_document(Some(104.0), &[rejected]);
    let actual = review_597702_ray(&d, true, Some(rejected));
    assert!(
        actual.0.distance(Point::new(104.0, 100.0)) < 1e-7 && review_597185_stroke_label(&actual.1),
        "REGRESSION: a nearby anchor behind the origin and off the forward line cannot hide its reachable painted contact: {actual:?}"
    );
    // A distant real anchor supplies an alignment, but is not a nearby point target.
    let axis = Point::new(103.0, 500.0);
    let aligned = review_597702_ray_document(Some(104.0), &[axis]);
    let off = review_597702_ray(&aligned, false, None);
    assert_eq!(off.0, Point::new(103.0, 100.0), "CONTROL: the ordinary forward route reaches the distant anchor's vertical alignment");
    let without_rejected = review_597702_ray(&aligned, true, None);
    assert!(
        without_rejected.0.distance(Point::new(104.0, 100.0)) < 1e-7 && review_597185_stroke_label(&without_rejected.1),
        "CONTROL: eligible paint precedes an unrelated alignment when there is no nearby point: {without_rejected:?}"
    );
    let aligned_and_rejected = review_597702_ray_document(Some(104.0), &[axis, rejected]);
    let actual = review_597702_ray(&aligned_and_rejected, true, Some(rejected));
    assert!(
        actual.0.distance(Point::new(104.0, 100.0)) < 1e-7 && review_597185_stroke_label(&actual.1),
        "REGRESSION: adding an unusable nearby anchor cannot let an unrelated forward alignment suppress paint: {actual:?}"
    );

    let near_invalid = Point::new(99.5, 100.5);
    let near_valid = Point::new(103.0, 100.0);
    let single = review_597702_ray_document(Some(104.0), &[near_valid]);
    assert_eq!(review_597942_ray(&single, true, Some(near_valid)).0, near_valid, "CONTROL: the reachable nearby anchor keeps forward priority");
    let both = review_597702_ray_document(Some(104.0), &[near_invalid, near_valid]);
    let off = review_597942_ray(&both, false, Some(near_invalid));
    assert_eq!(off.0, near_valid, "CONTROL: the ordinary forward route finds the farther usable nearby anchor");
    let on = review_597942_ray(&both, true, Some(near_invalid));
    assert_eq!(on.0, near_valid, "REGRESSION: an invalid nearest anchor cannot hide another nearby anchor's usable forward alignment");
    assert!(!review_597185_stroke_label(&on.1), "the reachable legacy anchor, not farther paint, owns the contact");
}

fn review_597702_bbox_document(center: Option<f64>, anchors: &[Point]) -> Document {
    let mut d = Document::new(2000.0, 1200.0);
    if let Some(center) = center {
        review_597185_path(&mut d, &[Point::new(center, 19.0), Point::new(center, 21.0)], Some(0.5));
    }
    for &at in anchors {
        review_597185_anchor(&mut d, at);
    }
    d
}

fn review_597702_bbox_endpoint_paint(d: &Document) {
    let (selection, paint) = (Selection::default(), paint());
    let c = ToolContext { smart_guides: true, snap_to_stroke_edges: true, snapping_tolerance: 4.0, ..cx(d, &selection, &paint) };
    let targets = Targets::for_scale(&c);
    let dir = Vec2::new(10.0, 0.0);
    for y in [10.0, 0.0] {
        assert!(
            targets.stroke_contours.on_line(Point::new(0.0, y), dir, Point::new(0.5, y), 4.0, false).is_none(),
            "CONTROL: neither the grabbed handle nor its top endpoint has a painted contact"
        );
    }
    let contact = targets.stroke_contours.on_line(Point::new(0.0, 20.0), dir, Point::new(0.5, 20.0), 4.0, false).unwrap();
    assert!(contact.distance(Point::new(3.0, 20.0)) < 1e-7, "CONTROL: only the bottom trajectory reaches the genuine painted band: {contact:?}");
}

#[test]
fn review_597702_bbox_uses_feasible_anchor_when_only_endpoint_has_paint() {
    let raw = Point::new(0.5, 10.0);
    let rejected = Point::new(-0.5, 10.0);
    let feasible = Point::new(2.0, 10.0);
    let both = [rejected, feasible];
    // No eligible paint keeps the full original fallback, including its labels.
    for center in [None, Some(1000.0), Some(-3.25)] {
        let d = review_597702_bbox_document(center, &both);
        let off = review_597185_scale(&d, false, Handle::Right, raw);
        let on = review_597185_scale(&d, true, Handle::Right, raw);
        assert_eq!(on.0, off.0, "CONTROL: empty, remote and sign-invalid paint leave OFF geometry unchanged: {center:?}");
        assert_eq!(labels(&on.1), labels(&off.1));
        assert!(!review_597185_stroke_label(&on.1));
    }
    let painted_document = review_597702_bbox_document(Some(3.25), &[]);
    review_597702_bbox_endpoint_paint(&painted_document);
    let ordinary = review_597185_scale(&painted_document, true, Handle::Right, raw);
    assert!(
        ordinary.0.distance(Point::new(3.25, 10.0)) < 1e-7,
        "CONTROL: this real short stroke also supplies its ordinary feasible legacy center: {ordinary:?}"
    );
    let valid = review_597702_bbox_document(Some(3.25), &[feasible]);
    let ordinary = review_597185_scale(&valid, true, Handle::Right, raw);
    assert_eq!(ordinary.0, feasible, "CONTROL: an already-nearest feasible handle anchor has priority");
    assert_eq!(labels(&ordinary.1), ["anchor"]);

    let d = review_597702_bbox_document(Some(3.25), &both);
    review_597702_bbox_endpoint_paint(&d);
    let (selection, paint) = (Selection::default(), paint());
    let c = ToolContext { smart_guides: true, snap_to_stroke_edges: true, snapping_tolerance: 4.0, ..cx(&d, &selection, &paint) };
    assert_eq!(
        Targets::for_scale(&c).nearest_point(raw, 4.0).map(|(q, _)| q),
        Some(rejected),
        "CONTROL: the raw nearest handle anchor is the sign-invalid one"
    );
    let actual = review_597185_scale(&d, true, Handle::Right, raw);
    assert!(
        actual.0.distance(feasible) < 1e-7,
        "REGRESSION: paint on another trajectory enables the nearer feasible handle anchor before endpoint contacts: {actual:?}"
    );
    assert_eq!(labels(&actual.1), ["anchor"], "the feasible handle anchor owns the contact independently of an equal-coordinate axis fallback");
}

fn review_597702_move_document(stroke: Option<(f64, f64)>, ruler: Option<f64>, anchor: Option<Point>) -> (Document, Selection) {
    let mut d = Document::new(800.0, 600.0);
    if let Some((center, width)) = stroke {
        review_597185_path(&mut d, &[Point::new(center, -1000.0), Point::new(center, 2000.0)], Some(width));
    }
    if let Some(x) = ruler {
        d.add_ruler_guide(&vectorcraft_doc::Guide::new(true, x));
    }
    if let Some(at) = anchor {
        review_597185_anchor(&mut d, at);
    }
    let moving = d.alloc_id();
    let path = vectorcraft_geom::shapes::rectangle(Rect::new(50.0, 150.0, 70.0, 170.0));
    d.insert(Some(d.layers[0].id), usize::MAX, Node::path(moving, path, vectorcraft_doc::Appearance::default())).unwrap();
    let mut selection = Selection::default();
    selection.set([moving]);
    (d, selection)
}

fn review_597702_shift_move(stroke: Option<(f64, f64)>, ruler: Option<f64>, anchor: Option<Point>, on: bool, edge: bool) -> (Point, Vec<Overlay>) {
    review_597702_shift_move_display(stroke, ruler, anchor, on, edge, true)
}

fn review_597702_shift_move_display(
    stroke: Option<(f64, f64)>,
    ruler: Option<f64>,
    anchor: Option<Point>,
    on: bool,
    edge: bool,
    show_guides: bool,
) -> (Point, Vec<Overlay>) {
    let (d, selection) = review_597702_move_document(stroke, ruler, anchor);
    let paint = paint();
    let mut c = ToolContext {
        smart_guides: true,
        guides: true,
        snap_to_point: true,
        snap_to_grid: false,
        snap_to_pixel: false,
        snap_to_glyph: false,
        snap_to_last_location: false,
        spacing_guides: false,
        snap_to_stroke_edges: on,
        snapping_tolerance: 4.0,
        ..cx(&d, &selection, &paint)
    };
    if !show_guides {
        c.alignment_guides = false;
        c.anchor_path_labels = false;
    }
    let raw = Point::new(200.0, 150.0);
    let targets = Targets::for_move_by_anchor(&c, &selection.objects);
    assert_eq!(targets.has_point_near(raw, 4.0), anchor.is_some(), "CONTROL: distant stroke endpoints and center introduce no nearby legacy anchor");
    let start = Point::new(if edge { 60.0 } else { 50.0 }, 150.0);
    let mut snap = if edge { crate::select::MoveSnap::by_edge(&c) } else { crate::select::MoveSnap::by_anchor(&c) };
    let original = d.clone();
    let (delta, guides) = snap.snap_along(&c, start, raw - start, false);
    assert_eq!(d, original, "the actual selected-anchor/edge constrained query leaves the source document intact");
    (start + delta, guides)
}

#[test]
fn review_597702_shift_move_keeps_a_closer_ruler_than_a_painted_contact() {
    // Both public gather modes use an actual point on the selected rectangle.
    for edge in [false, true] {
        for center in [None, Some(213.0), Some(1000.0)] {
            let off = review_597702_shift_move(center.map(|x| (x, 20.0)), Some(201.0), None, false, edge);
            assert_eq!(
                off.0,
                Point::new(201.0, 150.0),
                "CONTROL: ordinary OFF alignment followed by the shown ruler wins: edge={edge}, center={center:?}"
            );
            if center != Some(213.0) {
                let on = review_597702_shift_move(center.map(|x| (x, 20.0)), Some(201.0), None, true, edge);
                assert_eq!(on.0, off.0, "CONTROL: empty or remote-only paint preserves the ordinary ruler route");
                assert_eq!(labels(&on.1), labels(&off.1));
            }
        }
        let painted = review_597702_shift_move(Some((213.0, 20.0)), None, None, true, edge);
        assert_eq!(painted.0, Point::new(203.0, 150.0), "CONTROL: with no ruler the genuine painted contact remains");
        assert!(review_597185_stroke_label(&painted.1));
        let closer = review_597702_shift_move(Some((211.0, 20.0)), Some(203.0), None, true, edge);
        assert_eq!(closer.0, Point::new(201.0, 150.0), "CONTROL: a closer painted contact stays ahead of the farther ruler");
        assert!(review_597185_stroke_label(&closer.1));
        let legacy = review_597702_shift_move(Some((213.0, 20.0)), None, Some(Point::new(202.0, 150.0)), true, edge);
        assert_eq!(legacy.0, Point::new(202.0, 150.0), "CONTROL: an ordinary usable legacy anchor still wins");
        assert!(!review_597185_stroke_label(&legacy.1));
        let zero = review_597702_shift_move(Some((233.0, 60.0)), Some(200.0), None, false, edge);
        assert_eq!(zero.0, Point::new(200.0, 150.0), "CONTROL: the shown ruler already owns the raw constrained point");
        assert!(
            !zero.1.is_empty() && !review_597185_stroke_label(&zero.1),
            "CONTROL: a real zero-displacement ruler contact has ordinary guide evidence: {zero:?}"
        );
        // Display switches hide overlays, not the shown ruler's mathematical contact.
        for ruler in [200.0, 201.0] {
            let (center, width) = if ruler == 200.0 { (233.0, 60.0) } else { (213.0, 20.0) };
            let off = review_597702_shift_move_display(Some((center, width)), Some(ruler), None, false, edge, false);
            assert_eq!(off.0, Point::new(ruler, 150.0), "CONTROL: hidden guide graphics preserve the OFF ruler contact: edge={edge}, ruler={ruler}");
            assert!(off.1.is_empty(), "CONTROL: both display options suppress the ordinary ruler overlays");
        }
        let hidden_paint = review_597702_shift_move_display(Some((213.0, 20.0)), None, None, true, edge, false);
        assert_eq!(hidden_paint.0, Point::new(203.0, 150.0), "CONTROL: hiding guide graphics keeps the real no-ruler painted contact");
        assert!(hidden_paint.1.is_empty());
        let hidden_closer = review_597702_shift_move_display(Some((211.0, 20.0)), Some(203.0), None, true, edge, false);
        assert_eq!(hidden_closer.0, Point::new(201.0, 150.0), "CONTROL: hidden graphics retain nearer paint ahead of a farther ruler");
        assert!(hidden_closer.1.is_empty());
    }
    let observed: Vec<_> = [false, true]
        .into_iter()
        .map(|edge| {
            let moved = review_597702_shift_move(Some((213.0, 20.0)), Some(201.0), None, true, edge);
            let zero = review_597702_shift_move(Some((233.0, 60.0)), Some(200.0), None, true, edge);
            (edge, moved, zero)
        })
        .collect();
    assert!(
        observed.iter().all(|(_, moved, zero)| moved.0 == Point::new(201.0, 150.0)
            && !review_597185_stroke_label(&moved.1)
            && zero.0 == Point::new(200.0, 150.0)
            && !zero.1.is_empty()
            && !review_597185_stroke_label(&zero.1)),
        "REGRESSION: moved and zero-displacement closer ruler contacts must both survive farther genuine paint: {observed:?}"
    );

    let hidden_observed: Vec<_> = [false, true]
        .into_iter()
        .map(|edge| {
            let moved = review_597702_shift_move_display(Some((213.0, 20.0)), Some(201.0), None, true, edge, false);
            let zero = review_597702_shift_move_display(Some((233.0, 60.0)), Some(200.0), None, true, edge, false);
            (edge, moved, zero)
        })
        .collect();
    assert!(
        hidden_observed.iter().all(|(_, moved, zero)| moved.0 == Point::new(201.0, 150.0)
            && moved.1.is_empty()
            && zero.0 == Point::new(200.0, 150.0)
            && zero.1.is_empty()),
        "REGRESSION: the same moved and zero-displacement ruler contacts must win with both guide-display options off: {hidden_observed:?}"
    );
}

fn review_597942_ray(d: &Document, on: bool, nearest: Option<Point>) -> (Point, Vec<Overlay>) {
    let (selection, paint) = (Selection::default(), paint());
    let c = ToolContext {
        smart_guides: true,
        snap_to_stroke_edges: on,
        snap_to_grid: false,
        snap_to_pixel: false,
        snapping_tolerance: 4.0,
        constrain_angle: 0.0,
        ..cx(d, &selection, &paint)
    };
    let targets = Targets::for_drawing(d, &[], None, None).styled(&c).with_stroke_edges(&c, &[], None);
    let raw = Point::new(101.0, 100.0);
    assert_eq!(targets.nearest_point(raw, 4.0).map(|(q, _)| q), nearest, "CONTROL: the rejected point really is nearest to this raw pointer");
    let leave = Leave::segment(&c, Point::new(100.0, 100.0), true);
    let original = d.clone();
    let result = targets.snap_from(raw, Some(&leave), 4.0);
    assert_eq!(d, &original);
    result
}

fn review_597942_bounds_move(edge: Option<f64>, ruler: Option<f64>, on: bool, shift: bool, show: bool) -> (Point, Vec<Overlay>) {
    let (d, selection) = review_597702_move_document(edge.map(|edge| (edge + 10.0, 20.0)), ruler, None);
    let paint = paint();
    let c = ToolContext {
        smart_guides: true,
        guides: true,
        snap_to_point: true,
        snap_to_grid: false,
        snap_to_pixel: false,
        snap_to_glyph: false,
        snap_to_last_location: false,
        spacing_guides: false,
        snap_to_stroke_edges: on,
        snapping_tolerance: 4.0,
        alignment_guides: show,
        anchor_path_labels: show,
        ..cx(&d, &selection, &paint)
    };
    let raw = Point::new(190.0, 150.0);
    let start = Point::new(60.0, 150.0);
    let targets = Targets::for_move(&c, &selection.objects);
    assert!(!targets.has_point_near(raw, 4.0), "CONTROL: the ordinary body grab is not near an anchor");
    assert!(targets.stroke_contours.nearest(raw, 4.0).is_none(), "CONTROL: the grab cannot contact the paint directly");
    let mut snap = crate::select::MoveSnap::new(&c);
    let original = d.clone();
    let (delta, guides) = if shift { snap.snap_along(&c, start, raw - start, false) } else { snap.snap(&c, start, raw - start, false) };
    assert_eq!(d, original, "actual body-move snapping does not mutate the document or selected artwork");
    (start + delta, guides)
}

#[test]
fn review_597942_body_bounds_preserve_the_ordinary_ruler_priority() {
    for shift in [false, true] {
        for edge in [None, Some(203.0), Some(1003.0)] {
            let off = review_597942_bounds_move(edge, Some(191.0), false, shift, true);
            assert_eq!(off.0, Point::new(191.0, 150.0), "CONTROL: the actual OFF route holds the grab on the shown ruler");
            if edge != Some(203.0) {
                let on = review_597942_bounds_move(edge, Some(191.0), true, shift, true);
                assert_eq!(on.0, off.0, "CONTROL: no eligible paint keeps the ordinary route");
            }
        }
        let paint = review_597942_bounds_move(Some(203.0), None, true, shift, true);
        assert!(paint.0.distance(Point::new(193.0, 150.0)) < 1e-9, "CONTROL: paint moves the right bounds edge from200 to203: {paint:?}");
        assert!(review_597185_stroke_label(&paint.1));
    }
    let closer = review_597942_bounds_move(Some(201.0), Some(193.0), true, true, true);
    assert!(closer.0.distance(Point::new(191.0, 150.0)) < 1e-9, "CONTROL: Shift keeps the closer painted bounds contact: {closer:?}");
    assert!(review_597185_stroke_label(&closer.1));
    let tie = review_597942_bounds_move(Some(203.0), Some(193.0), true, true, true);
    assert!(tie.0.distance(Point::new(193.0, 150.0)) < 1e-9, "CONTROL: equal travel keeps the painted point: {tie:?}");
    assert!(review_597185_stroke_label(&tie.1), "CONTROL: equal travel retains the complete painted contact");
    let hidden_paint = review_597942_bounds_move(Some(203.0), None, true, true, false);
    assert!(hidden_paint.0.distance(Point::new(193.0, 150.0)) < 1e-9, "CONTROL: hidden guides keep the painted point: {hidden_paint:?}");
    assert!(hidden_paint.1.is_empty(), "CONTROL: display switches hide guides without disabling geometry");
    for shift in [false, true] {
        for show in [true, false] {
            let actual = review_597942_bounds_move(Some(203.0), Some(191.0), true, shift, show);
            assert_eq!(
                actual.0,
                Point::new(191.0, 150.0),
                "REGRESSION: an eligible bounds contact must not hide the ordinary closer held-grab ruler: shift={shift}, show={show}"
            );
            assert!(!review_597185_stroke_label(&actual.1));
            if !show {
                assert!(actual.1.is_empty());
            }
        }
    }
    let free = review_597942_bounds_move(Some(201.0), Some(193.0), true, false, true);
    assert_eq!(free.0, Point::new(193.0, 150.0), "REGRESSION: free movement preserves held-grab ruler priority even when bounds paint is closer");
    assert!(!review_597185_stroke_label(&free.1));
}

/// A compound shape's fill becomes an Outline stroke through its actual Pathfinder style
/// source. Query-only provenance must survive that source without changing ordinary artwork.
#[test]
fn stroke_review_compound_outline_fill_origin() {
    use vectorcraft_doc::color::{Color, Paint};
    use vectorcraft_doc::{Appearance, Effect, QueryPaintKind};
    use vectorcraft_geom::{FillRule, shapes};

    fn document(compound: bool) -> (Document, NodeId, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let (layer, owner, child, second) = (d.layers[0].id, d.alloc_id(), d.alloc_id(), d.alloc_id());
        let filled = Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 1.0);
        let rect = |id, bounds, appearance| Arc::new(Node::path(id, shapes::rectangle(bounds), appearance));
        let children = if compound {
            vec![
                rect(child, Rect::new(100.0, 100.0, 175.0, 200.0), Appearance::default()),
                rect(second, Rect::new(150.0, 100.0, 200.0, 200.0), Appearance::default()),
            ]
        } else {
            vec![rect(child, Rect::new(100.0, 100.0, 200.0, 200.0), filled.clone())]
        };
        let mut group = Node::group(owner, children);
        if compound {
            group.extras_mut().compound_shape = true;
            group.appearance = filled.into();
        }
        group.appearance.effects.push(Effect { id: "pathfinder.outline".into(), params: serde_json::json!({}), visible: true });
        d.insert(Some(layer), 0, group).unwrap();
        (d, owner, child)
    }

    let probe = Point::new(202.0, 140.0);
    for compound in [false, true] {
        let (d, owner, child) = document(compound);
        let original = d.clone();
        assert_eq!(d.node(owner).unwrap().is_compound_shape(), compound);
        // Independently resolve the ordinary painted result first: a missing target must not
        // be confused with a hidden or unpainted Outline result.
        let art = vectorcraft_effects::pathfinder_children(Some(&d), d.node(owner).unwrap(), None).unwrap();
        assert!(!art.is_empty(), "CONTROL: Outline produces real artwork");
        let contact = art
            .iter()
            .filter_map(|n| {
                assert!(n.extras().query_origin.0.is_none(), "ordinary rendering invents no query metadata");
                let path = n.path_data().expect("Outline returns path geometry");
                let stroke = n.appearance.stroke().expect("Outline returns a stroke");
                assert!(stroke.visible && !stroke.paint.is_none() && stroke.width == 1.0, "CONTROL: the result really paints its one-point stroke");
                let band = vectorcraft_effects::stroke::outline_region(path, FillRule::NonZero, stroke).to_bezpath();
                band.segments()
                    .map(|s| s.eval(s.nearest(probe, 1e-6).t))
                    .min_by(|a, b| a.distance_squared(probe).total_cmp(&b.distance_squared(probe)))
            })
            .min_by(|a, b| a.distance_squared(probe).total_cmp(&b.distance_squared(probe)))
            .unwrap();
        assert!(contact.distance(Point::new(200.5, 140.0)) < 1e-6, "CONTROL: the actual painted contour is independently resolved: {contact:?}");
        let (selection, paint) = (Selection::default(), paint());
        let on = ToolContext { smart_guides: true, snap_to_stroke_edges: true, snapping_tolerance: 4.0, ..cx(&d, &selection, &paint) };
        let off = ToolContext { snap_to_stroke_edges: false, ..on };
        assert!(stroke::gather(&off, &[], None, None).nearest(probe, 4.0).is_none(), "CONTROL: the disabled option emits no stroke targets");
        assert!(stroke::gather(&on, &[owner], None, None).nearest(probe, 4.0).is_none(), "CONTROL: selected owner is excluded");
        assert!(
            stroke::gather(&on, &[child], None, None).nearest(probe, 4.0).is_none(),
            "CONTROL: a selected child excludes dependent container geometry"
        );
        let roots = [owner];
        let actual = stroke::gather(&on, &[], None, Some(&roots)).nearest(probe, 4.0);
        assert!(
            actual.is_some_and(|q| q.distance(contact) < 1e-6),
            "REGRESSION: compound={compound}, target {actual:?} must match the visible Outline contour {contact:?}"
        );
        if compound {
            let view = vectorcraft_effects::stroke_paint_view(&d);
            let mut strokes = 0;
            for layer in &view.layers {
                layer.walk(&mut |n| {
                    if n.appearance.stroke().is_some_and(|s| s.visible && !s.paint.is_none()) {
                        let origin = n.extras().query_origin.0.expect("the Outline child retains its actual style owner's query origin");
                        assert_eq!((origin.geometry_source, origin.paint_owner, origin.kind), (owner, owner, QueryPaintKind::Geometry));
                        strokes += 1;
                    }
                });
            }
            assert!(strokes > 0, "the provenance oracle reaches actual Outline strokes");
        }
        assert_eq!(d, original, "query gathering leaves source art and its serialized state unchanged");
        for layer in &d.layers {
            layer.walk(&mut |n| assert!(n.extras().query_origin.0.is_none(), "source nodes keep no transient query metadata"));
        }
    }
}
