use super::*;
use crate::testutil::*;
use vectorcraft_doc::Selection;

/// While an opacity mask is edited, a press on a selected Shaper original shape moves it, over the
/// art the group shows and over a part of the shape the group does not show. A press on an original
/// shape that is not selected selects the Shaper Group or starts a marquee, as before.
#[test]
fn a_press_on_a_selected_shaper_source_moves_it_while_a_mask_is_edited() {
    let (mut d, g, sources, _) = doc_with_shaper_group();
    let rect = d.layers[0].children().unwrap()[0].id;
    let layer = d.layers[0].id;
    d.mask_edit = Some(vectorcraft_doc::MaskEdit { object: rect, layer });
    let ids: Vec<NodeId> = d.node(sources).unwrap().children().unwrap().iter().map(|n| n.id).collect();
    let (mut s, p) = (Selection::default(), paint());
    s.set([ids[1]]);
    let c = ToolContext { isolation: Some(layer), ..cx(&d, &s, &p) };
    for x in [75.0, 125.0] {
        let mut t = SelectionTool::default();
        assert_eq!(t.cursor(&c, Point::new(x, 350.0), Mods::default()), Cursor::Move, "at {x}");
        let down = t.pointer(&c, &ev(PointerKind::Down, x, 350.0));
        assert!(!down.iter().any(|a| matches!(a, Action::Exec(c, _) if c == "select.set")), "at {x}: {down:?}");
        let drag = t.pointer(&c, &ev(PointerKind::Drag, x + 20.0, 350.0));
        assert!(drag.iter().any(|a| matches!(a, Action::Preview(c, _) if c == "object.transform")), "at {x}: {drag:?}");
    }
    s.set([ids[0]]);
    let c = ToolContext { isolation: Some(layer), ..cx(&d, &s, &p) };
    let down = SelectionTool::default().pointer(&c, &ev(PointerKind::Down, 125.0, 350.0));
    assert!(down.is_empty(), "{down:?}");
    let down = SelectionTool::default().pointer(&c, &ev(PointerKind::Down, 75.0, 350.0));
    assert!(down.iter().any(|a| matches!(a, Action::Exec(c, v) if c == "select.set" && v["ids"] == json!([g.0]))), "{down:?}");
}

/// While an opacity mask is edited, a selected Shaper original shape that is not the first selected
/// object still moves on a press, and an object in front of the Shaper Group keeps the press.
#[test]
fn a_selected_shaper_source_moves_after_other_objects_and_behind_front_art() {
    use vectorcraft_doc::{Appearance, Node};
    let (mut d, _, sources, _) = doc_with_shaper_group();
    let rect = d.layers[0].children().unwrap()[0].id;
    let layer = d.layers[0].id;
    d.mask_edit = Some(vectorcraft_doc::MaskEdit { object: rect, layer });
    let ids: Vec<NodeId> = d.node(sources).unwrap().children().unwrap().iter().map(|n| n.id).collect();
    let (mut s, p) = (Selection::default(), paint());
    s.set([rect, ids[1]]);
    let c = ToolContext { isolation: Some(layer), ..cx(&d, &s, &p) };
    let mut t = SelectionTool::default();
    assert_eq!(t.cursor(&c, Point::new(125.0, 350.0), Mods::default()), Cursor::Move);
    let down = t.pointer(&c, &ev(PointerKind::Down, 125.0, 350.0));
    assert!(!down.iter().any(|a| matches!(a, Action::Exec(c, _) if c == "select.set")), "{down:?}");
    let drag = t.pointer(&c, &ev(PointerKind::Drag, 145.0, 350.0));
    assert!(drag.iter().any(|a| matches!(a, Action::Preview(c, _) if c == "object.transform")), "{drag:?}");
    let front = d.alloc_id();
    let square = vectorcraft_geom::shapes::rectangle(Rect::new(110.0, 340.0, 140.0, 360.0));
    d.insert(Some(layer), 2, Node::path(front, square, Appearance::default_art())).unwrap();
    s.set([ids[1]]);
    let c = ToolContext { isolation: Some(layer), ..cx(&d, &s, &p) };
    assert_ne!(SelectionTool::default().cursor(&c, Point::new(125.0, 350.0), Mods::default()), Cursor::Move);
    let down = SelectionTool::default().pointer(&c, &ev(PointerKind::Down, 125.0, 350.0));
    assert!(down.iter().any(|a| matches!(a, Action::Exec(c, v) if c == "select.set" && v["ids"] == json!([front.0]))), "{down:?}");
}

fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
    PointerEvent::new(kind, x, y)
}

/// A Selection tool press on a clip group's mask edge, where the art it clips lies over it,
/// selects the clip group, as a press on the art inside does.
#[test]
fn a_press_on_a_masks_edge_selects_its_clip_group() {
    let (d, g, _, _) = doc_with_masked_square();
    let (s, p) = (Selection::default(), paint());
    for x in [500.0, 501.5, 450.0] {
        let a = SelectionTool::default().pointer(&cx(&d, &s, &p), &ev(PointerKind::Down, x, 450.0));
        assert!(a.iter().any(|a| matches!(a, Action::Exec(c, v) if c == "select.set" && v["ids"] == json!([g.0]))), "at {x}: {a:?}");
    }
    // Just outside the mask, beyond the tolerance, the clipped-away art isn't there.
    let a = SelectionTool::default().pointer(&cx(&d, &s, &p), &ev(PointerKind::Down, 520.0, 450.0));
    assert!(!a.iter().any(|a| matches!(a, Action::Exec(c, _) if c == "select.set")), "{a:?}");
}

/// The art of a selected clipped object (picked in the Layers panel), pressed where its mask
/// hides it, is grabbed: a drag moves the selection (the art, not its clip group) and a click
/// keeps it selected. With nothing of it selected, or with Shift, a press there starts a marquee.
#[test]
fn a_press_where_a_mask_hides_a_selected_object_drags_it() {
    let (d, _, _, art) = doc_with_masked_square();
    let p = paint();
    let mut picked = Selection::default();
    picked.add(art);
    let shift = |kind, x, y| {
        let mut e = ev(kind, x, y);
        e.mods.shift = true;
        e
    };
    let changes_selection = |a: &[Action]| a.iter().any(|a| matches!(a, Action::Exec(c, _) if c.starts_with("select.")));
    // (370, 450): on the art, outside the mask (400..500).
    let mut t = SelectionTool::default();
    let c = cx(&d, &picked, &p);
    assert!(t.pointer(&c, &ev(PointerKind::Down, 370.0, 450.0)).is_empty());
    let a = t.pointer(&c, &ev(PointerKind::Drag, 390.0, 470.0));
    assert!(a.iter().any(|a| matches!(a, Action::Begin(n) if n == "Move")), "{a:?}");
    assert!(a.iter().any(|a| matches!(a, Action::Preview(c, _) if c == "object.transform")), "{a:?}");
    assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 390.0, 470.0)), vec![Action::Commit]);
    // A click there keeps the selection.
    let mut t = SelectionTool::default();
    t.pointer(&c, &ev(PointerKind::Down, 370.0, 450.0));
    assert!(!changes_selection(&t.pointer(&c, &ev(PointerKind::Up, 370.0, 450.0))));
    // Nothing selected: a marquee, and a click deselects.
    let none = Selection::default();
    let c = cx(&d, &none, &p);
    let mut t = SelectionTool::default();
    t.pointer(&c, &ev(PointerKind::Down, 370.0, 450.0));
    assert!(!t.pointer(&c, &ev(PointerKind::Drag, 390.0, 470.0)).iter().any(|a| matches!(a, Action::Begin(_))));
    // Shift: a marquee that adds to the selection.
    let c = cx(&d, &picked, &p);
    let mut t = SelectionTool::default();
    t.pointer(&c, &shift(PointerKind::Down, 370.0, 450.0));
    assert!(!t.pointer(&c, &shift(PointerKind::Drag, 390.0, 470.0)).iter().any(|a| matches!(a, Action::Begin(_))));
    // An object a click hits there wins: a press on a square over that point selects it.
    let mut d = d;
    let (front, l) = (d.alloc_id(), d.layers[0].id);
    let square = vectorcraft_geom::shapes::rectangle(Rect::new(350.0, 430.0, 390.0, 470.0));
    d.insert(Some(l), 2, vectorcraft_doc::Node::path(front, square, vectorcraft_doc::Appearance::default_art())).unwrap();
    let a = SelectionTool::default().pointer(&cx(&d, &picked, &p), &ev(PointerKind::Down, 370.0, 450.0));
    assert!(a.iter().any(|a| matches!(a, Action::Exec(c, v) if c == "select.set" && v["ids"] == json!([front.0]))), "{a:?}");
}

/// #955: grabbed by an anchor and dropped near another object's anchor, the anchor lands
/// exactly on it with Smart Guides, even where lining the bounds up would leave it beside it.
#[test]
fn a_move_grabbed_by_an_anchor_lands_it_on_another_anchor() {
    let (mut d, id) = doc_with_rect();
    let l = d.layers[0].id;
    let tri = d.alloc_id();
    let pts = [Point::new(350.3, 250.0), Point::new(300.0, 320.0), Point::new(420.0, 330.0)];
    let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&pts, true));
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(tri, path, vectorcraft_doc::Appearance::default_art())).unwrap();
    let (none, mut picked) = (Selection::default(), Selection::default());
    picked.add(id);
    let p = paint();
    let mut t = SelectionTool::default();
    // Grab the rectangle (not selected yet: no bounding box) by its bottom-right corner
    // (200, 200); the press selects it, then drop it beside the tip.
    let before = cx(&d, &none, &p);
    t.pointer(&before, &ev(PointerKind::Move, 201.0, 201.0));
    t.pointer(&before, &ev(PointerKind::Down, 201.0, 201.0));
    let a = t.pointer(&cx(&d, &picked, &p), &ev(PointerKind::Drag, 350.0, 249.8));
    let Some(Action::Preview(_, v)) = a.get(1) else { panic!("{a:?}") };
    let (x, y) = (v["matrix"][4].as_f64().unwrap(), v["matrix"][5].as_f64().unwrap());
    assert!((200.0 + x - 350.3).abs() < 1e-9 && (200.0 + y - 250.0).abs() < 1e-9, "the corner lands at ({}, {})", 200.0 + x, 200.0 + y);
}

/// #3716: a press a few pixels from an anchor grabs the object by that anchor, but the drag
/// threshold counts from where the pointer went down. A click that wobbles by a pixel selects
/// the object without moving it, instead of jumping the anchor onto the pointer.
#[test]
fn a_click_near_an_anchor_selects_without_moving() {
    let (d, id) = doc_with_rect();
    let (none, mut picked) = (Selection::default(), Selection::default());
    picked.add(id);
    let p = paint();
    let mut t = SelectionTool::default();
    // 5.7 px from the corner (200, 200): within reach of it, beyond the drag threshold.
    let a = t.pointer(&cx(&d, &none, &p), &ev(PointerKind::Down, 196.0, 196.0));
    assert!(a.iter().any(|a| matches!(a, Action::Exec(c, v) if c == "select.set" && v["ids"] == json!([id.0]))), "{a:?}");
    let a = t.pointer(&cx(&d, &picked, &p), &ev(PointerKind::Drag, 197.0, 196.0));
    assert!(a.is_empty(), "a 1 px wobble moves the object: {a:?}");
    let a = t.pointer(&cx(&d, &picked, &p), &ev(PointerKind::Up, 197.0, 196.0));
    assert!(!a.iter().any(|a| matches!(a, Action::Commit)), "{a:?}");
    // A drag past the threshold from the press still moves it by the corner (#812).
    let mut t = SelectionTool::default();
    t.pointer(&cx(&d, &none, &p), &ev(PointerKind::Down, 196.0, 196.0));
    let a = t.pointer(&cx(&d, &picked, &p), &ev(PointerKind::Drag, 230.0, 196.0));
    let Some(Action::Preview(_, v)) = a.get(1) else { panic!("{a:?}") };
    assert_eq!((v["matrix"][4].as_f64(), v["matrix"][5].as_f64()), (Some(30.0), Some(-4.0)), "the corner follows the pointer: {a:?}");
}

/// A corner-radius drag is a transform, so the sharp selection box hides while the radius changes.
#[test]
fn a_corner_drag_counts_as_transforming_once_it_moves() {
    use vectorcraft_doc::{Appearance, LiveShape, Node, NodeKind};
    let live = LiveShape::Rectangle {
        w: 100.0,
        h: 100.0,
        radii: [0.0; 4],
        kinds: Default::default(),
        radii_relative: false,
        xf: vectorcraft_geom::Affine::translate((100.0, 100.0)),
    };
    let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
    let id = d.alloc_id();
    let mut n = Node::path(id, live.to_path(), Appearance::default_art());
    if let NodeKind::Path { live: slot, .. } = &mut n.kind {
        *slot = Some(Box::new(live));
    }
    d.insert(Some(d.layers[0].id), 0, n).unwrap();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    assert!(!t.transforming());
    t.pointer(&cx, &ev(PointerKind::Down, 190.0, 191.0));
    assert!(!t.transforming(), "a press is still a click");
    t.pointer(&cx, &ev(PointerKind::Drag, 175.0, 176.0));
    assert!(t.transforming());
}

