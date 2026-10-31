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
    d.guides.push(vectorcraft_doc::Guide::new(true, 400.0));
    d.guides.push(vectorcraft_doc::Guide::new(false, 300.0));
    d.guides.push(vectorcraft_doc::Guide::new(false, 150.0));
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
    d.guides.push(vectorcraft_doc::Guide::new(false, 313.0));
    d.guides.push(vectorcraft_doc::Guide::new(true, 331.0));
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
        (GlyphSnap { bounds: false, ..all }, [4, 4, 0, 0]),
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
    // Rotated type: its baseline, x-height and width edges and its strokes run at an angle, all
    // Angular Guides; its glyph bounds still run across and down the page.
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