/// Escape during a corner widget drag cancels it, and the release after it commits nothing.
/// After an Alt-press on a widget, Escape drops the press, so its release cycles no corner kind.
#[test]
fn escape_cancels_a_corner_widget_drag() {
    use vectorcraft_doc::{Appearance, LiveShape, Node, NodeKind};
    let live = LiveShape::Rectangle {
        w: 100.0,
        h: 100.0,
        radii: [0.0; 4],
        kinds: Default::default(),
        radii_relative: false,
        xf: vectorcraft_geom::Affine::translate((100.0, 100.0)),
    };
    let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
    let id = d.alloc_id();
    let mut n = Node::path(id, live.to_path(), Appearance::default_art());
    if let NodeKind::Path { live: slot, .. } = &mut n.kind {
        *slot = Some(Box::new(live));
    }
    d.insert(Some(d.layers[0].id), 0, n).unwrap();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    assert!(t.pointer(&cx, &ev(PointerKind::Down, 190.0, 191.0)).is_empty());
    let a = t.pointer(&cx, &ev(PointerKind::Drag, 175.0, 176.0));
    assert_eq!(a.first(), Some(&Action::Begin("Corner Radius".into())), "{a:?}");
    assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel], "Escape cancels the drag");
    assert!(!t.busy() && !t.transforming());
    assert!(t.pointer(&cx, &ev(PointerKind::Drag, 170.0, 171.0)).is_empty(), "the drag is over");
    assert!(t.pointer(&cx, &ev(PointerKind::Up, 170.0, 171.0)).is_empty(), "the release commits nothing");
    let alt = Mods { alt: true, ..Default::default() };
    t.pointer(&cx, &ev(PointerKind::Down, 190.0, 191.0).with_mods(alt));
    assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
    assert!(t.pointer(&cx, &ev(PointerKind::Up, 190.0, 191.0).with_mods(alt)).is_empty(), "the Alt-click cycles no corner kind");
}

/// A type-on-path bracket drag is a transform, so the sharp selection box hides while the span moves.
#[test]
fn a_bracket_drag_counts_as_transforming_once_it_moves() {
    let (d, id) = crate::testutil::doc_with_path_type();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    assert!(!t.transforming());
    t.pointer(&cx, &ev(PointerKind::Down, 280.0, 290.0));
    assert!(!t.transforming(), "a press is still a click");
    t.pointer(&cx, &ev(PointerKind::Drag, 320.0, 290.0));
    assert!(t.transforming());
}

/// Escape during a drag of the brackets of type on a path cancels it, and the release after it
/// commits nothing.
#[test]
fn escape_cancels_a_bracket_drag() {
    let (d, id) = crate::testutil::doc_with_path_type();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    assert!(t.pointer(&cx, &ev(PointerKind::Down, 280.0, 290.0)).is_empty());
    let a = t.pointer(&cx, &ev(PointerKind::Drag, 320.0, 290.0));
    assert!(matches!(a.first(), Some(Action::Begin(_))), "{a:?}");
    assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel], "Escape cancels the drag");
    assert!(!t.busy() && !t.transforming());
    assert!(t.pointer(&cx, &ev(PointerKind::Drag, 330.0, 290.0)).is_empty(), "the drag is over");
    assert!(t.pointer(&cx, &ev(PointerKind::Up, 330.0, 290.0)).is_empty(), "the release commits nothing");
}

/// #812: with Smart Guides on, hovering near an object's centre or an anchor labels it, and a
/// press there grabs the object by that point.
#[test]
fn hovering_labels_the_centre_and_anchors_and_a_press_grabs_by_them() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let label = |t: &SelectionTool| {
        t.overlays(&cx).iter().find_map(|o| match o {
            Overlay::Label { p, text, .. } => Some((*p, text.clone())),
            _ => None,
        })
    };
    let mut t = SelectionTool::default();
    t.pointer(&cx, &ev(PointerKind::Move, 152.0, 151.0));
    assert_eq!(label(&t), Some((Point::new(150.0, 150.0), "center".to_string())));
    t.pointer(&cx, &ev(PointerKind::Move, 102.0, 101.0));
    assert_eq!(label(&t), Some((Point::new(100.0, 100.0), "anchor".to_string())));
    t.pointer(&cx, &ev(PointerKind::Move, 130.0, 170.0));
    assert_eq!(label(&t), None, "away from both");
    // Grabbed by its centre, the object's centre follows the pointer.
    t.pointer(&cx, &ev(PointerKind::Move, 152.0, 151.0));
    t.pointer(&cx, &ev(PointerKind::Down, 152.0, 151.0));
    let a = t.pointer(&cx, &ev(PointerKind::Drag, 250.0, 250.0));
    assert!(a.iter().any(|a| matches!(a, Action::Preview(_, v) if v["matrix"][4] == 100.0 && v["matrix"][5] == 100.0)), "{a:?}");
    // With Smart Guides off, nothing is labelled.
    let off = ToolContext { smart_guides: false, ..cx };
    let mut t = SelectionTool::default();
    t.pointer(&off, &ev(PointerKind::Move, 152.0, 151.0));
    assert!(!t.overlays(&off).iter().any(|o| matches!(o, Overlay::Label { .. })));
}

/// [`doc_with_rect`] (100, 100)–(200, 200), selected, and a second rectangle `other`.
fn two_rects(other: Rect) -> (vectorcraft_doc::Document, NodeId, NodeId) {
    let (mut d, a) = doc_with_rect();
    let l = d.layers[0].id;
    let b = d.alloc_id();
    let path = vectorcraft_geom::shapes::rectangle(other);
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(b, path, vectorcraft_doc::Appearance::default_art())).unwrap();
    (d, a, b)
}

fn alt(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
    ev(kind, x, y).with_mods(Mods { alt: true, ..Mods::default() })
}

/// A Distance Guides measured line: its ends and its label.
type DistanceLine = (Point, Point, String);

/// The Distance Guides overlays: each measured line, and the dashed lines out to it, in order.
fn distances(t: &SelectionTool, cx: &ToolContext) -> (Vec<DistanceLine>, Vec<(Point, Point)>) {
    let o = t.overlays(cx);
    let lines = o.iter().filter_map(|o| if let Overlay::Distance { a, b, text, .. } = o { Some((*a, *b, text.clone())) } else { None });
    let dashed = o.iter().filter_map(|o| if let Overlay::Line { a, b, dashed: true, .. } = o { Some((*a, *b)) } else { None });
    (lines.collect(), dashed.collect())
}

fn line(a: (f64, f64), b: (f64, f64), text: &str) -> DistanceLine {
    (Point::new(a.0, a.1), Point::new(b.0, b.1), text.to_string())
}

/// Alt held over an object beside the selection shows the horizontal gap between their
/// facing edges across the middle of where they overlap, and the vertical offsets of their
/// top and bottom edges across the selection's middle, with dashed lines out to the other
/// object's edges (helpx: Use Distance Guides for accurate placement).
#[test]
fn alt_over_an_object_beside_the_selection_measures_the_gap_and_the_edges() {
    let (d, a, _) = two_rects(Rect::new(300.0, 120.0, 400.0, 180.0));
    let s = Selection { objects: vec![a], ..Default::default() };
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    t.pointer(&cx, &alt(PointerKind::Move, 350.0, 150.0));
    let (lines, dashed) = distances(&t, &cx);
    assert_eq!(
        lines,
        vec![
            line((200.0, 150.0), (300.0, 150.0), "100.00 pt"),
            line((150.0, 100.0), (150.0, 120.0), "20.00 pt"),
            line((150.0, 200.0), (150.0, 180.0), "20.00 pt"),
        ]
    );
    let p = Point::new;
    assert_eq!(dashed, vec![(p(300.0, 120.0), p(150.0, 120.0)), (p(300.0, 180.0), p(150.0, 180.0))]);
    assert!(t.overlays(&cx).iter().all(|o| !matches!(o, Overlay::Distance { color, .. } if *color != cx.smart_guide_color)));
}

/// Below the selection: the vertical gap, and the left and right edges' offsets.
#[test]
fn alt_over_an_object_below_the_selection_measures_the_vertical_gap() {
    let (d, a, _) = two_rects(Rect::new(120.0, 300.0, 180.0, 400.0));
    let s = Selection { objects: vec![a], ..Default::default() };
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    t.pointer(&cx, &alt(PointerKind::Move, 150.0, 350.0));
    let (lines, dashed) = distances(&t, &cx);
    assert_eq!(
        lines,
        vec![
            line((100.0, 150.0), (120.0, 150.0), "20.00 pt"),
            line((200.0, 150.0), (180.0, 150.0), "20.00 pt"),
            line((150.0, 200.0), (150.0, 300.0), "100.00 pt"),
        ]
    );
    assert_eq!(dashed.len(), 2, "out to the left and right edges of the object below");
}

/// Alt held over an empty part of the artboard: the selection's distance from the artboard's
/// left, right, top and bottom edges, in the General unit.
#[test]
fn alt_over_the_artboard_measures_to_its_four_edges_in_the_general_unit() {
    let (d, a, _) = two_rects(Rect::new(300.0, 120.0, 400.0, 180.0));
    let s = Selection { objects: vec![a], ..Default::default() };
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    t.pointer(&cx, &alt(PointerKind::Move, 50.0, 450.0));
    let (lines, dashed) = distances(&t, &cx);
    assert_eq!(
        lines,
        vec![
            line((100.0, 150.0), (0.0, 150.0), "100.00 pt"),
            line((200.0, 150.0), (500.0, 150.0), "300.00 pt"),
            line((150.0, 100.0), (150.0, 0.0), "100.00 pt"),
            line((150.0, 200.0), (150.0, 500.0), "300.00 pt"),
        ]
    );
    assert!(dashed.is_empty());
    let mm = ToolContext { unit: vectorcraft_doc::Unit::Millimeters, ..cx };
    t.pointer(&mm, &alt(PointerKind::Move, 50.0, 450.0));
    let texts: Vec<String> = distances(&t, &mm).0.into_iter().map(|l| l.2).collect();
    assert_eq!(texts, ["35.28 mm", "105.83 mm", "35.28 mm", "105.83 mm"]);
    // Off the artboard, over nothing: no distances.
    t.pointer(&cx, &alt(PointerKind::Move, 600.0, 600.0));
    assert!(distances(&t, &cx).0.is_empty());
}

/// No Distance Guides without Alt, with nothing selected, over the selection itself, with
/// Smart Guides or Invoke Distance Guides off, or once Alt is let go.
#[test]
fn distance_guides_need_alt_a_selection_and_smart_guides() {
    let (d, a, _) = two_rects(Rect::new(300.0, 120.0, 400.0, 180.0));
    let s = Selection { objects: vec![a], ..Default::default() };
    let none = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let shown = |cx: &ToolContext, e: PointerEvent| {
        let mut t = SelectionTool::default();
        t.pointer(cx, &e);
        !distances(&t, cx).0.is_empty()
    };
    assert!(shown(&cx, alt(PointerKind::Move, 350.0, 150.0)));
    assert!(!shown(&cx, ev(PointerKind::Move, 350.0, 150.0)), "without Alt");
    assert!(!shown(&cx, ev(PointerKind::Move, 50.0, 450.0)), "without Alt, over the artboard");
    assert!(!shown(&crate::testutil::cx(&d, &none, &p), alt(PointerKind::Move, 350.0, 150.0)), "nothing selected");
    assert!(!shown(&cx, alt(PointerKind::Move, 150.0, 150.0)), "over the selection");
    assert!(
        !shown(&ToolContext { smart_guides: false, ..crate::testutil::cx(&d, &s, &p) }, alt(PointerKind::Move, 350.0, 150.0)),
        "View › Smart Guides off"
    );
    assert!(
        !shown(&ToolContext { distance_guides: false, ..crate::testutil::cx(&d, &s, &p) }, alt(PointerKind::Move, 350.0, 150.0)),
        "the preference off"
    );
    // Letting Alt go (the canvas sends a hover with the new modifiers) takes them away.
    let mut t = SelectionTool::default();
    t.pointer(&cx, &alt(PointerKind::Move, 350.0, 150.0));
    t.pointer(&cx, &ev(PointerKind::Move, 350.0, 150.0));
    assert!(distances(&t, &cx).0.is_empty());
    // So does the hover ending (the pointer leaving the canvas) with Alt still held.
    t.pointer(&cx, &alt(PointerKind::Move, 350.0, 150.0));
    assert!(!distances(&t, &cx).0.is_empty());
    assert!(t.wants_rehover(), "an Alt hover is hovered again when what lies under it changes");
    t.hover_ended(&cx);
    assert!(distances(&t, &cx).0.is_empty());
    assert!(!t.wants_rehover());
    t.pointer(&cx, &ev(PointerKind::Move, 350.0, 150.0));
    assert!(!t.wants_rehover(), "nor without Alt");
    // A press ends them too.
    t.pointer(&cx, &alt(PointerKind::Move, 350.0, 150.0));
    t.pointer(&cx, &alt(PointerKind::Down, 350.0, 150.0));
    assert!(distances(&t, &cx).0.is_empty());
}

#[test]
fn click_selects_object() {
    let (d, id) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    let a = t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0));
    assert_eq!(a, vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
    assert!(t.pointer(&cx, &ev(PointerKind::Up, 150.0, 150.0)).is_empty());
}

/// #414: a press on a ruler guide picks it over the art, and drags it.
#[test]
fn a_press_on_a_ruler_guide_picks_it_over_the_art() {
    let (mut d, _) = doc_with_rect();
    d.guides.push(vectorcraft_doc::Guide::new(false, 150.0));
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    assert_eq!(t.cursor(&cx, Point::new(160.0, 151.0), Mods::default()), Cursor::ResizeV);
    let a = t.pointer(&cx, &ev(PointerKind::Down, 160.0, 151.0));
    assert_eq!(a, vec![Action::Exec("guide.select".into(), json!({"indexes": [0]}))]);
    assert!(t.busy());
    let a = t.pointer(&cx, &ev(PointerKind::Drag, 160.0, 171.0));
    assert_eq!(a[1], Action::Preview("guide.move".into(), json!({"dx": 0.0, "dy": 20.0, "copy": false})));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 160.0, 171.0)), vec![Action::Commit]);
    assert!(!t.busy());
    // A double-click there doesn't go through to the art.
    assert!(t.pointer(&cx, &ev(PointerKind::DoubleClick, 160.0, 150.0)).is_empty());
}

/// A Cmd/Ctrl+Shift double-click releases the ruler guide or the guide path under the pointer;
/// without both keys, or while guides are hidden or locked, nothing is released.
#[test]
fn a_cmd_shift_double_click_releases_the_guide_under_the_pointer() {
    let (mut d, id) = doc_with_rect();
    d.guides.push(vectorcraft_doc::Guide::new(false, 300.0));
    let Some(vectorcraft_doc::NodeKind::Path { guide, .. }) = d.node_mut(id).map(|n| &mut n.kind) else { panic!("the rectangle is a path") };
    *guide = true;
    let s = Selection::default();
    let p = paint();
    let both = Mods { cmd: true, shift: true, ..Mods::default() };
    let dbl = |c: &ToolContext, x: f64, y: f64, m: Mods| SelectionTool::default().pointer(c, &ev(PointerKind::DoubleClick, x, y).with_mods(m));
    let releases = |a: &[Action]| a.iter().any(|a| matches!(a, Action::Exec(c, _) if c == "view.guides.release"));
    let c = cx(&d, &s, &p);
    assert_eq!(dbl(&c, 350.0, 301.0, both), vec![Action::Exec("view.guides.release".into(), json!({"index": 0}))]);
    assert_eq!(dbl(&c, 200.0, 150.0, both), vec![Action::Exec("view.guides.release".into(), json!({"ids": [id.0]}))]);
    assert!(dbl(&c, 350.0, 301.0, Mods::default()).is_empty(), "a plain double-click on a ruler guide");
    for m in [Mods { cmd: true, ..Mods::default() }, Mods { shift: true, ..Mods::default() }, Mods::default()] {
        assert!(!releases(&dbl(&c, 200.0, 150.0, m)), "{m:?}");
    }
    let locked = ToolContext { guides: false, ..cx(&d, &s, &p) };
    assert!(!releases(&dbl(&locked, 350.0, 301.0, both)));
    assert!(!releases(&dbl(&locked, 200.0, 150.0, both)));
    // A double-click on art that is not a guide does not release it.
    let (plain, _) = doc_with_rect();
    assert!(!releases(&dbl(&cx(&plain, &s, &p), 200.0, 150.0, both)));
}

/// Just outside each corner of the box the rotate cursor bows toward that corner, its arc
/// opening toward the box: at the top right it opens to the bottom left.
#[test]
fn the_rotate_cursor_wraps_the_corner_it_is_at() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let t = SelectionTool::default();
    // The box runs from (100, 100) to (200, 200); 10 pt out from each corner, diagonally.
    for ((x, y), octant) in [((93.0, 93.0), 7), ((207.0, 93.0), 1), ((207.0, 207.0), 3), ((93.0, 207.0), 5)] {
        assert_eq!(t.cursor(&cx, Point::new(x, y), Mods::default()), Cursor::RotateToward(octant), "at ({x}, {y})");
    }
    assert_eq!(Cursor::rotate_about(Point::new(1.0, 1.0), Point::new(1.0, 1.0)), Cursor::Rotate, "on the centre");
    assert_eq!(Cursor::rotate_about(Point::ORIGIN, Point::new(0.0, -5.0)), Cursor::RotateToward(0), "straight up");
}

#[test]
fn turned_cursors_step_with_the_view() {
    assert_eq!(Cursor::ResizeH.turned(90.0), Cursor::ResizeV);
    assert_eq!(Cursor::ResizeNeSw.turned(45.0), Cursor::ResizeH);
    assert_eq!(Cursor::ResizeH.turned(45.0), Cursor::ResizeNwSe);
    assert_eq!(Cursor::ResizeNwSe.turned(-135.0), Cursor::ResizeV);
    assert_eq!(Cursor::RotateToward(0).turned(90.0), Cursor::RotateToward(2));
    assert_eq!(Cursor::RotateToward(1).turned(-45.0), Cursor::RotateToward(0));
    assert_eq!(Cursor::RotateToward(7).turned(180.0), Cursor::RotateToward(3));
    assert_eq!(Cursor::ResizeV.turned(20.0), Cursor::ResizeV, "under 22.5° it stays");
    assert_eq!(Cursor::Pen.turned(90.0), Cursor::Pen);
    assert_eq!(Cursor::Rotate.turned(90.0), Cursor::Rotate);
    assert_eq!(Cursor::ResizeH.turned(f64::NAN), Cursor::ResizeH);
    // `rotate_about_in_view` turns the pointer's direction by the view before it rounds: a
    // pointer 15° clockwise from straight up, in a view turned 15°, shows 30° from straight up
    // on screen, which rounds to the top right. `turned` rounds the 15° view to 0°.
    let (sn, cs) = 15f64.to_radians().sin_cos();
    let q = Point::new(sn, -cs);
    assert_eq!(Cursor::rotate_about_in_view(Point::ORIGIN, q, 15.0), Cursor::RotateToward(1));
    assert_eq!(Cursor::rotate_about(Point::ORIGIN, q).turned(15.0), Cursor::RotateToward(0));
    assert_eq!(Cursor::rotate_about_in_view(Point::ORIGIN, q, 0.0), Cursor::rotate_about(Point::ORIGIN, q));
    assert_eq!(Cursor::rotate_about_in_view(Point::ORIGIN, Point::new(0.0, -5.0), 90.0), Cursor::RotateToward(2));
    assert_eq!(Cursor::rotate_about_in_view(Point::ORIGIN, Point::new(0.0, -5.0), f64::NAN), Cursor::RotateToward(0));
    assert_eq!(Cursor::rotate_about_in_view(Point::ORIGIN, Point::ORIGIN, 90.0), Cursor::Rotate, "on the center");
}

/// In a rotated view the resize and rotate cursors turn with the view.
#[test]
fn box_cursors_turn_with_the_view() {
    let (mut d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    d.guides.push(vectorcraft_doc::Guide::new(false, 300.0));
    let mut c = cx(&d, &s, &p);
    c.screen = turned_view(90.0);
    assert!((c.view_angle() - 90.0).abs() < 1e-9);
    let t = SelectionTool::default();
    // The box runs from (100, 100) to (200, 200). Its right side shows at the bottom, and its
    // top shows on the right.
    assert_eq!(t.cursor(&c, Point::new(200.0, 150.0), Mods::default()), Cursor::ResizeV, "right-middle");
    assert_eq!(t.cursor(&c, Point::new(150.0, 100.0), Mods::default()), Cursor::ResizeH, "top-middle");
    assert_eq!(t.cursor(&c, Point::new(207.0, 93.0), Mods::default()), Cursor::RotateToward(3), "out from the top right");
    // While a rotate drags, the cursor turns with the view too: straight up the page shows to
    // the right.
    let mut r = SelectionTool::default();
    assert_eq!(r.pointer(&c, &ev(PointerKind::Down, 207.0, 93.0)), vec![Action::Begin("Rotate".into())]);
    assert_eq!(r.cursor(&c, Point::new(150.0, 80.0), Mods::default()), Cursor::RotateToward(2), "rotating");
    // A horizontal ruler guide shows upright and moves left and right on screen.
    assert_eq!(t.cursor(&c, Point::new(400.0, 300.0), Mods::default()), Cursor::ResizeH, "guide");
    // A box turned 20° in a view turned 25° shows 5° off level: the right handle shows the
    // left-right arrow.
    let (center, a) = (Point::new(150.0, 150.0), 20f64.to_radians());
    let turn = Affine::translate(center.to_vec2()) * Affine::rotate(-a) * Affine::translate(-center.to_vec2());
    d.node_mut(id).unwrap().transform(turn, false);
    let mut c = cx(&d, &s, &p);
    c.screen = turned_view(25.0);
    let right = Point::new(150.0 + 50.0 * a.cos(), 150.0 - 50.0 * a.sin());
    assert_eq!(t.cursor(&c, right, Mods::default()), Cursor::ResizeH, "20° box, 25° view");
}

#[test]
fn drag_moves_with_single_undo() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    assert!(t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0)).is_empty());
    let a = t.pointer(&cx, &ev(PointerKind::Drag, 160.0, 150.0));
    assert_eq!(a[0], Action::Begin("Move".into()));
    assert!(matches!(&a[1], Action::Preview(c, v) if c == "object.transform" && v["matrix"][4] == 10.0));
    let a = t.pointer(&cx, &ev(PointerKind::Up, 160.0, 150.0));
    assert_eq!(a, vec![Action::Commit]);
}

/// Escape during a move, or an Alt-drag copy, cancels it: the release after it commits nothing,
/// and the next drag is a new one. Escape with nothing to cancel sends nothing.
#[test]
fn escape_cancels_a_move_or_an_alt_drag_copy() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    for (what, m) in [("move", Mods::default()), ("copy", Mods { alt: true, ..Default::default() })] {
        let mut t = SelectionTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0).with_mods(m));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 160.0, 150.0).with_mods(m));
        assert!(matches!(&a[1], Action::Preview(_, v) if v["copy"] == m.alt), "{what}: {a:?}");
        assert!(t.transforming(), "{what}");
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel], "{what}: Escape cancels the drag");
        assert!(!t.busy() && !t.transforming(), "{what}");
        assert!(t.overlays(&cx).iter().all(|o| !matches!(o, Overlay::Measure { .. })), "{what}: no readout stays");
        assert!(t.pointer(&cx, &ev(PointerKind::Drag, 170.0, 150.0).with_mods(m)).is_empty(), "{what}: the drag is over");
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 170.0, 150.0).with_mods(m)).is_empty(), "{what}: the release commits nothing");
        assert!(t.key(&cx, ToolKey::Escape, Mods::default()).is_empty(), "{what}: nothing to cancel");
    }
}

/// #1534: with Smart Guides on (the default), the point a selection is dragged by still snaps
/// to a ruler guide within the Snap to Point distance; not with Snap to Point off.
#[test]
fn a_move_snaps_the_grabbed_point_to_a_ruler_guide_with_smart_guides_on() {
    let (mut d, id) = doc_with_rect();
    d.guides = vec![vectorcraft_doc::Guide::new(true, 173.4)];
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let dx = |c: &ToolContext, to: f64| {
        let mut t = SelectionTool::default();
        t.pointer(c, &ev(PointerKind::Down, 150.0, 150.0));
        let a = t.pointer(c, &ev(PointerKind::Drag, to, 150.0));
        let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
        v["matrix"][4].as_f64().unwrap()
    };
    let c = cx(&d, &s, &p);
    assert!(c.smart_guides && c.snap_to_point);
    assert!((dx(&c, 172.0) - 23.4).abs() < 1e-9, "onto the guide");
    assert_eq!(dx(&c, 165.0), 15.0, "out of reach");
    assert_eq!(dx(&ToolContext { snap_to_point: false, ..cx(&d, &s, &p) }, 172.0), 22.0);
}

/// Alt pressed during a move makes a copy, which snaps to the original it leaves behind (as
/// gathered before the previews moved it); released, the move doesn't.
#[test]
fn an_alt_copy_snaps_to_the_original_left_behind() {
    use vectorcraft_doc::{Appearance, Node};
    let (mut d, id) = doc_with_rect();
    // An artboard whose edges and centre are out of reach.
    d.artboards[0].rect = Rect::new(0.0, 0.0, 1000.0, 1000.0);
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let mut t = SelectionTool::default();
    let dx = |a: &[Action]| {
        let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
        v["matrix"][4].as_f64().unwrap()
    };
    t.pointer(&cx(&d, &s, &p), &ev(PointerKind::Down, 150.0, 150.0));
    assert_eq!(dx(&t.pointer(&cx(&d, &s, &p), &ev(PointerKind::Drag, 200.0, 150.0))), 50.0);
    // The previews moved the rectangle: the tool sees it 50 pt along.
    let mut moved = d.clone();
    let l = moved.layers[0].id;
    moved.remove(id).unwrap();
    let r = vectorcraft_geom::shapes::rectangle(Rect::new(150.0, 100.0, 250.0, 200.0));
    moved.insert(Some(l), 0, Node::path(id, r, Appearance::default_art())).unwrap();
    let c = cx(&moved, &s, &p);
    let alt = Mods { alt: true, ..Mods::default() };
    // The copy's left edge, 3 pt past the original's right edge (x = 200), lands on it.
    assert_eq!(dx(&t.pointer(&c, &ev(PointerKind::Drag, 253.0, 150.0).with_mods(alt))), 100.0);
    // Without Alt nothing stays behind there.
    assert_eq!(dx(&t.pointer(&c, &ev(PointerKind::Drag, 253.0, 150.0))), 103.0);
}

/// A move shows the filled arrow, hollow while it snaps (here B's left side onto A's right
/// one); Alt, which leaves a copy behind, the double arrowhead.
#[test]
fn a_move_shows_the_filled_arrow_hollow_on_a_snap_and_alt_the_copy_cursor() {
    use vectorcraft_doc::{Appearance, Node};
    let (mut d, _) = doc_with_rect();
    d.artboards[0].rect = Rect::new(0.0, 0.0, 1000.0, 1000.0);
    let l = d.layers[0].id;
    let b = d.alloc_id();
    let sq = vectorcraft_geom::shapes::rectangle(Rect::new(300.0, 300.0, 400.0, 400.0));
    d.insert(Some(l), 1, Node::path(b, sq, Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.add(b);
    let p = paint();
    let c = cx(&d, &s, &p);
    let none = Mods::default();
    let mut t = SelectionTool::default();
    t.pointer(&c, &ev(PointerKind::Down, 350.0, 350.0));
    t.pointer(&c, &ev(PointerKind::Drag, 377.0, 371.0));
    assert_eq!(t.cursor(&c, Point::new(377.0, 371.0), none), Cursor::Arrow);
    t.pointer(&c, &ev(PointerKind::Drag, 253.0, 371.0));
    assert_eq!(t.cursor(&c, Point::new(253.0, 371.0), none), Cursor::ArrowHollow, "lined up with A");
    let alt = Mods { alt: true, ..none };
    t.pointer(&c, &ev(PointerKind::Drag, 377.0, 371.0).with_mods(alt));
    assert_eq!(t.cursor(&c, Point::new(377.0, 371.0), alt), Cursor::SelectionCopy);
}

/// #740, #1816: with Snap to Grid on, a moved object lands on the grid (every 9 pt by default)
/// by whichever of its edges is nearest a gridline, on each axis.
#[test]
fn a_move_lands_the_nearest_edge_on_the_grid() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = ToolContext { snap_to_grid: true, smart_guides: true, ..cx(&d, &s, &p) };
    let moved = |to: (f64, f64)| {
        let mut t = SelectionTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, to.0, to.1));
        match a.get(1) {
            Some(Action::Preview(_, v)) => (v["matrix"][4].as_f64().unwrap(), v["matrix"][5].as_f64().unwrap()),
            other => panic!("{other:?}"),
        }
    };
    // Bounds 100–200 moved by (13, 4): the right and bottom edges (213, 204) are nearest
    // gridlines (216, 207).
    let (x, y) = moved((163.0, 154.0));
    assert!((x - 16.0).abs() < 1e-9 && (y - 7.0).abs() < 1e-9, "{x} {y}");
    // Moved by (7, 0): the right edge is on a gridline (207) and the top goes up to one (99).
    let (x, y) = moved((157.0, 150.0));
    assert!((x - 7.0).abs() < 1e-9 && (y + 1.0).abs() < 1e-9, "{x} {y}");
}

/// The selection Shift-dragged from (150, 150) to `to`: the move it previews, and its guide
/// lines.
fn shift_moved(cx: &ToolContext, to: (f64, f64)) -> (Vec2, Vec<(Point, Point)>) {
    let shift = |kind, x, y| ev(kind, x, y).with_mods(Mods { shift: true, ..Mods::default() });
    let mut t = SelectionTool::default();
    t.pointer(cx, &shift(PointerKind::Down, 150.0, 150.0));
    let a = t.pointer(cx, &shift(PointerKind::Drag, to.0, to.1));
    let Some(Action::Preview(_, v)) = a.get(1) else { panic!("{a:?}") };
    let lines = t.overlays(cx).into_iter().filter_map(|o| if let Overlay::Line { a, b, .. } = o { Some((a, b)) } else { None }).collect();
    (Vec2::new(v["matrix"][4].as_f64().unwrap(), v["matrix"][5].as_f64().unwrap()), lines)
}

/// `d` is `want` within 1e-9 on each axis.
fn moved_by(d: Vec2, want: (f64, f64)) -> bool {
    (d.x - want.0).abs() < 1e-9 && (d.y - want.1).abs() < 1e-9
}

/// A Shift-constrained move snapped to the grid stays on its line: a level or upright one lands
/// on the grid along its own axis only, and a diagonal one slides along the diagonal until an
/// edge is on a gridline on the axis that needs the shorter way there.
#[test]
fn a_shift_move_snapped_to_the_grid_stays_on_its_line() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
    // Level by 13.04: the right edge (213.04) lands on the gridline 216, and the top stays at
    // 100, between the gridlines 99 and 108.
    let (m, _) = shift_moved(&cx, (163.0, 151.0));
    assert!(moved_by(m, (16.0, 0.0)), "{m:?}");
    let (m, _) = shift_moved(&cx, (151.0, 163.0));
    assert!(moved_by(m, (0.0, 16.0)), "{m:?}");
    // Up and to the right by 13 on each axis: the bottom edge (187) is 2 from the gridline 189,
    // and the right edge (213) is 3 from 216, so the move goes back along the diagonal by 2 on
    // each axis.
    let (m, _) = shift_moved(&cx, (163.0, 137.0));
    assert!(moved_by(m, (11.0, -11.0)), "{m:?}");
}

/// #1816: on a 20 pt grid (gridlines every 100, 5 subdivisions), the 100 pt square at 100–200
/// sits on the grid with its center half a step off. A move by (8, 8) brings the left and top
/// edges back onto the gridlines at 100, so the square stays where it was; a move by (31, 29)
/// lands its edges on the nearest gridlines (140, 120). The center does not snap.
#[test]
fn a_move_keeps_an_object_on_the_grid_when_its_center_is_off_it() {
    let (mut d, id) = doc_with_rect();
    d.grid = vectorcraft_doc::GridPrefs { spacing: 100.0, subdivisions: 5 };
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
    let moved = |to: (f64, f64)| {
        let mut t = SelectionTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, to.0, to.1));
        let Some(Action::Preview(_, v)) = a.get(1) else { panic!("{a:?}") };
        Vec2::new(v["matrix"][4].as_f64().unwrap(), v["matrix"][5].as_f64().unwrap())
    };
    let m = moved((158.0, 158.0));
    assert!(moved_by(m, (0.0, 0.0)), "{m:?}");
    let m = moved((181.0, 179.0));
    assert!(moved_by(m, (40.0, 20.0)), "{m:?}");
    // With Shift, level and diagonal moves by 8 come back to where they started too.
    let (m, _) = shift_moved(&cx, (158.0, 150.0));
    assert!(moved_by(m, (0.0, 0.0)), "{m:?}");
    let (m, _) = shift_moved(&cx, (158.0, 158.0));
    assert!(moved_by(m, (0.0, 0.0)), "{m:?}");
}

/// A Shift-constrained move with Snap to Pixel on puts the top-left on a whole pixel on the
/// axis its line runs along, and on a diagonal on the axis that needs the shorter way there.
#[test]
fn a_shift_move_snapped_to_pixels_stays_on_its_line() {
    let mut d = vectorcraft_doc::Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let square = vectorcraft_geom::shapes::rectangle(Rect::new(100.3, 100.4, 200.3, 200.4));
    d.insert(Some(l), 0, vectorcraft_doc::Node::path(id, square, vectorcraft_doc::Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = ToolContext { snap_to_pixel: true, ..cx(&d, &s, &p) };
    // Pressed at (150, 150), the square is grabbed by its center (150.3, 150.4).
    let (m, _) = shift_moved(&cx, (163.0, 151.0));
    assert!(moved_by(m, (12.7, 0.0)), "{m:?}");
    let (m, _) = shift_moved(&cx, (151.0, 163.0));
    assert!(moved_by(m, (0.0, 12.6)), "{m:?}");
    // By 13.16 on each axis, the top-left (113.46, 113.56) is 0.44 from a whole pixel on y and
    // 0.46 on x.
    let (m, _) = shift_moved(&cx, (163.0, 164.0));
    assert!(moved_by(m, (13.6, 13.6)), "{m:?}");
}

/// A Shift-constrained move that Smart Guides line up with another object stays on its line:
/// a level or upright one lines up along its own axis only, with the guide of that axis, and
/// a diagonal one slides along the diagonal to the target that needs the shorter way.
#[test]
fn a_shift_move_lined_up_by_smart_guides_stays_on_its_line() {
    let with = |r: Rect| {
        let (mut d, id) = doc_with_rect();
        let l = d.layers[0].id;
        let other = d.alloc_id();
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(other, vectorcraft_geom::shapes::rectangle(r), vectorcraft_doc::Appearance::default_art()))
            .unwrap();
        (d, id)
    };
    let p = paint();
    // Level by 100.02: the right edge lines up with the other square's left (300). That
    // square's top (203), 3 below the selection's bottom and within reach, does not pull it
    // down.
    let (d, id) = with(Rect::new(300.0, 203.0, 350.0, 253.0));
    let mut s = Selection::default();
    s.add(id);
    let (m, lines) = shift_moved(&cx(&d, &s, &p), (250.0, 152.0));
    assert!(moved_by(m, (100.0, 0.0)), "{m:?}");
    assert!(!lines.is_empty() && lines.iter().all(|(a, b)| a.x == b.x), "only upright guides: {lines:?}");
    let (d, id) = with(Rect::new(203.0, 300.0, 253.0, 350.0));
    let mut s = Selection::default();
    s.add(id);
    let (m, lines) = shift_moved(&cx(&d, &s, &p), (152.0, 250.0));
    assert!(moved_by(m, (0.0, 100.0)), "{m:?}");
    assert!(!lines.is_empty() && lines.iter().all(|(a, b)| a.y == b.y), "only level guides: {lines:?}");
    // By 110.5 on each axis: the right edge (310.5) is 0.5 from the other square's left and the
    // bottom 3 from its top, so the move goes on by 0.5 on each axis.
    let (d, id) = with(Rect::new(311.0, 313.5, 361.0, 363.5));
    let mut s = Selection::default();
    s.add(id);
    let (m, lines) = shift_moved(&cx(&d, &s, &p), (260.5, 260.5));
    assert!(moved_by(m, (111.0, 111.0)), "{m:?}");
    assert!(matches!(lines[..], [(a, b)] if a.x == 311.0 && b.x == 311.0), "{lines:?}");
}

/// A level Shift-constrained move that Spacing Guides space evenly keeps its line: the gap to
/// the next square becomes 60, the gap between the other two, and their tops (102), 2 below
/// the selection's, do not pull it down.
#[test]
fn a_shift_move_spaced_by_smart_guides_stays_on_its_line() {
    let (mut d, id) = doc_with_rect();
    let l = d.layers[0].id;
    for x in [330.0, 440.0] {
        let other = d.alloc_id();
        let sq = vectorcraft_geom::shapes::rectangle(Rect::new(x, 102.0, x + 50.0, 202.0));
        d.insert(Some(l), 1, vectorcraft_doc::Node::path(other, sq, vectorcraft_doc::Appearance::default_art())).unwrap();
    }
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let (m, lines) = shift_moved(&cx(&d, &s, &p), (221.0, 151.0));
    assert!(moved_by(m, (70.0, 0.0)), "{m:?}");
    assert!(!lines.is_empty(), "the equal gaps are marked");
}

/// A Shift-constrained move that snaps by its grabbed point stays on its line: with Smart
/// Guides off and Snap to Point on, it stops where the line passes nearest an anchor, or
/// where it crosses a ruler guide; with Smart Guides on, Snap to Point still slides it along
/// the line onto a ruler guide.
#[test]
fn a_shift_move_snapped_by_its_grabbed_point_stays_on_its_line() {
    let (mut d, id) = doc_with_rect();
    let l = d.layers[0].id;
    let other = d.alloc_id();
    let sq = vectorcraft_geom::shapes::rectangle(Rect::new(300.0, 151.5, 350.0, 201.5));
    d.insert(Some(l), 1, vectorcraft_doc::Node::path(other, sq, vectorcraft_doc::Appearance::default_art())).unwrap();
    d.guides = vec![vectorcraft_doc::Guide::new(true, 420.5)];
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let off = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    // The grabbed point (299, 150) is 1.8 from the anchor (300, 151.5): it stops at (300, 150),
    // the point of the line nearest the anchor.
    let (m, _) = shift_moved(&off, (299.0, 151.0));
    assert!(moved_by(m, (150.0, 0.0)), "{m:?}");
    // On the diagonal, 1 short of the ruler guide at 420.5: it goes on to it on both axes.
    let (m, _) = shift_moved(&off, (419.5, 419.5));
    assert!(moved_by(m, (270.5, 270.5)), "{m:?}");
    // Smart Guides on, the selection alone with a ruler guide 1.5 below the grabbed point.
    let (mut d, id) = doc_with_rect();
    d.guides = vec![vectorcraft_doc::Guide::new(false, 330.5)];
    let mut s = Selection::default();
    s.add(id);
    let (m, _) = shift_moved(&cx(&d, &s, &p), (329.0, 329.0));
    assert!(moved_by(m, (180.5, 180.5)), "{m:?}");
}

#[test]
fn shift_drag_on_a_selected_object_moves_it_constrained() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    let shift = |kind, x, y| ev(kind, x, y).with_mods(Mods { shift: true, ..Mods::default() });
    // Pressing keeps the selection: nothing is toggled yet.
    assert!(t.pointer(&cx, &shift(PointerKind::Down, 150.0, 150.0)).is_empty());
    let a = t.pointer(&cx, &shift(PointerKind::Drag, 190.0, 160.0));
    assert_eq!(a[0], Action::Begin("Move".into()));
    assert!(
        matches!(&a[1], Action::Preview(c, v) if c == "object.transform"
            && v["matrix"][4].as_f64().is_some_and(|x| x > 39.0) && v["matrix"][5] == 0.0),
        "moved right, constrained to horizontal: {a:?}"
    );
    assert_eq!(t.pointer(&cx, &shift(PointerKind::Up, 190.0, 160.0)), vec![Action::Commit]);
}

#[test]
fn shift_click_toggles_an_object_on_release() {
    let (d, id) = doc_with_rect();
    let p = paint();
    let shift = |kind, x, y| ev(kind, x, y).with_mods(Mods { shift: true, ..Mods::default() });
    let toggle = vec![Action::Exec("select.toggle".into(), json!({"id": id.0}))];
    // Selected: it leaves the selection when the click is released.
    let mut s = Selection::default();
    s.add(id);
    let cx1 = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    assert!(t.pointer(&cx1, &shift(PointerKind::Down, 150.0, 150.0)).is_empty());
    assert_eq!(t.pointer(&cx1, &shift(PointerKind::Up, 150.0, 150.0)), toggle);
    // Not selected: it joins the selection at once, ready to be dragged.
    let s = Selection::default();
    let cx2 = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    assert_eq!(t.pointer(&cx2, &shift(PointerKind::Down, 150.0, 150.0)), toggle);
    assert!(t.pointer(&cx2, &shift(PointerKind::Up, 150.0, 150.0)).is_empty());
}

/// #541: a click on one object of a selection of several makes it the key object; a click on
/// the key again lets it go. A drag moves them all, a lone object has no key unless ruler
/// guides are selected with it.
#[test]
fn a_click_on_a_selected_object_makes_it_the_key() {
    let (d, id) = doc_with_area_type();
    let rect = d.layers[0].children().and_then(|c| c.first()).map(|n| n.id).unwrap();
    let p = paint();
    let click = |s: &Selection, x, y| {
        let c = cx(&d, s, &p);
        let mut t = SelectionTool::default();
        assert!(t.pointer(&c, &ev(PointerKind::Down, x, y)).is_empty());
        t.pointer(&c, &ev(PointerKind::Up, x, y))
    };
    let mut s = Selection::default();
    s.add(rect);
    s.add(id);
    assert_eq!(click(&s, 150.0, 150.0), vec![Action::Exec("select.key".into(), json!({"id": rect.0}))]);
    s.key = Some(rect);
    assert_eq!(click(&s, 150.0, 150.0), vec![Action::Exec("select.key".into(), json!({}))], "the key again: no key");
    // A drag moves the selection and leaves the key alone.
    let c = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    t.pointer(&c, &ev(PointerKind::Down, 150.0, 150.0));
    t.pointer(&c, &ev(PointerKind::Drag, 170.0, 150.0));
    assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 170.0, 150.0)), vec![Action::Commit]);
    // One object selected: a click on it does nothing, unless ruler guides are selected
    // with it for it to be the key of (#2143).
    let mut one = Selection::default();
    one.add(rect);
    assert!(click(&one, 150.0, 150.0).is_empty());
    one.guides = vec![0];
    assert_eq!(click(&one, 150.0, 150.0), vec![Action::Exec("select.key".into(), json!({"id": rect.0}))]);
}

/// In isolation mode a click makes only a selected top-level object the key, as `select.key`
/// takes no other: a click on a sublayer's art inside an isolated layer picks the sublayer,
/// a click inside an isolated group that is also selected picks an object inside it, and a
/// click inside an isolated compound path picks a member.
#[test]
fn a_click_makes_only_a_selected_top_level_object_the_key() {
    use vectorcraft_doc::{Appearance, Document, LayerColor, Node, NodeKind};
    use vectorcraft_geom::shapes;
    let p = paint();
    let click = |d: &Document, s: &Selection, isolation: NodeId| {
        let c = ToolContext { isolation: Some(isolation), ..cx(d, s, &p) };
        let mut t = SelectionTool::default();
        assert!(t.pointer(&c, &ev(PointerKind::Down, 150.0, 150.0)).is_empty());
        t.pointer(&c, &ev(PointerKind::Up, 150.0, 150.0))
    };
    let path = |id: NodeId, r: Rect| Node::path(id, shapes::rectangle(r), Appearance::default_art());
    let (under, beside) = (Rect::new(100.0, 100.0, 200.0, 200.0), Rect::new(300.0, 300.0, 340.0, 340.0));
    // An isolated layer whose art under the click is in a sublayer.
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let (sub, a, b) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    d.insert(Some(l), 0, Node::layer(sub, "Sub", LayerColor::Preset(1))).unwrap();
    d.insert(Some(sub), 0, path(a, under)).unwrap();
    d.insert(Some(l), 1, path(b, beside)).unwrap();
    let mut s = Selection::default();
    s.add(sub);
    s.add(b);
    assert!(click(&d, &s, l).is_empty(), "a sublayer is not a key object");
    // An isolated group that is also selected.
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let (g, a, b) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    d.insert(Some(l), 0, Node::group(g, vec![])).unwrap();
    d.insert(Some(g), 0, path(a, under)).unwrap();
    d.insert(Some(g), 1, path(b, beside)).unwrap();
    let mut s = Selection::default();
    s.add(a);
    s.add(g);
    assert!(click(&d, &s, g).is_empty(), "an object inside the selected group is not a key object");
    s.key = Some(a);
    assert_eq!(click(&d, &s, g), vec![Action::Exec("select.key".into(), json!({}))], "the key again: no key");
    // With the group not selected, the object becomes the key.
    let mut s = Selection::default();
    s.add(a);
    s.add(b);
    assert_eq!(click(&d, &s, g), vec![Action::Exec("select.key".into(), json!({"id": a.0}))]);
    // An isolated compound path. Make Compound Path clears its members' appearance, so these
    // members have none, and the click is on a's outline.
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let (c, a, b) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    let member = |id: NodeId, r: Rect| std::sync::Arc::new(Node::path(id, shapes::rectangle(r), Appearance::default()));
    let children = vec![member(a, Rect::new(150.0, 100.0, 250.0, 200.0)), member(b, beside)];
    let mut compound = Node::new(c, NodeKind::Compound { children, rule: Default::default() });
    compound.appearance = Appearance::default_art().into();
    d.insert(Some(l), 0, compound).unwrap();
    let mut s = Selection::default();
    s.add(a);
    s.add(b);
    assert!(click(&d, &s, c).is_empty(), "a compound-path member is not a key object");
}

#[test]
fn marquee_selects() {
    let (d, id) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0));
    t.pointer(&cx, &ev(PointerKind::Drag, 120.0, 120.0));
    assert_eq!(t.overlays(&cx).len(), 1);
    let a = t.pointer(&cx, &ev(PointerKind::Up, 120.0, 120.0));
    assert_eq!(a, vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
}

/// Escape during a marquee drag ends it: the marquee is removed, and its release selects nothing.
#[test]
fn escape_cancels_a_marquee() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0));
    t.pointer(&cx, &ev(PointerKind::Drag, 120.0, 120.0));
    assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel], "Escape cancels the marquee");
    assert!(!t.busy());
    assert!(t.overlays(&cx).iter().all(|o| !matches!(o, Overlay::Marquee(_))), "the marquee is removed");
    assert!(t.pointer(&cx, &ev(PointerKind::Up, 120.0, 120.0)).is_empty(), "the release selects nothing");
}

/// Shift-drag a marquee: the objects it reaches toggle (#483), so selected ones can be
/// taken out of the selection; a Shift-click on empty canvas leaves the selection alone.
#[test]
fn shift_marquee_toggles() {
    let (d, id) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    let shift = Mods { shift: true, ..Default::default() };
    t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0).with_mods(shift));
    t.pointer(&cx, &ev(PointerKind::Drag, 120.0, 120.0).with_mods(shift));
    let a = t.pointer(&cx, &ev(PointerKind::Up, 120.0, 120.0).with_mods(shift));
    assert_eq!(a, vec![Action::Exec("select.toggle".into(), json!({"ids": [id.0]}))]);
    t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0).with_mods(shift));
    assert!(t.pointer(&cx, &ev(PointerKind::Up, 50.0, 50.0).with_mods(shift)).is_empty());
}

/// E pressed during a marquee drag makes the marquee take only the objects entirely inside it
/// (a group whole), and E pressed again makes it take what it touches. Only a marquee takes E.
#[test]
fn e_during_a_marquee_takes_only_the_objects_inside() {
    use vectorcraft_doc::{Appearance, Node};
    let (mut d, inside) = doc_with_rect();
    let l = d.layers[0].id;
    let rect = |id, r| Node::path(id, vectorcraft_geom::shapes::rectangle(r), Appearance::default_art());
    let (touched, g, a, b) = (d.alloc_id(), d.alloc_id(), d.alloc_id(), d.alloc_id());
    d.insert(Some(l), 1, rect(touched, Rect::new(250.0, 100.0, 350.0, 200.0))).unwrap();
    // A group with one member inside the marquee and one outside it.
    d.insert(Some(l), 2, Node::group(g, vec![])).unwrap();
    d.insert(Some(g), 0, rect(a, Rect::new(100.0, 220.0, 120.0, 240.0))).unwrap();
    d.insert(Some(g), 1, rect(b, Rect::new(100.0, 300.0, 120.0, 320.0))).unwrap();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let e = ToolKey::Letter('e');
    let drag = |presses: usize, mods: Mods| {
        let mut t = SelectionTool::default();
        assert!(!t.claims_key(&cx, e), "no marquee yet");
        t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0).with_mods(mods));
        t.pointer(&cx, &ev(PointerKind::Drag, 300.0, 250.0).with_mods(mods));
        for _ in 0..presses {
            assert!(t.claims_key(&cx, e));
            assert!(t.key(&cx, e, Mods::default()).is_empty());
        }
        let out = t.pointer(&cx, &ev(PointerKind::Up, 300.0, 250.0).with_mods(mods));
        assert!(!t.claims_key(&cx, e), "the marquee is over");
        out
    };
    let set = |ids: &[NodeId]| vec![Action::Exec("select.set".into(), json!({ "ids": json_ids(ids) }))];
    assert_eq!(drag(0, Mods::default()), set(&[inside, touched, g]));
    assert_eq!(drag(1, Mods::default()), set(&[inside]));
    assert_eq!(drag(2, Mods::default()), set(&[inside, touched, g]), "E again: what it touches");
    let shift = Mods { shift: true, ..Mods::default() };
    assert_eq!(drag(1, shift), vec![Action::Exec("select.toggle".into(), json!({ "ids": [inside.0] }))]);
    // The next marquee starts out taking what it touches.
    let mut t = SelectionTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0));
    t.pointer(&cx, &ev(PointerKind::Drag, 300.0, 250.0));
    t.key(&cx, e, Mods::default());
    t.pointer(&cx, &ev(PointerKind::Up, 300.0, 250.0));
    t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 300.0, 250.0)), set(&[inside, touched, g]));
    // A press on an object starts a move, which does not take E.
    let mut t = SelectionTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0));
    assert!(t.busy() && !t.claims_key(&cx, e));
}

/// With E pressed during the drag, a marquee takes only the artboard guides that lie entirely
/// inside it, and no canvas guide, which runs across the whole canvas.
#[test]
fn e_during_a_marquee_takes_only_the_guides_inside() {
    let (mut d, id) = doc_with_rect();
    let board = d.artboards[0].id;
    d.guides.push(vectorcraft_doc::Guide::new(true, 150.0));
    d.guides.push(vectorcraft_doc::Guide { artboard: Some(board), ..vectorcraft_doc::Guide::new(false, 150.0) });
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let marquee = |to: f64| {
        let mut t = SelectionTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, -10.0, -10.0));
        t.pointer(&cx, &ev(PointerKind::Drag, to, to));
        t.key(&cx, ToolKey::Letter('e'), Mods::default());
        t.pointer(&cx, &ev(PointerKind::Up, to, to))
    };
    let guides = |a: &[Action]| a.iter().find_map(|a| matches!(a, Action::Exec(c, _) if c == "guide.select").then(|| a.clone()));
    // The artboard runs from 0 to 500: a marquee to (510, 510) holds its guide.
    let a = marquee(510.0);
    assert_eq!(a.first(), Some(&Action::Exec("select.set".into(), json!({ "ids": [id.0] }))));
    assert_eq!(guides(&a), Some(Action::Exec("guide.select".into(), json!({ "indexes": [1], "toggle": true }))));
    // A marquee to (250, 250) crosses both guides and holds neither.
    assert_eq!(guides(&marquee(250.0)), None);
}

#[test]
fn marquee_selects_ruler_guides() {
    let (mut d, id) = doc_with_rect();
    d.guides.push(vectorcraft_doc::Guide::new(false, 80.0));
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0));
    t.pointer(&cx, &ev(PointerKind::Drag, 120.0, 120.0));
    let a = t.pointer(&cx, &ev(PointerKind::Up, 120.0, 120.0));
    assert_eq!(
        a,
        vec![Action::Exec("select.set".into(), json!({"ids": [id.0]})), Action::Exec("guide.select".into(), json!({"indexes": [0], "toggle": true})),]
    );
}

#[test]
fn marquee_selects_only_guide() {
    let (mut d, _) = doc_with_rect();
    d.guides.push(vectorcraft_doc::Guide::new(true, 20.0));
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, 10.0, 10.0));
    t.pointer(&cx, &ev(PointerKind::Drag, 30.0, 30.0));
    let a = t.pointer(&cx, &ev(PointerKind::Up, 30.0, 30.0));
    assert_eq!(
        a,
        vec![Action::Exec("select.set".into(), json!({"ids": []})), Action::Exec("guide.select".into(), json!({"indexes": [0], "toggle": true})),]
    );
}

#[test]
fn marquee_ignores_locked_guides() {
    let (mut d, id) = doc_with_rect();
    d.guides.push(vectorcraft_doc::Guide::new(false, 80.0));
    let s = Selection::default();
    let p = paint();
    let mut cx_locked = cx(&d, &s, &p);
    cx_locked.guides = false;
    let mut t = SelectionTool::default();
    t.pointer(&cx_locked, &ev(PointerKind::Down, 50.0, 50.0));
    t.pointer(&cx_locked, &ev(PointerKind::Drag, 120.0, 120.0));
    let a = t.pointer(&cx_locked, &ev(PointerKind::Up, 120.0, 120.0));
    assert_eq!(a, vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
}

/// #3036: with Lock Guides on, a guide object neither takes the press nor stops a marquee
/// from starting over it; with guides unlocked, the press still picks it.
#[test]
fn marquee_passes_through_locked_guide_objects() {
    let (mut d, id) = doc_with_rect();
    let l = d.layers[0].id;
    let g = d.alloc_id();
    let pts = [Point::new(20.0, 20.0), Point::new(400.0, 20.0), Point::new(400.0, 400.0), Point::new(20.0, 400.0)];
    let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&pts, true));
    let mut guide = vectorcraft_doc::Node::path(g, path, vectorcraft_doc::Appearance::default_art());
    let vectorcraft_doc::NodeKind::Path { guide: flag, .. } = &mut guide.kind else { panic!("a path") };
    *flag = true;
    d.insert(Some(l), 1, guide).unwrap();
    let s = Selection::default();
    let p = paint();
    let locked = ToolContext { guides: false, ..cx(&d, &s, &p) };
    for (x, y) in [(20.0, 60.0), (50.0, 50.0)] {
        let mut t = SelectionTool::default();
        assert!(t.pointer(&locked, &ev(PointerKind::Down, x, y)).is_empty(), "press at ({x}, {y})");
        t.pointer(&locked, &ev(PointerKind::Drag, 250.0, 250.0));
        assert!(matches!(t.state, State::Marquee { .. }), "press at ({x}, {y})");
        // The rectangle is rubber-band selected (the engine leaves locked guides out).
        let a = t.pointer(&locked, &ev(PointerKind::Up, 250.0, 250.0));
        let Some(Action::Exec(c, v)) = a.first() else { panic!("press at ({x}, {y}): {a:?}") };
        assert_eq!(c, "select.set", "press at ({x}, {y})");
        assert!(v["ids"].as_array().unwrap().contains(&json!(id.0)), "press at ({x}, {y}): {a:?}");
    }
    let unlocked = ToolContext { guides: true, ..cx(&d, &s, &p) };
    let mut t = SelectionTool::default();
    assert_eq!(t.pointer(&unlocked, &ev(PointerKind::Down, 20.0, 60.0)), vec![Action::Exec("select.set".into(), json!({"ids": [g.0]}))]);
}

#[test]
fn handle_drag_scales() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    assert_eq!(t.cursor(&cx, Point::new(200.0, 200.0), Mods::default()), Cursor::ResizeNwSe);
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 200.0, 200.0)), vec![Action::Begin("Scale".into())]);
    let a = t.pointer(&cx, &ev(PointerKind::Drag, 300.0, 300.0));
    assert!(matches!(&a[0], Action::Preview(_, v) if v["matrix"][0] == 2.0));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 300.0, 300.0)), vec![Action::Commit]);
}

#[test]
fn backquote_move_uses_release_state_and_one_commit() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0));
    let drag = t.pointer(&cx, &PointerEvent { backquote: true, ..ev(PointerKind::Drag, 170.0, 150.0) });
    assert!(matches!(drag.as_slice(), [Action::Begin(_), Action::Preview(_, v)] if v["objects"] == false && v["patterns"] == true));
    let up = t.pointer(&cx, &PointerEvent { backquote: true, ..ev(PointerKind::Up, 170.0, 150.0) });
    assert!(matches!(up.as_slice(), [Action::Preview(_, v), Action::Commit] if v["objects"] == false && v["patterns"] == true));
    let mut t = SelectionTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0));
    t.pointer(&cx, &PointerEvent { backquote: true, ..ev(PointerKind::Drag, 170.0, 150.0) });
    let up = t.pointer(&cx, &ev(PointerKind::Up, 170.0, 150.0));
    assert!(matches!(up.as_slice(), [Action::Preview(_, v), Action::Commit] if v.get("objects").is_none() && v.get("patterns").is_none()));
}

#[test]
fn backquote_box_scale_and_rotate_use_pattern_matrix_at_release() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = ToolContext { preview_bounds: true, ..cx(&d, &s, &p) };
    for (from, to) in [((200.0, 200.0), (300.0, 300.0)), ((210.0, 210.0), (150.0, 230.0))] {
        let mut t = SelectionTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, from.0, from.1));
        let drag = t.pointer(&cx, &PointerEvent { backquote: true, ..ev(PointerKind::Drag, to.0, to.1) });
        assert!(
            matches!(drag.as_slice(), [Action::Preview(c, v)] if c == "object.transform" && v["objects"] == false && v["patterns"] == true),
            "{drag:?}"
        );
        let up = t.pointer(&cx, &PointerEvent { backquote: true, ..ev(PointerKind::Up, to.0, to.1) });
        assert!(
            matches!(up.as_slice(), [Action::Preview(c, v), Action::Commit] if c == "object.transform" && v["objects"] == false && v["patterns"] == true),
            "{up:?}"
        );
        let mut t = SelectionTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, from.0, from.1));
        t.pointer(&cx, &PointerEvent { backquote: true, ..ev(PointerKind::Drag, to.0, to.1) });
        let up = t.pointer(&cx, &ev(PointerKind::Up, to.0, to.1));
        assert!(
            matches!(up.as_slice(), [Action::Preview(_, v), Action::Commit] if v.get("objects").is_none() && v.get("patterns").is_none()),
            "{up:?}"
        );
    }
}

/// Escape during a bounding-box scale or rotation cancels it, and the release after it commits
/// nothing.
#[test]
fn escape_cancels_a_scale_or_a_rotation() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    // (what, the press, its Begin, the drag)
    for (what, from, begin, to) in [("scale", (200.0, 200.0), "Scale", (300.0, 300.0)), ("rotate", (210.0, 210.0), "Rotate", (150.0, 230.0))] {
        let mut t = SelectionTool::default();
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, from.0, from.1)), vec![Action::Begin(begin.into())], "{what}");
        assert!(matches!(t.pointer(&cx, &ev(PointerKind::Drag, to.0, to.1)).as_slice(), [Action::Preview(..)]), "{what}");
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel], "{what}: Escape cancels the drag");
        assert!(!t.busy() && !t.transforming(), "{what}");
        assert!(t.overlays(&cx).iter().all(|o| !matches!(o, Overlay::Measure { .. })), "{what}: no readout stays");
        assert!(t.pointer(&cx, &ev(PointerKind::Drag, to.0 + 5.0, to.1)).is_empty(), "{what}: the drag is over");
        assert!(t.pointer(&cx, &ev(PointerKind::Up, to.0 + 5.0, to.1)).is_empty(), "{what}: the release commits nothing");
    }
}

#[test]
fn handle_drag_snaps_to_the_other_objects_with_smart_guides() {
    use vectorcraft_doc::{Appearance, Node};
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let r = Rect::new(250.0, 100.0, 300.0, 150.0);
    d.insert(Some(l), 1, Node::path(id, vectorcraft_geom::shapes::rectangle(r), Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let mut c = cx(&d, &s, &p);
    let height = |a: &[Action]| {
        let Action::Preview(_, v) = &a[0] else { panic!("{a:?}") };
        v["matrix"][3].as_f64().unwrap() * 50.0
    };
    // The bottom handle, dragged near the neighbour's bottom edge (y = 200), lands on it.
    let mut t = SelectionTool::default();
    t.pointer(&c, &ev(PointerKind::Down, 275.0, 150.0));
    let a = t.pointer(&c, &ev(PointerKind::Drag, 275.0, 197.0));
    assert!((height(&a) - 100.0).abs() < 1e-9, "{a:?}");
    assert!(t.overlays(&c).iter().any(|o| matches!(o, Overlay::Line { .. })));
    t.pointer(&c, &ev(PointerKind::Up, 275.0, 197.0));
    assert!(!t.overlays(&c).iter().any(|o| matches!(o, Overlay::Line { .. })));
    // Smart Guides off: the edge follows the pointer.
    c.smart_guides = false;
    t.pointer(&c, &ev(PointerKind::Down, 275.0, 150.0));
    let a = t.pointer(&c, &ev(PointerKind::Drag, 275.0, 197.0));
    assert!((height(&a) - 97.0).abs() < 1e-9, "{a:?}");
}

/// The box the 100–200 rectangle's bounds become under the scale a handle drag previews.
fn scaled(a: &[Action]) -> Rect {
    let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
    let m: Vec<f64> = v["matrix"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
    Affine::new([m[0], m[1], m[2], m[3], m[4], m[5]]).transform_rect_bbox(Rect::new(100.0, 100.0, 200.0, 200.0))
}

fn near(a: Rect, b: Rect) -> bool {
    [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|d| d.abs() < 1e-9)
}

/// #2054: with Snap to Grid on, a handle drag lands what the handle moves on the nearest
/// gridline (every 9 pt by default) while the opposite side stays put; Shift keeps the
/// proportions, and Alt keeps the center.
#[test]
fn handle_drag_lands_what_it_moves_on_the_grid() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let c = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
    let drag = |from: (f64, f64), to: (f64, f64), mods: Mods| {
        let mut t = SelectionTool::default();
        assert_eq!(t.pointer(&c, &ev(PointerKind::Down, from.0, from.1).with_mods(mods)), vec![Action::Begin("Scale".into())]);
        scaled(&t.pointer(&c, &ev(PointerKind::Drag, to.0, to.1).with_mods(mods)))
    };
    let none = Mods::default();
    // The right side, dragged to x = 213, lands on the gridline at 216.
    let r = drag((200.0, 150.0), (213.0, 150.0), none);
    assert!(near(r, Rect::new(100.0, 100.0, 216.0, 200.0)), "{r:?}");
    // The bottom-right corner, dragged to (213, 204), lands on (216, 207).
    let r = drag((200.0, 200.0), (213.0, 204.0), none);
    assert!(near(r, Rect::new(100.0, 100.0, 216.0, 207.0)), "{r:?}");
    // Shift: the corner, at (213, 213) in proportion, lands on (216, 216), still square.
    let r = drag((200.0, 200.0), (213.0, 204.0), Mods { shift: true, ..none });
    assert!(near(r, Rect::new(100.0, 100.0, 216.0, 216.0)), "{r:?}");
    // Alt: the right side lands on 216 and the left side mirrors it about the center.
    let r = drag((200.0, 150.0), (213.0, 150.0), Mods { alt: true, ..none });
    assert!(near(r, Rect::new(84.0, 100.0, 216.0, 200.0)), "{r:?}");
}

/// #2054: with Snap to Pixel on, what a handle drag moves lands on whole pixels.
#[test]
fn handle_drag_lands_what_it_moves_on_whole_pixels() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let c = ToolContext { snap_to_pixel: true, ..cx(&d, &s, &p) };
    let mut t = SelectionTool::default();
    t.pointer(&c, &ev(PointerKind::Down, 200.0, 200.0));
    let r = scaled(&t.pointer(&c, &ev(PointerKind::Drag, 212.4, 230.7)));
    assert!(near(r, Rect::new(100.0, 100.0, 212.0, 231.0)), "{r:?}");
}

/// #2054: with Smart Guides off, View › Snap to Point lands a dragged handle on another
/// object's anchor or a ruler guide within its distance (2 px); with Smart Guides on, the
/// ruler guides still pull it within the Snapping Tolerance (4 px), as a move does (#1534).
#[test]
fn handle_drag_snaps_to_anchors_and_ruler_guides_with_snap_to_point() {
    use vectorcraft_doc::{Appearance, Node};
    let (mut d, id) = doc_with_rect();
    let l = d.layers[0].id;
    let other = d.alloc_id();
    let sq = vectorcraft_geom::shapes::rectangle(Rect::new(250.0, 250.0, 300.0, 300.0));
    d.insert(Some(l), 1, Node::path(other, sq, Appearance::default_art())).unwrap();
    d.guides = vec![vectorcraft_doc::Guide::new(true, 230.5)];
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let drag = |c: &ToolContext, from: (f64, f64), to: (f64, f64)| {
        let mut t = SelectionTool::default();
        t.pointer(c, &ev(PointerKind::Down, from.0, from.1));
        scaled(&t.pointer(c, &ev(PointerKind::Drag, to.0, to.1)))
    };
    let off = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    assert!(off.snap_to_point);
    // The corner, 1.4 px from the other square's corner, lands on it.
    let r = drag(&off, (200.0, 200.0), (249.0, 251.0));
    assert!(near(r, Rect::new(100.0, 100.0, 250.0, 250.0)), "{r:?}");
    // The right side, 1 px from the ruler guide, lands on it.
    let r = drag(&off, (200.0, 150.0), (231.5, 150.0));
    assert!(near(r, Rect::new(100.0, 100.0, 230.5, 200.0)), "{r:?}");
    // Snap to Point off: the side follows the pointer.
    let r = drag(&ToolContext { snap_to_point: false, ..off }, (200.0, 150.0), (231.5, 150.0));
    assert!(near(r, Rect::new(100.0, 100.0, 231.5, 200.0)), "{r:?}");
    // Smart Guides on: the ruler guide, 2.5 px away, pulls the side too.
    let r = drag(&cx(&d, &s, &p), (200.0, 150.0), (233.0, 150.0));
    assert!(near(r, Rect::new(100.0, 100.0, 230.5, 200.0)), "{r:?}");
}

/// Smart Guides › Measurement Labels, Transform Tools and Snapping Tolerance (#394): the
/// readouts while moving and while scaling each follow their option; the tolerance sets how
/// far a dragged handle is pulled.
#[test]
fn smart_guide_display_preferences_drive_the_readouts_and_the_pull() {
    use vectorcraft_doc::{Appearance, Node};
    let (mut d, _) = doc_with_rect();
    let l = d.layers[0].id;
    let id = d.alloc_id();
    d.insert(Some(l), 1, Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(250.0, 100.0, 300.0, 150.0)), Appearance::default_art()))
        .unwrap();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let measures = |t: &SelectionTool, c: &ToolContext| t.overlays(c).iter().filter(|o| matches!(o, Overlay::Measure { .. })).count();
    let height = |a: &[Action]| {
        let Action::Preview(_, v) = &a[0] else { panic!("{a:?}") };
        v["matrix"][3].as_f64().unwrap() * 50.0
    };
    // Moving: the offset readout is a measurement label.
    for on in [true, false] {
        let c = ToolContext { measurement_labels: on, ..cx(&d, &s, &p) };
        let mut t = SelectionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 275.0, 125.0));
        t.pointer(&c, &ev(PointerKind::Drag, 285.0, 135.0));
        assert_eq!(measures(&t, &c), usize::from(on), "measurement labels {on}");
        t.pointer(&c, &ev(PointerKind::Up, 285.0, 135.0));
    }
    // Scaling by a handle: the size readout belongs to Transform Tools.
    for on in [true, false] {
        let c = ToolContext { transform_tools_guides: on, ..cx(&d, &s, &p) };
        let mut t = SelectionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 275.0, 150.0));
        t.pointer(&c, &ev(PointerKind::Drag, 275.0, 180.0));
        assert_eq!(measures(&t, &c), usize::from(on), "transform tools {on}");
        t.pointer(&c, &ev(PointerKind::Up, 275.0, 180.0));
    }
    // Snapping Tolerance: 6 px from the neighbour's bottom edge is beyond 4, within 8.
    for (tol, want) in [(4.0, 94.0), (8.0, 100.0), (0.0, 94.0)] {
        let c = ToolContext { snapping_tolerance: tol, ..cx(&d, &s, &p) };
        let mut t = SelectionTool::default();
        t.pointer(&c, &ev(PointerKind::Down, 275.0, 150.0));
        let a = t.pointer(&c, &ev(PointerKind::Drag, 275.0, 194.0));
        assert!((height(&a) - want).abs() < 1e-9, "tolerance {tol}: {a:?}");
        t.pointer(&c, &ev(PointerKind::Up, 275.0, 194.0));
    }
}

#[test]
fn handle_drag_of_a_turned_box_snaps_on_the_page() {
    use vectorcraft_doc::{Appearance, Node};
    let matrix = |a: &[Action]| {
        let Action::Preview(_, v) = &a[0] else { panic!("{a:?}") };
        let m: Vec<f64> = v["matrix"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        Affine::new([m[0], m[1], m[2], m[3], m[4], m[5]])
    };
    // A 50 pt square beside the 100..200 rect, turned about its centre (275, 125).
    for (turn, low) in [(90.0_f64, 150.0), (45.0, 125.0 + 25.0 * std::f64::consts::SQRT_2)] {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let r = Rect::new(250.0, 100.0, 300.0, 150.0);
        d.insert(Some(l), 1, Node::path(id, vectorcraft_geom::shapes::rectangle(r), Appearance::default_art())).unwrap();
        let c = Point::new(275.0, 125.0);
        d.node_mut(id)
            .unwrap()
            .transform(Affine::translate(c.to_vec2()) * Affine::rotate(turn.to_radians()) * Affine::translate(-c.to_vec2()), false);
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let bx = selection_box(&cx).unwrap();
        assert_ne!(bx.angle, 0.0);
        // The handle at the lowest point of the shape: a side at 90°, a corner at 45°.
        let grab = Point::new(275.0, low);
        assert!(Handle::ALL.iter().any(|h| (bx.to_doc() * h.pos(bx.rect)).distance(grab) < 1e-6), "{bx:?}");
        let mut t = SelectionTool::default();
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, grab.x, grab.y)), vec![Action::Begin("Scale".into())]);
        // Dragged near the neighbour's bottom edge (y = 200), it lands on it.
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 275.0, 197.0));
        let landed = matrix(&a) * grab;
        assert!((landed.y - 200.0).abs() < 1e-6 && (landed.x - 275.0).abs() < 1e-6, "{turn}: {landed:?}");
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Line { .. })));
        t.pointer(&cx, &ev(PointerKind::Up, 275.0, 197.0));
    }
}

#[test]
fn handle_drag_on_area_type_resizes_its_frame() {
    let (d, text) = doc_with_area_type();
    let p = paint();
    let mut s = Selection::default();
    s.add(text);
    let cx1 = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    // Bottom-right handle of the 120 × 40 frame at (300, 300).
    assert_eq!(t.pointer(&cx1, &ev(PointerKind::Down, 420.0, 340.0)), vec![Action::Begin("Resize Type Area".into())]);
    let a = t.pointer(&cx1, &ev(PointerKind::Drag, 460.0, 400.0));
    assert!(matches!(&a[0], Action::Preview(c, v) if c == "object.transform" && v["typeAreas"] == true), "{a:?}");
    assert_eq!(t.pointer(&cx1, &ev(PointerKind::Up, 460.0, 400.0)), vec![Action::Commit]);
    // With another object the step is a scale, still resizing the type area.
    let rect = d.layers[0].children().unwrap()[0].id;
    s.add(rect);
    let cx2 = cx(&d, &s, &p);
    assert_eq!(t.pointer(&cx2, &ev(PointerKind::Down, 420.0, 340.0)), vec![Action::Begin("Scale".into())]);
    let a = t.pointer(&cx2, &ev(PointerKind::Drag, 460.0, 400.0));
    assert!(matches!(&a[0], Action::Preview(_, v) if v["typeAreas"] == true), "{a:?}");
    t.pointer(&cx2, &ev(PointerKind::Up, 460.0, 400.0));
    // Without area type the drag scales as before.
    let mut s = Selection::default();
    s.add(rect);
    let cx3 = cx(&d, &s, &p);
    t.pointer(&cx3, &ev(PointerKind::Down, 200.0, 200.0));
    let a = t.pointer(&cx3, &ev(PointerKind::Drag, 300.0, 300.0));
    assert!(matches!(&a[0], Action::Preview(_, v) if v.get("typeAreas").is_none()), "{a:?}");
}

#[test]
fn turned_box_handles_turn_and_scale_along_the_object() {
    let (mut d, id) = doc_with_rect();
    let c = Point::new(150.0, 150.0);
    let turn = Affine::translate(c.to_vec2()) * Affine::rotate(-std::f64::consts::FRAC_PI_4) * Affine::translate(-c.to_vec2());
    d.node_mut(id).unwrap().transform(turn, false);
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    // The Right handle sits up and to the right, where a diagonal cursor fits it.
    let k = 50.0 * std::f64::consts::FRAC_1_SQRT_2;
    let right = Point::new(150.0 + k, 150.0 - k);
    let mut t = SelectionTool::default();
    assert_eq!(t.cursor(&cx, right, Mods::default()), Cursor::ResizeNeSw);
    // The page box's corner is no handle any more.
    assert_ne!(t.cursor(&cx, Point::new(150.0 + 2.0 * k, 150.0 - 2.0 * k), Mods::default()), Cursor::ResizeNeSw);
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, right.x, right.y)), vec![Action::Begin("Scale".into())]);
    let a = t.pointer(&cx, &ev(PointerKind::Drag, right.x + k, right.y - k));
    let Action::Preview(_, v) = &a[0] else { panic!("{a:?}") };
    // Doubling the width along 45° about the left side: the left handle stays put.
    let m: Vec<f64> = v["matrix"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
    let m = Affine::new([m[0], m[1], m[2], m[3], m[4], m[5]]);
    let left = Point::new(150.0 - k, 150.0 + k);
    assert!((m * left).distance(left) < 1e-9);
    assert!((m * right).distance(Point::new(right.x + k, right.y - k)) < 1e-9);
}

#[test]
fn click_empty_deselects() {
    let (d, _) = doc_with_rect();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, 400.0, 400.0));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 400.0, 400.0)), vec![Action::Exec("select.none".into(), json!({}))]);
}

/// [`doc_with_area_type`] with more text than its frame holds, selected; the rectangle's id and
/// where the frame's out port is.
fn overflowing_area_type() -> (vectorcraft_doc::Document, NodeId, NodeId, Point) {
    let (mut d, id) = doc_with_area_type();
    let rect = d.children(Some(d.layers[0].id)).unwrap()[0].id;
    let Some(vectorcraft_doc::NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) else { panic!("not text") };
    let style = vectorcraft_doc::CharStyle { size: 20.0, ..Default::default() };
    t.runs = vec![vectorcraft_doc::TextRun::new("Words that fill more than the frame holds. ".repeat(20), style)];
    // On the frame's right side, above its bottom-right corner (120 × 40 at 100%).
    let port = t.xf * Point::new(120.0, 40.0 - crate::text::PORT_RISE_PX);
    (d, id, rect, port)
}

/// Type on a path whose text overflows has its out port just past its end bracket, along the
/// path: a click there loads the pointer with its text, as area type's does.
#[test]
fn the_out_port_of_type_on_a_path_loads_its_text() {
    let (mut d, id) = doc_with_area_type();
    let Some(vectorcraft_doc::NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) else { panic!("not text") };
    let line = vectorcraft_geom::kurbo::Line::new((0.0, 200.0), (100.0, 200.0));
    t.kind = vectorcraft_doc::TextKind::OnPath {
        path: vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::kurbo::Shape::to_path(&line, 0.1)),
        start: 0.0,
        end: Some(0.8),
    };
    t.xf = vectorcraft_geom::Affine::IDENTITY;
    t.runs = vec![vectorcraft_doc::TextRun::new("Words that run past the end of the path. ".repeat(5), vectorcraft_doc::CharStyle::default())];
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    // The end bracket stands at 80, 200; the port past it, along the path.
    let port = Point::new(80.0 + crate::text::PORT_RISE_PX, 200.0);
    assert!(t.pointer(&cx, &ev(PointerKind::Down, port.x, port.y)).is_empty());
    assert!(t.pointer(&cx, &ev(PointerKind::Up, port.x, port.y)).is_empty());
    assert_eq!(t.cursor(&cx, Point::new(30.0, 420.0), Mods::default()), Cursor::LoadedText);
    assert!(t.pointer(&cx, &ev(PointerKind::Down, 30.0, 420.0)).is_empty());
    assert_eq!(
        t.pointer(&cx, &ev(PointerKind::Up, 30.0, 420.0)),
        vec![Action::Exec("text.thread.add".into(), json!({"from": id.0, "at": [30.0, 420.0]}))]
    );
}

/// #2572: a click on the out port loads the pointer with the text; a click on empty canvas then
/// threads it into a new frame there, a click on a closed path into the path, a drag into the
/// rectangle it draws.
#[test]
fn a_click_on_the_out_port_loads_the_text_and_the_next_click_or_drag_threads_it() {
    let (d, id, rect, port) = overflowing_area_type();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    let thread = |params: Value| vec![Action::Exec("text.thread.add".into(), params)];
    let load = |t: &mut SelectionTool| {
        assert!(t.pointer(&cx, &ev(PointerKind::Down, port.x, port.y)).is_empty());
        assert!(t.pointer(&cx, &ev(PointerKind::Up, port.x, port.y)).is_empty());
        assert_eq!(t.cursor(&cx, Point::new(30.0, 420.0), Mods::default()), Cursor::LoadedText);
    };
    load(&mut t);
    assert!(t.pointer(&cx, &ev(PointerKind::Down, 30.0, 420.0)).is_empty());
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 30.0, 420.0)), thread(json!({"from": id.0, "at": [30.0, 420.0]})));
    assert_ne!(t.cursor(&cx, Point::new(30.0, 420.0), Mods::default()), Cursor::LoadedText, "the text is placed");
    load(&mut t);
    t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 150.0, 150.0)), thread(json!({"from": id.0, "to": rect.0})));
    load(&mut t);
    t.pointer(&cx, &ev(PointerKind::Down, 20.0, 20.0));
    t.pointer(&cx, &ev(PointerKind::Drag, 80.0, 90.0));
    assert!(t.overlays(&cx).contains(&Overlay::Marquee(Rect::new(20.0, 20.0, 80.0, 90.0))), "the new frame's outline");
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 80.0, 90.0)), thread(json!({"from": id.0, "rect": [20.0, 20.0, 60.0, 70.0]})));
    // The bounding box's corner handle below the port still resizes the frame.
    let corner = Point::new(port.x, port.y + crate::text::PORT_RISE_PX);
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, corner.x, corner.y)), vec![Action::Begin("Resize Type Area".into())]);
}

/// Loaded text clicked just inside a layer's clipping mask, where art is drawn over the mask's
/// edge, threads into that art, not into the unpainted mask: placing text acts on what is drawn,
/// so it takes the ordinary hit, not the selection tools' clipping-path outline.
#[test]
fn loaded_text_threads_into_the_art_on_a_layer_masks_edge() {
    let (mut d, id, _, port) = overflowing_area_type();
    let art = d.alloc_id();
    let picture = vectorcraft_doc::Node::path(
        art,
        vectorcraft_geom::shapes::rectangle(Rect::new(350.0, 350.0, 550.0, 550.0)),
        vectorcraft_doc::Appearance::default_art(),
    );
    add_masked_layer(&mut d, Rect::new(400.0, 400.0, 500.0, 500.0), picture);
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let cx = cx(&d, &s, &p);
    assert!(cx.selection_hit_options().clip_outlines, "the selection tools would take the mask here");
    let mut t = SelectionTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, port.x, port.y));
    t.pointer(&cx, &ev(PointerKind::Up, port.x, port.y));
    t.pointer(&cx, &ev(PointerKind::Down, 498.5, 450.0));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 498.5, 450.0)), vec![Action::Exec("text.thread.add".into(), json!({"from": id.0, "to": art.0}))]);
}

/// A double-click on type where a layer's clipping mask edge runs through it edits the type,
/// though a press there selects the mask: what a double-click edits is what is drawn.
#[test]
fn a_double_click_on_type_at_a_layer_masks_edge_edits_it() {
    let (d, _, at) = doc_with_type_on_a_layer_masks_edge();
    let (s, p) = (Selection::default(), paint());
    let a = SelectionTool::default().pointer(&cx(&d, &s, &p), &ev(PointerKind::DoubleClick, at.x, at.y));
    assert_eq!(a, vec![Action::SwitchTool("type".into())]);
}

/// A Cmd/Ctrl+Shift double-click releases a guide path where it crosses a layer's clipping
/// mask edge (its left side, x = 450, just inside the mask's bottom): the mask, unpainted and
/// never a guide, doesn't stand in front of it.
#[test]
fn a_guide_on_a_layer_masks_edge_is_released() {
    let (mut d, _) = doc_with_rect();
    let id = d.alloc_id();
    let mut guide_path =
        vectorcraft_doc::Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(450.0, 350.0, 550.0, 550.0)), Default::default());
    let vectorcraft_doc::NodeKind::Path { guide, .. } = &mut guide_path.kind else { panic!("a path") };
    *guide = true;
    add_masked_layer(&mut d, Rect::new(400.0, 400.0, 500.0, 500.0), guide_path);
    let (s, p) = (Selection::default(), paint());
    let both = Mods { cmd: true, shift: true, ..Mods::default() };
    let a = SelectionTool::default().pointer(&cx(&d, &s, &p), &ev(PointerKind::DoubleClick, 450.0, 498.5).with_mods(both));
    assert_eq!(a, vec![Action::Exec("view.guides.release".into(), json!({"ids": [id.0]}))]);
}

/// Escape drops the loaded text, and the next click is an ordinary one.
#[test]
fn escape_drops_the_loaded_text() {
    let (d, id, _, port) = overflowing_area_type();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, port.x, port.y));
    t.pointer(&cx, &ev(PointerKind::Up, port.x, port.y));
    assert!(t.claims_key(&cx, ToolKey::Escape), "Escape is the loaded pointer's");
    assert!(t.key(&cx, ToolKey::Escape, Mods::default()).is_empty());
    assert!(!t.claims_key(&cx, ToolKey::Escape));
    assert_ne!(t.cursor(&cx, Point::new(30.0, 420.0), Mods::default()), Cursor::LoadedText);
    t.pointer(&cx, &ev(PointerKind::Down, 30.0, 420.0));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 30.0, 420.0)), vec![Action::Exec("select.none".into(), json!({}))]);
}

/// [`doc_with_area_type`], its text fitting its frame; where its in and out ports are.
fn fitting_area_type() -> (vectorcraft_doc::Document, NodeId, Point, Point) {
    let (d, id) = doc_with_area_type();
    let Some(vectorcraft_doc::NodeKind::Text(t)) = d.node(id).map(|n| &n.kind) else { panic!("not text") };
    assert!(!vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).overflow, "the text fits");
    // On the left side below the top-left corner, on the right side above the bottom-right one.
    let (input, output) = (t.xf * Point::new(0.0, crate::text::PORT_RISE_PX), t.xf * Point::new(120.0, 40.0 - crate::text::PORT_RISE_PX));
    (d, id, input, output)
}

/// #2572: selected area type has both ports to click whether its text fits or not; other
/// points of its frame, and type that isn't selected, have none.
#[test]
fn every_selected_area_type_has_an_in_and_an_out_port() {
    use crate::text::{Port, port_at};
    let (d, id, input, output) = fitting_area_type();
    let p = paint();
    let mut s = Selection::default();
    assert_eq!(port_at(&cx(&d, &s, &p), output), None, "not selected");
    s.set([id]);
    let cx = cx(&d, &s, &p);
    assert_eq!(port_at(&cx, output), Some((id, Port::Out)));
    assert_eq!(port_at(&cx, input), Some((id, Port::In)));
    assert_eq!(port_at(&cx, Point::new(input.x + 4.0, input.y + 4.0)), Some((id, Port::In)), "the box around it");
    assert_eq!(port_at(&cx, Point::new(input.x + 12.0, input.y)), None);
    assert_eq!(port_at(&cx, Point::new(output.x, output.y + crate::text::PORT_RISE_PX)), None, "the corner");
}

/// #2572: a click on the out port of type that fits loads its text as well; the next click
/// threads it into a new frame after it.
#[test]
fn a_click_on_the_out_port_of_type_that_fits_starts_threading() {
    let (d, id, _, output) = fitting_area_type();
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    assert!(t.pointer(&cx, &ev(PointerKind::Down, output.x, output.y)).is_empty());
    assert!(t.pointer(&cx, &ev(PointerKind::Up, output.x, output.y)).is_empty());
    assert_eq!(t.cursor(&cx, Point::new(30.0, 420.0), Mods::default()), Cursor::LoadedText);
    t.pointer(&cx, &ev(PointerKind::Down, 30.0, 420.0));
    assert_eq!(
        t.pointer(&cx, &ev(PointerKind::Up, 30.0, 420.0)),
        vec![Action::Exec("text.thread.add".into(), json!({"from": id.0, "at": [30.0, 420.0]}))]
    );
}

/// #2572: a click on the in port loads the text to thread backwards: the next click on empty
/// canvas, on a closed path or a drag makes the frame the story flows from into this one
/// (`before`). The corner handle above the in port still resizes the frame.
#[test]
fn a_click_on_the_in_port_threads_into_a_frame_before_it() {
    let (d, id, input, _) = fitting_area_type();
    let rect = d.children(Some(d.layers[0].id)).unwrap()[0].id;
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = SelectionTool::default();
    let thread = |params: Value| vec![Action::Exec("text.thread.add".into(), params)];
    let load = |t: &mut SelectionTool| {
        assert!(t.pointer(&cx, &ev(PointerKind::Down, input.x, input.y)).is_empty());
        assert!(t.pointer(&cx, &ev(PointerKind::Up, input.x, input.y)).is_empty());
        assert_eq!(t.cursor(&cx, Point::new(30.0, 420.0), Mods::default()), Cursor::LoadedText);
    };
    load(&mut t);
    t.pointer(&cx, &ev(PointerKind::Down, 30.0, 420.0));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 30.0, 420.0)), thread(json!({"from": id.0, "at": [30.0, 420.0], "before": true})));
    load(&mut t);
    t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 150.0, 150.0)), thread(json!({"from": id.0, "to": rect.0, "before": true})));
    load(&mut t);
    t.pointer(&cx, &ev(PointerKind::Down, 20.0, 20.0));
    t.pointer(&cx, &ev(PointerKind::Drag, 80.0, 90.0));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 80.0, 90.0)), thread(json!({"from": id.0, "rect": [20.0, 20.0, 60.0, 70.0], "before": true})));
    let corner = Point::new(input.x, input.y - crate::text::PORT_RISE_PX);
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, corner.x, corner.y)), vec![Action::Begin("Resize Type Area".into())]);
}

/// A double-click on a threaded port breaks the thread there, its in port before the frame
/// and its out port after it; it doesn't thread the loaded text into a new frame. A
/// double-click on a port with no thread does nothing (it doesn't switch to the Type tool).
#[test]
fn a_double_click_on_a_threaded_port_breaks_the_thread() {
    let (mut d, id, input, output) = fitting_area_type();
    let rect = d.children(Some(d.layers[0].id)).unwrap()[0].id;
    let mut s = Selection::default();
    s.set([id]);
    let p = paint();
    let double_click = |d: &vectorcraft_doc::Document, at: Point| {
        let cx = cx(d, &s, &p);
        let mut t = SelectionTool::default();
        let kinds = [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick];
        let out: Vec<Action> = kinds.into_iter().flat_map(|k| t.pointer(&cx, &ev(k, at.x, at.y))).collect();
        assert_ne!(t.cursor(&cx, Point::new(30.0, 420.0), Mods::default()), Cursor::LoadedText, "no text stays loaded");
        out
    };
    let breaks = |port: &str| vec![Action::Exec("text.thread.break".into(), json!({"id": id.0, "port": port}))];
    d.text_threads = vec![vec![id, rect]];
    assert_eq!(double_click(&d, output), breaks("out"));
    assert_eq!(double_click(&d, input), vec![], "no frame before it");
    d.text_threads = vec![vec![rect, id]];
    assert_eq!(double_click(&d, input), breaks("in"));
    assert_eq!(double_click(&d, output), vec![], "no frame after it");
    d.text_threads.clear();
    assert_eq!(double_click(&d, output), vec![]);
}

/// The memo answers for the revision, the selected objects and Use Preview Bounds it measured
/// for, and measures again when any of them changes.
#[test]
fn the_preview_memo_measures_once_per_revision_selection_and_preview_bounds() {
    let (d, id) = doc_with_rect();
    let mut moved = d.clone();
    if let Some(n) = moved.node_mut(id) {
        n.transform(Affine::translate((50.0, 0.0)), false);
    }
    let (mut s, p, memo) = (Selection::default(), paint(), PreviewMemo::default());
    s.set([id]);
    let at = |d, s, revision, preview_bounds| {
        let c = ToolContext { revision, preview_bounds, preview_memo: Some(&memo), ..cx(d, s, &p) };
        (selection_bounds(&c).map(|r| r.x1), object_bounds(&c, id).map(|r| r.x1))
    };
    assert_eq!(at(&d, &s, (1, 1), false), (Some(200.0), Some(200.0)));
    assert_eq!(memo.measurements(), 2);
    // The same revision with the moved square: the memo answers with what it measured.
    assert_eq!(at(&moved, &s, (1, 1), false), (Some(200.0), Some(200.0)));
    assert_eq!(memo.measurements(), 2);
    assert_eq!(at(&moved, &s, (1, 2), false), (Some(250.0), Some(250.0)), "a new revision");
    assert_eq!(memo.measurements(), 4);
    assert_eq!(at(&moved, &s, (1, 2), true), (Some(250.5), Some(250.5)), "Use Preview Bounds counts the stroke");
    assert_eq!(memo.measurements(), 6);
    let none = Selection::default();
    assert_eq!(at(&moved, &none, (1, 2), true), (None, Some(250.5)), "another selection");
    assert_eq!(memo.measurements(), 8);
    // Without a memo, every call measures.
    let c = cx(&moved, &s, &p);
    assert_eq!(selection_bounds(&c).map(|r| r.x1), Some(250.0));
}
