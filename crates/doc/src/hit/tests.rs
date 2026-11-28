use super::*;
use crate::{Appearance, Node};
use std::sync::Arc;
use vectorcraft_color::{Color, Paint};
use vectorcraft_geom::shapes;

#[test]
fn hits_topmost() {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let a = d.alloc_id();
    d.insert(Some(l), 9, Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art())).unwrap();
    let b = d.alloc_id();
    d.insert(Some(l), 9, Node::path(b, shapes::rectangle(Rect::new(25.0, 25.0, 75.0, 75.0)), Appearance::default_art())).unwrap();
    let h = hit_test(&d, Point::new(30.0, 30.0), HitOptions::default()).unwrap();
    assert_eq!(h.leaf, b);
    assert_eq!(h.kind, HitKind::Fill);
    let h = hit_test(&d, Point::new(10.0, 10.0), HitOptions::default()).unwrap();
    assert_eq!(h.leaf, a);
    assert!(hit_test(&d, Point::new(150.0, 150.0), HitOptions::default()).is_none());
    // Outline mode: interior misses.
    assert!(hit_test(&d, Point::new(10.0, 10.0), HitOptions { outline: true, ..Default::default() }).is_none());
    assert!(hit_test(&d, Point::new(0.5, 10.0), HitOptions { outline: true, ..Default::default() }).is_some());
}

#[test]
fn unfilled_interior_misses() {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let a = d.alloc_id();
    d.insert(
        Some(l),
        9,
        Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0)),
    )
    .unwrap();
    assert!(hit_test(&d, Point::new(25.0, 25.0), HitOptions::default()).is_none());
    assert!(hit_test(&d, Point::new(50.5, 25.0), HitOptions::default()).is_some());
}

/// The selection tools' options with unfilled closed paths hit inside.
const INTERIORS: HitOptions = HitOptions { unfilled_interiors: true, ..SELECTING };

/// A path of `points` with a black stroke and no fill, inserted at the top of the first layer.
fn stroked(d: &mut Document, points: &[(f64, f64)], closed: bool) -> NodeId {
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let pts: Vec<Point> = points.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&pts, closed));
    d.insert(Some(l), 99, Node::path(id, path, Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0))).unwrap();
    id
}

const FRAME: [(f64, f64); 4] = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];

#[test]
fn an_unfilled_closed_path_is_hit_inside_unless_by_path_only_or_in_outline() {
    let mut d = Document::new(200.0, 200.0);
    let frame = stroked(&mut d, &FRAME, true);
    let at = |opt: HitOptions| hit_test(&d, Point::new(50.0, 50.0), opt).map(|h| h.leaf);
    assert_eq!(at(INTERIORS), Some(frame));
    assert_eq!(at(SELECTING), None, "without the option, the interior misses");
    assert_eq!(at(HitOptions { path_only: true, ..INTERIORS }), None, "Object Selection by Path Only");
    assert_eq!(at(HitOptions { outline: true, ..INTERIORS }), None, "Outline view");
    assert_eq!(objects_at(&d, Point::new(50.0, 50.0), INTERIORS, None), vec![frame]);
    assert!(objects_at(&d, Point::new(150.0, 50.0), INTERIORS, None).is_empty(), "outside it");
}

#[test]
fn an_open_path_is_not_hit_inside() {
    let mut d = Document::new(200.0, 200.0);
    stroked(&mut d, &FRAME, false);
    assert_eq!(hit_test(&d, Point::new(50.0, 50.0), INTERIORS), None);
}

/// What a click paints over comes first: an object drawn inside an unfilled frame above it is hit
/// where it paints, and the frame elsewhere inside; Select Behind lists the frame after it.
#[test]
fn art_inside_an_unfilled_frame_above_it_is_hit_before_the_frame() {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let dot = d.alloc_id();
    d.insert(Some(l), 0, Node::path(dot, shapes::rectangle(Rect::new(40.0, 40.0, 60.0, 60.0)), Appearance::default_art())).unwrap();
    let frame = stroked(&mut d, &FRAME, true);
    let at = |d: &Document, x: f64| hit_test(d, Point::new(x, 50.0), INTERIORS).map(|h| h.leaf);
    assert_eq!(at(&d, 50.0), Some(dot));
    assert_eq!(at(&d, 20.0), Some(frame));
    assert_eq!(objects_at(&d, Point::new(50.0, 50.0), INTERIORS, None), vec![dot, frame]);
    // The topmost of two unfilled frames is hit inside both.
    let inner = stroked(&mut d, &[(10.0, 10.0), (90.0, 10.0), (90.0, 90.0), (10.0, 90.0)], true);
    assert_eq!(at(&d, 20.0), Some(inner));
    assert_eq!(at(&d, 5.0), Some(frame));
}

#[test]
fn an_unfilled_compound_path_is_hit_inside_its_members_but_not_its_holes() {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let look = || Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
    let ring = |id, r: Rect| Arc::new(Node::path(id, shapes::rectangle(r), look()));
    let (id, outer, hole) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    let members = vec![ring(outer, Rect::new(0.0, 0.0, 100.0, 100.0)), ring(hole, Rect::new(30.0, 30.0, 70.0, 70.0))];
    let mut compound = Node::new(id, NodeKind::Compound { children: members, rule: FillRule::EvenOdd });
    compound.appearance = look().into();
    d.insert(Some(l), 0, compound).unwrap();
    let at = |x: f64| hit_test(&d, Point::new(x, 50.0), INTERIORS).map(|h| h.leaf);
    assert_eq!(at(15.0), Some(id));
    assert_eq!(at(50.0), None, "in the hole");
}

#[test]
fn guides_and_clipping_paths_are_not_hit_inside() {
    let mut d = Document::new(200.0, 200.0);
    let guide = stroked(&mut d, &FRAME, true);
    if let Some(NodeKind::Path { guide: g, .. }) = d.node_mut(guide).map(|n| &mut n.kind) {
        *g = true;
    }
    assert_eq!(hit_test(&d, Point::new(50.0, 50.0), INTERIORS), None, "a guide");
    // A clip group whose art covers only part of its unfilled clipping path.
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let (g, clip, art) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    let mut mask = Node::path(clip, shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(Paint::None, Paint::None, 0.0));
    if let NodeKind::Path { clipping, .. } = &mut mask.kind {
        *clipping = true;
    }
    let picture = Node::path(art, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 100.0)), Appearance::default_art());
    d.insert(Some(l), 0, Node::new(g, NodeKind::Group { children: vec![Arc::new(mask), Arc::new(picture)], clip: true })).unwrap();
    let at = |x: f64| hit_test(&d, Point::new(x, 50.0), INTERIORS).map(|h| h.leaf);
    assert_eq!(at(25.0), Some(art));
    assert_eq!(at(75.0), None, "inside the mask where no art paints");
    assert!(objects_at(&d, Point::new(75.0, 50.0), INTERIORS, None).is_empty());
}

/// A repeat's copies of an unfilled frame are hit inside, and the copies of a clip group's unfilled
/// compound clipping path are not.
#[test]
fn a_repeat_is_hit_inside_its_unfilled_copies_but_not_inside_their_masks() {
    let look = || Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
    let kind = crate::pattern::RepeatKind::Grid {
        h_spacing: 20.0,
        v_spacing: 20.0,
        rows: 1,
        cols: 2,
        grid_type: crate::pattern::TileType::Grid,
        flip_rows: false,
        flip_cols: false,
    };
    let repeat = |d: &mut Document, source: Node| {
        let r = d.alloc_id();
        let l = d.layers[0].id;
        d.insert(Some(l), 0, Node::new(r, NodeKind::Repeat(crate::pattern::RepeatSpec { kind: kind.clone(), source: vec![Arc::new(source)] })))
            .unwrap();
        r
    };
    // Copies at 0..40 and 60..100.
    let mut d = Document::new(300.0, 200.0);
    let frame = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 40.0)), look());
    let r = repeat(&mut d, frame);
    assert_eq!(hit_test(&d, Point::new(80.0, 20.0), INTERIORS).map(|h| h.leaf), Some(r));
    assert_eq!(hit_test(&d, Point::new(50.0, 20.0), INTERIORS), None, "between the copies");
    let mut d = Document::new(300.0, 200.0);
    let ring = |id, r: Rect| Arc::new(Node::path(id, shapes::rectangle(r), look()));
    let members = vec![ring(d.alloc_id(), Rect::new(0.0, 0.0, 40.0, 40.0)), ring(d.alloc_id(), Rect::new(5.0, 5.0, 35.0, 35.0))];
    let mut mask = Node::new(d.alloc_id(), NodeKind::Compound { children: members, rule: FillRule::NonZero });
    mask.appearance = look().into();
    let picture = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 40.0)), Appearance::default_art());
    let mut clip = Node::group(d.alloc_id(), vec![Arc::new(mask), Arc::new(picture)]);
    clip.set_clips(true);
    let r = repeat(&mut d, clip);
    assert_eq!(hit_test(&d, Point::new(65.0, 20.0), INTERIORS).map(|h| h.leaf), Some(r), "on the clipped art");
    assert_eq!(hit_test(&d, Point::new(80.0, 20.0), INTERIORS), None, "inside the mask where no art paints");
}

/// Direct Selection in a dynamic symbol's art picks parts as a click on the document does: what
/// paints first, then an unfilled frame inside; an unfilled compound clipping path by its outline.
#[test]
fn symbol_art_parts_are_picked_inside_unfilled_frames_after_what_paints() {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let look = || Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
    let dot = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(40.0, 40.0, 60.0, 60.0)), Appearance::default_art());
    let frame = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), look());
    let ring = |id, r: Rect| Arc::new(Node::path(id, shapes::rectangle(r), look()));
    let (mask_id, a, b) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    let members = vec![ring(a, Rect::new(150.0, 0.0, 250.0, 100.0)), ring(b, Rect::new(160.0, 10.0, 240.0, 90.0))];
    let mut mask = Node::new(mask_id, NodeKind::Compound { children: members, rule: FillRule::NonZero });
    mask.appearance = look().into();
    let picture = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(150.0, 0.0, 180.0, 100.0)), Appearance::default_art());
    let mut clip = Node::group(d.alloc_id(), vec![Arc::new(mask), Arc::new(picture)]);
    clip.set_clips(true);
    let root = Node::group(d.alloc_id(), vec![Arc::new(dot), Arc::new(frame), Arc::new(clip)]);
    d.symbols.push(crate::Symbol { dynamic: true, name: "Framed".into(), art: Arc::new(root) });
    let inst = d.alloc_id();
    d.insert(Some(l), 0, Node::new(inst, NodeKind::SymbolInstance { symbol: "Framed".into(), xf: vectorcraft_geom::Affine::IDENTITY })).unwrap();
    let n = d.node(inst).unwrap();
    let at = |x: f64| symbol_part_at(&d, n, Point::new(x, 50.0), INTERIORS);
    assert_eq!(at(50.0), Some(vec![0]), "the dot under the frame");
    assert_eq!(at(20.0), Some(vec![1]), "inside the frame");
    assert_eq!(at(220.0), None, "inside the clipping path where no art paints");
}

#[test]
fn long_plain_stroke_hits_its_middle_and_curved_end() {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let mut sp = vectorcraft_geom::SubPath::polyline(&(0..=4097).map(|i| Point::new(i as f64 * 10.0, 0.0)).collect::<Vec<_>>(), false);
    sp.anchors[4096].h_out = Point::new(40960.0, 100.0);
    sp.anchors[4097].h_in = Point::new(40970.0, 100.0);
    sp.anchors.push(vectorcraft_geom::Anchor::corner(Point::new(40970.0, 100.0)));
    sp.anchors.push(vectorcraft_geom::Anchor::corner(Point::new(0.0, 100.0)));
    let path = vectorcraft_geom::PathData::single(sp);
    let look = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
    d.insert(Some(l), 0, Node::path(id, path, look)).unwrap();
    let outline = HitOptions { outline: true, ..Default::default() };
    for point in [Point::new(20485.0, 0.0), Point::new(40965.0, 75.0)] {
        assert_eq!(hit_test(&d, point, HitOptions::default()).map(|h| h.kind), Some(HitKind::Stroke));
        assert_eq!(hit_test(&d, point, outline).map(|h| h.kind), Some(HitKind::Outline));
    }
    assert!(hit_test(&d, Point::new(20485.0, 50.0), HitOptions::default()).is_none());
}

#[test]
fn coincident_anchors_remain_hittable_as_an_outline() {
    let mut d = Document::new(200.0, 200.0);
    let id = d.alloc_id();
    let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&[Point::new(50.0, 50.0); 2], false));
    let look = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
    d.insert(d.layers.first().map(|l| l.id), 0, Node::path(id, path, look)).unwrap();
    let at = Point::new(50.0, 50.0);
    assert_eq!(hit_test(&d, at, HitOptions::default()).map(|h| h.kind), Some(HitKind::Outline));
    assert_eq!(hit_test(&d, at, HitOptions { outline: true, ..Default::default() }).map(|h| h.kind), Some(HitKind::Outline));
}

/// #1103: a marquee drawn over a background picks what is on it, not the background, whose
/// outline it doesn't touch.
#[test]
fn a_marquee_inside_a_background_leaves_the_background() {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let (back, small) = (d.alloc_id(), d.alloc_id());
    d.insert(Some(l), 0, Node::path(back, shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 200.0)), Appearance::default_art())).unwrap();
    d.insert(Some(l), 1, Node::path(small, shapes::rectangle(Rect::new(50.0, 50.0, 60.0, 60.0)), Appearance::default_art())).unwrap();
    assert_eq!(marquee(&d, Rect::new(40.0, 40.0, 70.0, 70.0), None, false), vec![small]);
    // Reaching the background's edge takes it too.
    assert_eq!(marquee(&d, Rect::new(-5.0, 40.0, 70.0, 70.0), None, false), vec![back, small]);
}

#[test]
fn group_top_object() {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let a = d.alloc_id();
    let g = d.alloc_id();
    let p = Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art());
    d.insert(Some(l), 0, Node::group(g, vec![Arc::new(p)])).unwrap();
    let h = hit_test(&d, Point::new(10.0, 10.0), HitOptions::default()).unwrap();
    assert_eq!(h.leaf, a);
    assert_eq!(h.top_object(None), g);
    assert_eq!(h.top_object(Some(g)), a);
    assert_eq!(marquee(&d, Rect::new(-5.0, -5.0, 5.0, 5.0), None, false), vec![g]);
    assert_eq!(marquee(&d, Rect::new(-5.0, -5.0, 5.0, 5.0), None, true), vec![a]);
    assert!(marquee(&d, Rect::new(100.0, 100.0, 105.0, 105.0), None, false).is_empty());
}

/// The art of a 200 pt wide rectangle clipped to its left half (0..100 × 0..100): the mask and
/// the art, in that order.
fn half_clipped(d: &mut Document) -> (Node, Node) {
    let mask = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default());
    let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 100.0)), Appearance::default_art());
    (mask, art)
}

/// A stand-in for the engine's [`ClipRegion`]: with an effect named `test.move` in the clipping
/// path, its first shape moved 100 pt right; with one named `test.empty`, no region left to clip
/// by; without either, no geometry effect.
fn moved_clip_region(clip: &Node) -> Option<Option<(BezPath, FillRule)>> {
    let (mut moves, mut empty) = (false, false);
    clip.walk(&mut |n| {
        moves |= n.appearance.effects.iter().any(|e| e.visible && e.id == "test.move");
        empty |= n.appearance.effects.iter().any(|e| e.visible && e.id == "test.empty");
    });
    if empty {
        return Some(None);
    }
    if !moves {
        return None;
    }
    Some(clip_regions(clip).into_iter().next().map(|(mut bp, rule)| {
        bp.apply_affine(vectorcraft_geom::Affine::translate((100.0, 0.0)));
        (bp, rule)
    }))
}

/// A visible effect named `test.move`, which [`moved_clip_region`] reads as moving the region.
fn test_move() -> crate::Effect {
    crate::Effect { id: "test.move".into(), params: serde_json::json!({}), visible: true }
}

/// Clicks inside a clip group whose clipping path has a live effect fall in the region the
/// registered [`ClipRegion`] gives it. The pointer moving over it evaluates the region once, and
/// again after the clipping path changes; a clipping path without effects is not evaluated.
#[test]
fn clicks_in_a_clip_group_evaluate_its_clipping_paths_region_once_until_it_changes() {
    set_clip_region(moved_clip_region);
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let (mut mask, art) = half_clipped(&mut d);
    mask.appearance.effects.push(crate::Effect { id: "test.move".into(), params: serde_json::json!({}), visible: true });
    let (g, mask_id, art_id) = (d.alloc_id(), mask.id, art.id);
    let mut clip = Node::group(g, vec![Arc::new(mask), Arc::new(art)]);
    clip.set_clips(true);
    d.insert(Some(l), 0, clip).unwrap();
    let evaluated = || CLIPS_EVALUATED.with(std::cell::Cell::get);
    let before = evaluated();
    let leaf = |d: &Document, x: f64| hit_test(d, Point::new(x, 50.0), HitOptions::default()).map(|h| h.leaf);
    for x in [148.0, 150.0, 152.0] {
        assert_eq!(leaf(&d, x), Some(art_id), "in the moved region");
    }
    assert_eq!(leaf(&d, 50.0), None, "the clipping path's own place");
    assert_eq!(evaluated(), before + 1);
    // Outline mode draws no effects: the art's edge shows within the clipping path's own shape.
    let outline = HitOptions { outline: true, ..HitOptions::default() };
    assert_eq!(hit_test(&d, Point::new(50.0, 0.5), outline).map(|h| h.leaf), Some(art_id), "outline mode");
    // So with a marquee across the art's left edge: in the view, only in Outline view. The
    // clipping path's own left edge lies there too, and counts in both.
    let edge = Rect::new(-5.0, 40.0, 5.0, 60.0);
    assert_eq!(marquee_in_view(&d, edge, None, true, false, false), [mask_id]);
    assert_eq!(marquee_in_view(&d, edge, None, true, true, false), [mask_id, art_id]);
    // `marquee_in` measures by the plain shapes, as its callers clip by them.
    assert!(marquee_in(d.node(g).unwrap(), edge).is_some());
    // Hidden, the effect no longer moves the region.
    d.node_mut(mask_id).unwrap().appearance.effects[0].visible = false;
    assert_eq!(leaf(&d, 50.0), Some(art_id));
    assert_eq!(leaf(&d, 150.0), None);
    assert_eq!(evaluated(), before + 1);
    // Shown again and widened, the region is evaluated again.
    let mask = d.node_mut(mask_id).unwrap();
    mask.appearance.effects[0].visible = true;
    if let NodeKind::Path { path, .. } = &mut mask.kind {
        *path = shapes::rectangle(Rect::new(0.0, 0.0, 90.0, 100.0));
    }
    assert_eq!(leaf(&d, 180.0), Some(art_id));
    assert_eq!(leaf(&d, 195.0), None);
    assert_eq!(evaluated(), before + 2);
}

/// A marquee around a clipping path that an effect moved into the middle of large art touches the
/// clipping path on the edge of its moved region, in a clip group and in a clipping layer.
#[test]
fn a_marquee_touches_a_moved_clipping_path_on_its_region() {
    set_clip_region(moved_clip_region);
    let parts = |d: &mut Document| {
        let mut mask = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(20.0, 20.0, 60.0, 60.0)), Appearance::default());
        mask.appearance.effects.push(crate::Effect { id: "test.move".into(), params: serde_json::json!({}), visible: true });
        let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 300.0, 300.0)), Appearance::default_art());
        (mask, art)
    };
    // Around the moved region (120..160) and the clipping path's own place, inside the art.
    let around = Rect::new(10.0, 10.0, 170.0, 70.0);
    let mut d = Document::new(400.0, 400.0);
    let l = d.layers[0].id;
    let (mask, art) = parts(&mut d);
    let (g, mask_id) = (d.alloc_id(), mask.id);
    let mut clip = Node::group(g, vec![Arc::new(mask), Arc::new(art)]);
    clip.set_clips(true);
    d.insert(Some(l), 0, clip).unwrap();
    assert_eq!(marquee(&d, around, None, false), [g]);
    assert_eq!(marquee(&d, around, None, true), [mask_id]);
    // Only over the clipping path's own place, the marquee misses the region but touches the
    // clipping path's anchors, which a click on its outline also finds.
    let own = Rect::new(10.0, 10.0, 70.0, 70.0);
    assert_eq!(marquee(&d, own, None, false), [g]);
    assert_eq!(marquee(&d, own, None, true), [mask_id]);
    // Away from both, nothing.
    assert!(marquee(&d, Rect::new(70.0, 10.0, 110.0, 70.0), None, true).is_empty());

    let mut d = Document::new(400.0, 400.0);
    let top = d.layers[0].id;
    let (mask, art) = parts(&mut d);
    let mask_id = mask.id;
    let mut sublayer = (*d.layers[0]).clone();
    sublayer.id = d.alloc_id();
    if let NodeKind::Layer { children, clip, .. } = &mut sublayer.kind {
        (*children, *clip) = (vec![Arc::new(mask), Arc::new(art)], true);
    }
    d.insert(Some(top), 0, sublayer).unwrap();
    assert_eq!(marquee(&d, around, None, false), [mask_id]);
}

/// A clipping layer whose Preview is off draws its art in outline: clicks, the objects at a point
/// and marquees count its clipping path by its plain shape, without the effect that moves it.
#[test]
fn a_clipping_layer_with_preview_off_counts_its_plain_clipping_path() {
    set_clip_region(moved_clip_region);
    let mut d = Document::new(300.0, 200.0);
    let top = d.layers[0].id;
    let (mut mask, art) = half_clipped(&mut d);
    mask.appearance.effects.push(crate::Effect { id: "test.move".into(), params: serde_json::json!({}), visible: true });
    let art_id = art.id;
    let mut sublayer = (*d.layers[0]).clone();
    sublayer.id = d.alloc_id();
    if let NodeKind::Layer { children, clip, preview, .. } = &mut sublayer.kind {
        (*children, *clip, *preview) = (vec![Arc::new(mask), Arc::new(art)], true, false);
    }
    d.insert(Some(top), 0, sublayer).unwrap();
    // The art's top edge, within the clipping path's own shape.
    let p = Point::new(50.0, 0.5);
    assert_eq!(hit_test(&d, p, HitOptions::default()).map(|h| h.leaf), Some(art_id));
    assert_eq!(objects_at(&d, p, HitOptions::default(), None).first(), Some(&art_id));
    assert!(marquee(&d, Rect::new(45.0, -5.0, 55.0, 5.0), None, true).contains(&art_id));
}

/// The pointer moving over empty canvas past more clip groups with live effects than the regions
/// kept measures each group's box once: the boxes are kept for every clip group.
#[test]
fn hovers_past_many_clip_groups_evaluate_each_clipping_path_once() {
    set_clip_region(moved_clip_region);
    let mut d = Document::new(1000.0, 1000.0);
    let l = d.layers[0].id;
    for i in 0..(2 * CLIPS_KEPT) {
        let x = 20.0 * i as f64;
        let mut mask = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)), Appearance::default());
        mask.appearance.effects.push(crate::Effect { id: "test.move".into(), params: serde_json::json!({}), visible: true });
        let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 0.0, x + 20.0, 10.0)), Appearance::default_art());
        let mut clip = Node::group(d.alloc_id(), vec![Arc::new(mask), Arc::new(art)]);
        clip.set_clips(true);
        d.insert(Some(l), 0, clip).unwrap();
    }
    let evaluated = || CLIPS_EVALUATED.with(std::cell::Cell::get);
    let empty = Point::new(500.0, 500.0);
    assert!(hit_test(&d, empty, HitOptions::default()).is_none());
    let first = evaluated();
    for _ in 0..3 {
        assert!(hit_test(&d, empty, HitOptions::default()).is_none());
    }
    assert_eq!(evaluated(), first);
}

/// With more clip groups with live effects than the boxes kept, a new box replaces only as many
/// others as it needs room for: the next hover finds most of the boxes and evaluates few regions.
#[test]
fn hovers_past_more_clip_groups_than_the_boxes_kept_evaluate_few_again() {
    set_clip_region(moved_clip_region);
    let mut d = Document::new(1000.0, 1000.0);
    let l = d.layers[0].id;
    let groups = CLIP_BOXES_KEPT + 64;
    let mut members = Vec::with_capacity(groups);
    for i in 0..groups {
        let x = (i % 100) as f64 * 5.0;
        let y = (i / 100) as f64 * 5.0;
        let mut mask = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, y, x + 2.0, y + 2.0)), Appearance::default());
        mask.appearance.effects.push(test_move());
        let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, y, x + 4.0, y + 2.0)), Appearance::default_art());
        let mut clip = Node::group(d.alloc_id(), vec![Arc::new(mask), Arc::new(art)]);
        clip.set_clips(true);
        members.push(Arc::new(clip));
    }
    if let Some(children) = d.node_mut(l).and_then(|n| n.children_mut()) {
        *children = members;
    }
    let evaluated = || CLIPS_EVALUATED.with(std::cell::Cell::get);
    let empty = Point::new(900.0, 900.0);
    assert!(hit_test(&d, empty, HitOptions::default()).is_none());
    let first = evaluated();
    assert!(hit_test(&d, empty, HitOptions::default()).is_none());
    let again = evaluated() - first;
    assert!(again < groups / 8, "{again} of {groups} regions evaluated again");
}

/// A stand-in for the engine's [`DrawnArt`]: an object with a visible effect named `test.shift`
/// draws 100 pt to the right of its own place, and one with a visible effect named `test.copy`
/// draws at its own place and 100 pt to the right. The effects are applied innermost first, and
/// the art keeps the ids of the objects it is drawn from. An instance of a symbol whose art has a
/// visible effect draws as a 50 pt square from (100, 0), white with a black 1 pt stroke. A clip
/// group with a visible effect named `test.mask_only` draws as a group of its clipping path alone,
/// as the engine's art for a clip group whose clipping path's strokes have geometry effects is.
/// Without any of these, no geometry effect changes what an object draws.
fn shifted_art(s: &SymbolBoxes<'_>, n: &Node) -> Option<Node> {
    if n.clips()
        && n.appearance.effects.iter().any(|e| e.visible && e.id == "test.mask_only")
        && let Some(mask) = n.children().and_then(|c| c.first())
    {
        return Some(Node::group(n.id, vec![mask.clone()]));
    }
    fn draw(s: &SymbolBoxes<'_>, n: &Node, found: &mut bool) -> Node {
        if let NodeKind::SymbolInstance { symbol, .. } = &n.kind
            && s.has_effects(symbol)
        {
            *found = true;
            return Node::path(n.id, shapes::rectangle(Rect::new(100.0, 0.0, 150.0, 50.0)), Appearance::default_art());
        }
        let mut m = n.clone();
        if let Some(children) = m.children_mut() {
            for c in children.iter_mut() {
                *c = Arc::new(draw(s, c, found));
            }
        }
        let has = |id: &str| n.appearance.effects.iter().any(|e| e.visible && e.id == id);
        let (shift, copy) = (has("test.shift"), has("test.copy"));
        m.appearance.effects.retain(|e| e.id != "test.shift" && e.id != "test.copy");
        *found |= shift || copy;
        let shifted = |m: &Node| moved(m, vectorcraft_geom::Affine::translate((100.0, 0.0)));
        if shift {
            shifted(&m)
        } else if copy {
            Node::group(n.id, vec![Arc::new(shifted(&m)), Arc::new(m)])
        } else {
            m
        }
    }
    let mut found = false;
    let art = draw(s, n, &mut found);
    found.then_some(art)
}

/// A visible effect named `id`.
fn effect(id: &str) -> crate::Effect {
    crate::Effect { id: id.into(), params: serde_json::json!({}), visible: true }
}

/// A 50 pt square at `x`, `y`, white with a black 1 pt stroke.
fn square(d: &mut Document, x: f64, y: f64) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, y, x + 50.0, y + 50.0)), Appearance::default_art())
}

/// The leaf and the kind a click at (`x`, `y`) hits.
fn hit_at(d: &Document, x: f64, y: f64, opt: HitOptions) -> Option<(NodeId, HitKind)> {
    hit_test(d, Point::new(x, y), opt).map(|h| (h.leaf, h.kind))
}

/// A path that a geometry effect moves is clicked where it is drawn, and not at its own place,
/// which it no longer paints; with Object Selection by Path Only on its drawn outline, not its
/// fill. Outline view draws no effects: there its own outline is clicked. Hidden, the effect no
/// longer moves what clicks hit.
#[test]
fn a_click_hits_a_paths_shifted_art_not_its_source() {
    set_drawn_art(shifted_art);
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let mut p = square(&mut d, 0.0, 0.0);
    p.appearance.effects.push(effect("test.shift"));
    let a = p.id;
    d.insert(Some(l), 0, p).unwrap();
    let opt = HitOptions::default();
    assert_eq!(hit_at(&d, 125.0, 25.0, opt), Some((a, HitKind::Fill)), "where it is drawn");
    assert_eq!(hit_at(&d, 25.0, 25.0, opt), None, "its own place");
    assert_eq!(objects_at(&d, Point::new(125.0, 25.0), opt, None), [a]);
    assert!(objects_at(&d, Point::new(25.0, 25.0), opt, None).is_empty());
    let path_only = HitOptions { path_only: true, ..opt };
    assert_eq!(hit_at(&d, 125.0, 25.0, path_only), None, "path only: the drawn fill");
    assert_eq!(hit_at(&d, 100.0, 25.0, path_only), Some((a, HitKind::Stroke)), "path only: the drawn outline");
    let outline = HitOptions { outline: true, ..opt };
    assert_eq!(hit_at(&d, 0.5, 25.0, outline), Some((a, HitKind::Outline)), "outline: its own outline");
    assert_eq!(hit_at(&d, 100.5, 25.0, outline), None, "outline: no drawn art");
    d.node_mut(a).unwrap().appearance.effects[0].visible = false;
    assert_eq!(hit_at(&d, 25.0, 25.0, opt), Some((a, HitKind::Fill)), "hidden effect: its own place");
    assert_eq!(hit_at(&d, 125.0, 25.0, opt), None, "hidden effect: nothing drawn there");
}

/// A geometry effect that copies a path is clicked on the copy and on the path, the topmost of the
/// stack first. The pointer moving over it evaluates its art once until it changes, and a path
/// without effects is not evaluated.
#[test]
fn a_click_hits_a_paths_copy_and_evaluates_its_art_once_until_it_changes() {
    set_drawn_art(shifted_art);
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let below = square(&mut d, 100.0, 0.0);
    let mut p = square(&mut d, 0.0, 0.0);
    p.appearance.effects.push(effect("test.copy"));
    let (a, b) = (p.id, below.id);
    d.insert(Some(l), 0, below).unwrap();
    d.insert(Some(l), 1, p).unwrap();
    let evaluated = || DRAWN_EVALUATED.with(std::cell::Cell::get);
    let before = evaluated();
    let opt = HitOptions::default();
    for _ in 0..3 {
        assert_eq!(hit_at(&d, 25.0, 25.0, opt).map(|h| h.0), Some(a), "the path");
        assert_eq!(hit_at(&d, 125.0, 25.0, opt).map(|h| h.0), Some(a), "its copy, over the square below");
    }
    assert_eq!(objects_at(&d, Point::new(125.0, 25.0), opt, None), [a, b]);
    assert_eq!(evaluated(), before + 1, "evaluated once");
    if let Some(NodeKind::Path { path, .. }) = d.node_mut(a).map(|n| &mut n.kind) {
        *path = shapes::rectangle(Rect::new(0.0, 0.0, 30.0, 50.0));
    }
    assert_eq!(hit_at(&d, 140.0, 25.0, opt).map(|h| h.0), Some(b), "narrowed, the copy leaves the square below");
    assert_eq!(evaluated(), before + 2, "evaluated again after it changed");
    // The square below has no effects: it is never evaluated.
    assert_eq!(hit_at(&d, 145.0, 45.0, opt).map(|h| h.0), Some(b));
    assert_eq!(evaluated(), before + 2);
}

/// A group without effects or fills and strokes of its own is not evaluated as a whole: a click
/// on the art of a moved member in it evaluates that member only, and finds the group by it.
#[test]
fn a_click_in_a_plain_group_evaluates_only_its_member_with_effects() {
    set_drawn_art(shifted_art);
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let mut moved = square(&mut d, 0.0, 0.0);
    moved.appearance.effects.push(effect("test.shift"));
    let still = square(&mut d, 0.0, 100.0);
    let (a, g) = (moved.id, d.alloc_id());
    d.insert(Some(l), 0, Node::group(g, vec![Arc::new(moved), Arc::new(still)])).unwrap();
    let evaluated = || DRAWN_EVALUATED.with(std::cell::Cell::get);
    let before = evaluated();
    let opt = HitOptions::default();
    for _ in 0..3 {
        assert_eq!(hit_at(&d, 125.0, 25.0, opt), Some((a, HitKind::Fill)));
        assert_eq!(objects_at(&d, Point::new(125.0, 25.0), opt, None), [g]);
    }
    assert_eq!(evaluated(), before + 1, "only the member with effects, once");
}

/// A compound path that a geometry effect moves keeps its hole where it is drawn, for clicks and
/// marquees.
#[test]
fn a_compound_paths_shifted_art_keeps_its_hole() {
    set_drawn_art(shifted_art);
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let outer = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 60.0, 60.0)), Appearance::default());
    let hole = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(20.0, 20.0, 40.0, 40.0)), Appearance::default());
    let c = d.alloc_id();
    let mut compound = Node::new(c, NodeKind::Compound { children: vec![Arc::new(outer), Arc::new(hole)], rule: FillRule::EvenOdd });
    compound.appearance = Appearance::default_art().into();
    compound.appearance.effects.push(effect("test.shift"));
    d.insert(Some(l), 0, compound).unwrap();
    let opt = HitOptions::default();
    assert_eq!(hit_at(&d, 105.0, 5.0, opt), Some((c, HitKind::Fill)), "its drawn fill");
    assert_eq!(hit_at(&d, 130.0, 30.0, opt), None, "its drawn hole");
    assert_eq!(hit_at(&d, 5.0, 5.0, opt), None, "its own place");
    assert_eq!(marquee(&d, Rect::new(155.0, 25.0, 165.0, 35.0), None, false), [c], "a marquee across the drawn edge");
    assert!(marquee(&d, Rect::new(-5.0, 25.0, 5.0, 35.0), None, false).is_empty(), "a marquee across its own edge");
}

/// A marquee touches a path that a geometry effect moves where it is drawn, and not at its own
/// place; in Outline view, at its own place.
#[test]
fn a_marquee_touches_a_paths_shifted_art_not_its_source() {
    set_drawn_art(shifted_art);
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let mut p = square(&mut d, 0.0, 0.0);
    p.appearance.effects.push(effect("test.shift"));
    let a = p.id;
    d.insert(Some(l), 0, p).unwrap();
    let (drawn, own) = (Rect::new(140.0, 40.0, 160.0, 60.0), Rect::new(40.0, 40.0, 60.0, 60.0));
    for leaves in [false, true] {
        assert_eq!(marquee(&d, drawn, None, leaves), [a], "where it is drawn");
        assert!(marquee(&d, own, None, leaves).is_empty(), "its own place");
        assert!(marquee_in_view(&d, drawn, None, leaves, true, false).is_empty(), "outline: no drawn art");
        assert_eq!(marquee_in_view(&d, own, None, leaves, true, false), [a], "outline: its own place");
    }
}

/// A group whose own geometry effect moves its members is clicked where each member is drawn: the
/// member, inside the group, is hit, and a locked member lets the click through to what is below
/// and is listed for unlocking there. A marquee there touches the group, or the member for Direct
/// Selection.
#[test]
fn a_groups_own_effect_moves_where_its_members_are_hit() {
    set_drawn_art(shifted_art);
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let below = square(&mut d, 150.0, 0.0);
    let (a, b) = (square(&mut d, 0.0, 0.0), square(&mut d, 50.0, 0.0));
    let (below_id, a_id, b_id, g) = (below.id, a.id, b.id, d.alloc_id());
    let mut group = Node::group(g, vec![Arc::new(a), Arc::new(b)]);
    group.appearance.effects.push(effect("test.shift"));
    d.insert(Some(l), 0, below).unwrap();
    d.insert(Some(l), 1, group).unwrap();
    let opt = HitOptions::default();
    let h = hit_test(&d, Point::new(125.0, 25.0), opt).unwrap();
    assert_eq!((h.leaf, h.top_object(None)), (a_id, g), "the first member, drawn there");
    assert_eq!(h.ancestry, [l, g, a_id]);
    assert_eq!(hit_at(&d, 175.0, 25.0, opt).map(|h| h.0), Some(b_id), "the second member, over the square below");
    assert_eq!(hit_at(&d, 25.0, 25.0, opt), None, "the first member's own place");
    assert_eq!(objects_at(&d, Point::new(175.0, 25.0), opt, None), [g, below_id]);
    let drawn = Rect::new(195.0, 20.0, 205.0, 30.0);
    assert_eq!(marquee(&d, drawn, None, false), [below_id, g]);
    assert_eq!(marquee(&d, drawn, None, true), [below_id, b_id]);
    assert!(marquee(&d, Rect::new(-5.0, 20.0, 5.0, 30.0), None, true).is_empty(), "the members' own place");
    d.node_mut(b_id).unwrap().locked = true;
    assert_eq!(hit_at(&d, 175.0, 25.0, opt).map(|h| h.0), Some(below_id), "a locked member lets the click through");
    assert_eq!(marquee(&d, drawn, None, true), [below_id], "nor does a marquee collect it");
    assert_eq!(locked_objects_at(&d, Point::new(175.0, 25.0), opt), [b_id], "the Unlock submenu lists it where it is drawn");
    assert!(locked_objects_at(&d, Point::new(75.0, 25.0), opt).is_empty(), "and not at its own place");
    // Outline view draws no effects: the members are clicked at their own places.
    let outline = HitOptions { outline: true, ..opt };
    assert_eq!(hit_at(&d, 0.5, 25.0, outline).map(|h| h.0), Some(a_id));
}

/// A layer whose own geometry effect moves its art is clicked and marqueed where the art is
/// drawn. With the layer's Preview off, its art is clicked at its own place, in outline.
#[test]
fn a_layers_own_effect_moves_where_its_art_is_hit() {
    set_drawn_art(shifted_art);
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let a = square(&mut d, 0.0, 0.0);
    let a_id = a.id;
    d.insert(Some(l), 0, a).unwrap();
    d.node_mut(l).unwrap().appearance.effects.push(effect("test.shift"));
    let opt = HitOptions::default();
    assert_eq!(hit_at(&d, 125.0, 25.0, opt), Some((a_id, HitKind::Fill)), "where it is drawn");
    assert_eq!(hit_at(&d, 25.0, 25.0, opt), None, "its own place");
    assert_eq!(objects_at(&d, Point::new(125.0, 25.0), opt, None), [a_id]);
    assert!(objects_at(&d, Point::new(25.0, 25.0), opt, None).is_empty());
    let (drawn, own) = (Rect::new(145.0, 20.0, 155.0, 30.0), Rect::new(-5.0, 20.0, 5.0, 30.0));
    for leaves in [false, true] {
        assert_eq!(marquee(&d, drawn, None, leaves), [a_id], "a marquee where it is drawn");
        assert!(marquee(&d, own, None, leaves).is_empty(), "a marquee at its own place");
    }
    if let Some(NodeKind::Layer { preview, .. }) = d.node_mut(l).map(|n| &mut n.kind) {
        *preview = false;
    }
    assert_eq!(hit_at(&d, 0.5, 25.0, opt), Some((a_id, HitKind::Outline)), "preview off: its own outline");
    assert_eq!(hit_at(&d, 125.0, 25.0, opt), None, "preview off: no drawn art");
    assert_eq!(marquee(&d, own, None, false), [a_id], "preview off: a marquee at its own place");
}

/// In a layer whose own geometry effect moves its art, a sublayer with Preview off is clicked in
/// outline: on the outline of its art where the art is drawn, not inside its fill.
#[test]
fn a_preview_off_sublayer_of_an_effected_layer_is_clicked_in_outline() {
    set_drawn_art(shifted_art);
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let sub = d.alloc_id();
    d.insert(Some(l), 0, Node::layer(sub, "Sub", crate::LayerColor::Preset(2))).unwrap();
    let a = square(&mut d, 0.0, 0.0);
    let a_id = a.id;
    d.insert(Some(sub), 0, a).unwrap();
    d.node_mut(l).unwrap().appearance.effects.push(effect("test.shift"));
    let opt = HitOptions::default();
    assert_eq!(hit_at(&d, 125.0, 25.0, opt), Some((a_id, HitKind::Fill)), "preview on: inside its fill");
    if let Some(NodeKind::Layer { preview, .. }) = d.node_mut(sub).map(|n| &mut n.kind) {
        *preview = false;
    }
    assert_eq!(hit_at(&d, 125.0, 25.0, opt), None, "preview off: inside its fill");
    assert_eq!(hit_at(&d, 100.5, 25.0, opt), Some((a_id, HitKind::Outline)), "preview off: on its outline");
}

/// A group with fills and strokes of its own around an instance of a symbol whose art has an
/// effect is clicked where its art is drawn, though no object in the group has an effect.
#[test]
fn a_painted_group_around_an_effected_symbols_instance_is_hit_where_its_art_is_drawn() {
    set_drawn_art(shifted_art);
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let mut art = square(&mut d, 0.0, 0.0);
    art.appearance.effects.push(effect("test.raster"));
    d.symbols.push(crate::Symbol { dynamic: false, name: "Effected".into(), art: Arc::new(art) });
    let (g, inst) = (d.alloc_id(), d.alloc_id());
    let instance = Node::new(inst, NodeKind::SymbolInstance { symbol: "Effected".into(), xf: vectorcraft_geom::Affine::IDENTITY });
    let mut group = Node::group(g, vec![Arc::new(instance)]);
    group.appearance = Appearance::default_art().into();
    d.insert(Some(l), 0, group).unwrap();
    let opt = HitOptions::default();
    assert_eq!(hit_at(&d, 125.0, 25.0, opt), Some((inst, HitKind::Fill)), "where its art is drawn");
    assert_eq!(objects_at(&d, Point::new(125.0, 25.0), opt, None), [g]);
}

/// In a layer with fills and strokes of its own whose art is evaluated for a member's geometry
/// effect, an Intertwine group's members are still clicked in the order they are drawn at the
/// click: where the lower member is woven over the upper one, the click takes the lower member.
#[test]
fn an_intertwine_group_in_a_layers_drawn_art_is_hit_in_its_woven_order() {
    set_drawn_art(shifted_art);
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    d.node_mut(l).unwrap().appearance = Appearance::default_art().into();
    let mut a = square(&mut d, 0.0, 100.0);
    a.extras_mut().intertwine = Some(Box::new(shapes::rectangle(Rect::new(25.0, 100.0, 50.0, 150.0))));
    let b = square(&mut d, 25.0, 100.0);
    let (a_id, b_id, g) = (a.id, b.id, d.alloc_id());
    d.insert(Some(l), 0, Node::new(g, NodeKind::Intertwine { children: vec![Arc::new(a), Arc::new(b)] })).unwrap();
    let mut moved = square(&mut d, 200.0, 0.0);
    moved.appearance.effects.push(effect("test.shift"));
    d.insert(Some(l), 1, moved).unwrap();
    let opt = HitOptions::default();
    assert_eq!(hit_at(&d, 40.0, 125.0, opt), Some((a_id, HitKind::Fill)), "where the lower member is woven over");
    assert_eq!(hit_at(&d, 60.0, 125.0, opt), Some((b_id, HitKind::Fill)), "where the upper member is drawn over");
}

/// The clipping path of a clip group and of a clipping layer is still marqueed by its own geometry
/// when a geometry effect moves what it draws.
#[test]
fn a_clipping_path_keeps_its_own_geometry_under_a_geometry_effect() {
    set_drawn_art(shifted_art);
    let edge = Rect::new(-5.0, 40.0, 5.0, 60.0);
    // A clip group.
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let (mut mask, art) = half_clipped(&mut d);
    mask.appearance.effects.push(effect("test.shift"));
    let (g, mask_id, art_id) = (d.alloc_id(), mask.id, art.id);
    let mut clip = Node::group(g, vec![Arc::new(mask), Arc::new(art)]);
    clip.set_clips(true);
    d.insert(Some(l), 0, clip).unwrap();
    assert_eq!(marquee(&d, edge, None, true), [mask_id, art_id], "clip group");
    // A clipping layer.
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let (mut mask, art) = half_clipped(&mut d);
    mask.appearance.effects.push(effect("test.shift"));
    let (mask_id, art_id) = (mask.id, art.id);
    let layer = d.node_mut(l).unwrap();
    if let NodeKind::Layer { children, clip, .. } = &mut layer.kind {
        (*children, *clip) = (vec![Arc::new(mask), Arc::new(art)], true);
    }
    assert_eq!(marquee(&d, edge, None, true), [mask_id, art_id], "clipping layer");
}

/// The selection tools take the clipping path of a clip group whose own geometry effect moves it
/// on its outline where it is drawn, before the art it clips, and not at its own place.
#[test]
fn a_clip_groups_own_effect_moves_where_its_mask_outline_is_hit() {
    set_drawn_art(shifted_art);
    let mut d = Document::new(400.0, 200.0);
    let l = d.layers[0].id;
    let (mask, art) = half_clipped(&mut d);
    let (g, mask_id, art_id) = (d.alloc_id(), mask.id, art.id);
    let mut clip = Node::group(g, vec![Arc::new(mask), Arc::new(art)]);
    clip.set_clips(true);
    clip.appearance.effects.push(effect("test.shift"));
    d.insert(Some(l), 0, clip).unwrap();
    let tools = HitOptions { clip_outlines: true, ..HitOptions::default() };
    assert_eq!(hit_at(&d, 199.5, 50.0, tools), Some((mask_id, HitKind::Outline)), "the drawn edge, over the art");
    assert_eq!(hit_at(&d, 150.0, 50.0, tools).map(|h| h.0), Some(art_id), "the drawn art inside the mask");
    assert_eq!(hit_at(&d, 0.5, 50.0, tools), None, "the mask's own edge");
    // The same clip group in a group whose own effect moves it.
    let mut d = Document::new(400.0, 200.0);
    let l = d.layers[0].id;
    let (mask, art) = half_clipped(&mut d);
    let (g, mask_id, art_id) = (d.alloc_id(), mask.id, art.id);
    let mut clip = Node::group(g, vec![Arc::new(mask), Arc::new(art)]);
    clip.set_clips(true);
    let mut outer = Node::group(d.alloc_id(), vec![Arc::new(clip)]);
    outer.appearance.effects.push(effect("test.shift"));
    d.insert(Some(l), 0, outer).unwrap();
    assert_eq!(hit_at(&d, 199.5, 50.0, tools), Some((mask_id, HitKind::Outline)), "in a moved group: the drawn edge, over the art");
    assert_eq!(hit_at(&d, 150.0, 50.0, tools).map(|h| h.0), Some(art_id), "in a moved group: the drawn art");
}

/// In isolation mode in a clip group whose own geometry effect moves it, the selection tools take
/// the art outside the clipping mask by its own geometry, for a marquee as for a click.
#[test]
fn an_isolated_clip_group_with_its_own_effect_is_marqueed_by_its_members_own_geometry() {
    set_drawn_art(shifted_art);
    let mut d = Document::new(400.0, 200.0);
    let l = d.layers[0].id;
    let (mask, art) = half_clipped(&mut d);
    let (g, art_id) = (d.alloc_id(), art.id);
    let mut clip = Node::group(g, vec![Arc::new(mask), Arc::new(art)]);
    clip.set_clips(true);
    clip.appearance.effects.push(effect("test.shift"));
    d.insert(Some(l), 0, clip).unwrap();
    let tools = HitOptions { clip_outlines: true, scope: Some(g), ..HitOptions::default() };
    assert_eq!(hit_at(&d, 150.0, 50.0, tools).map(|h| h.0), Some(art_id), "a click outside the mask");
    let edge = Rect::new(190.0, 40.0, 210.0, 60.0);
    for leaves in [false, true] {
        assert_eq!(marquee(&d, edge, Some(g), leaves), [art_id], "a marquee across its own edge outside the mask");
    }
}

/// A clip group whose drawn art holds only its clipping path still has the art it clips clicked
/// inside the mask.
#[test]
fn a_clip_groups_art_is_hit_when_its_drawn_art_holds_only_the_clipping_path() {
    set_drawn_art(shifted_art);
    let mut d = Document::new(400.0, 200.0);
    let l = d.layers[0].id;
    let (mask, art) = half_clipped(&mut d);
    let (g, art_id) = (d.alloc_id(), art.id);
    let mut clip = Node::group(g, vec![Arc::new(mask), Arc::new(art)]);
    clip.set_clips(true);
    clip.appearance.effects.push(effect("test.mask_only"));
    d.insert(Some(l), 0, clip).unwrap();
    let opt = HitOptions::default();
    assert_eq!(hit_at(&d, 50.0, 50.0, opt), Some((art_id, HitKind::Fill)), "inside the mask");
    assert_eq!(objects_at(&d, Point::new(50.0, 50.0), opt, None), [g]);
    assert!(hit_at(&d, 150.0, 50.0, opt).is_none(), "outside the mask");
}

/// Every test of a clip group by its clipping path follows the region a live effect moved it to:
/// the selected art hidden by the mask, a click into a group holding the clip group, the locked
/// objects under the pointer, and, below a clipping layer whose Preview is on, art on a sublayer
/// whose Preview is off, which is clicked in outline while the layer above clips by its region.
#[test]
fn every_clip_test_follows_the_moved_region() {
    set_clip_region(moved_clip_region);
    let (moved, own) = (Point::new(150.0, 50.0), Point::new(50.0, 50.0));
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let (mut mask, art) = half_clipped(&mut d);
    mask.appearance.effects.push(test_move());
    let (g, art_id) = (d.alloc_id(), art.id);
    let mut clip = Node::group(g, vec![Arc::new(mask), Arc::new(art)]);
    clip.set_clips(true);
    let outer = Node::group(d.alloc_id(), vec![Arc::new(clip)]);
    d.insert(Some(l), 0, outer.clone()).unwrap();
    let opt = HitOptions::default();
    assert_eq!(selected_hidden_at(&d, own, opt, &[art_id]), Some(art_id), "hidden at the clipping path's own place");
    assert_eq!(selected_hidden_at(&d, moved, opt, &[art_id]), None, "shown in the moved region");
    let symbols = d.symbol_boxes();
    assert_eq!(hit_any(&outer, moved, opt, &symbols), Some(HitKind::Fill));
    assert_eq!(hit_any(&outer, own, opt, &symbols), None);
    d.node_mut(art_id).unwrap().locked = true;
    assert_eq!(locked_objects_at(&d, moved, opt), [art_id]);
    assert!(locked_objects_at(&d, own, opt).is_empty());

    // A clipping layer holding the mask and a sublayer whose Preview is off, with the art.
    let mut d = Document::new(300.0, 200.0);
    let top = d.layers[0].id;
    let (mut mask, art) = half_clipped(&mut d);
    mask.appearance.effects.push(test_move());
    let art_id = art.id;
    let mut sublayer = (*d.layers[0]).clone();
    sublayer.id = d.alloc_id();
    if let NodeKind::Layer { children, preview, .. } = &mut sublayer.kind {
        (*children, *preview) = (vec![Arc::new(art)], false);
    }
    d.insert(Some(top), 0, sublayer).unwrap();
    d.insert(Some(top), 0, mask).unwrap();
    d.node_mut(top).unwrap().set_clips(true);
    // The art's top edge, at the clipping path's own place and in the moved region.
    let (own, moved) = (Point::new(50.0, 0.5), Point::new(150.0, 0.5));
    assert!(hit_test(&d, own, opt).is_none());
    assert_eq!(hit_test(&d, moved, opt).map(|h| h.leaf), Some(art_id));
    assert_eq!(selected_hidden_at(&d, own, opt, &[art_id]), Some(art_id));
    assert_eq!(selected_hidden_at(&d, moved, opt, &[art_id]), None);
}

/// The art a mask hides counts only for a selected object: there a click hits nothing, and the
/// selection tools grab the selection by that art. Not where the art shows (the click hits it),
/// beyond the art, for the clip group selected as a whole, nor for art a click can't hit (locked,
/// hidden, outside the isolated container). A layer's clipping mask hides art as a clip group's
/// does.
#[test]
fn a_selected_objects_art_hidden_by_a_mask_is_found_under_the_pointer() {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let (mask, art) = half_clipped(&mut d);
    let (g, art_id) = (d.alloc_id(), art.id);
    let mut clip = Node::group(g, vec![Arc::new(mask), Arc::new(art)]);
    clip.set_clips(true);
    d.insert(Some(l), 0, clip).unwrap();
    let (hidden, shown, beyond) = (Point::new(150.0, 50.0), Point::new(50.0, 50.0), Point::new(250.0, 50.0));
    let opt = HitOptions::default();
    assert!(hit_test(&d, hidden, opt).is_none());
    assert_eq!(selected_hidden_at(&d, hidden, opt, &[art_id]), Some(art_id));
    assert_eq!(selected_hidden_at(&d, hidden, opt, &[]), None);
    assert_eq!(selected_hidden_at(&d, shown, opt, &[art_id]), None);
    assert_eq!(selected_hidden_at(&d, beyond, opt, &[art_id]), None);
    assert_eq!(selected_hidden_at(&d, hidden, opt, &[g]), None);
    assert_eq!(selected_hidden_at(&d, hidden, HitOptions { scope: Some(g), ..opt }, &[art_id]), Some(art_id));
    assert_eq!(selected_hidden_at(&d, hidden, HitOptions { scope: Some(art_id), ..opt }, &[art_id]), Some(art_id));
    let other = d.add_layer(Some("Other"));
    assert_eq!(selected_hidden_at(&d, hidden, HitOptions { scope: Some(other), ..opt }, &[art_id]), None);
    for change in [|n: &mut Node| n.locked = true, |n: &mut Node| n.visible = false] {
        let mut d = d.clone();
        change(d.node_mut(art_id).unwrap());
        assert_eq!(selected_hidden_at(&d, hidden, opt, &[art_id]), None);
    }
    // A layer's clipping mask.
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let (mask, art) = half_clipped(&mut d);
    let art_id = art.id;
    d.insert(Some(l), 0, mask).unwrap();
    d.insert(Some(l), 1, art).unwrap();
    d.node_mut(l).unwrap().set_clips(true);
    assert_eq!(selected_hidden_at(&d, hidden, opt, &[art_id]), Some(art_id));
}

/// A selected group's own art is what a click would hit in it: not a hidden or locked child, nor
/// art its own clipping mask hides, all under the pointer where an outer mask hides them.
#[test]
fn a_selected_groups_hidden_art_counts_only_where_a_click_would_hit_it() {
    // An outer clip group masking to the left half (0..100); the selected group inside it holds
    // `child` (100..200 × 0..100, under the pointer at (150, 50)) and `shown` (inside the mask).
    let build = |change: &dyn Fn(&mut Node, &mut Node)| {
        let mut d = Document::new(300.0, 200.0);
        let l = d.layers[0].id;
        let outer_mask = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default());
        let mut child = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(100.0, 0.0, 200.0, 100.0)), Appearance::default_art());
        let shown = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 40.0, 40.0)), Appearance::default_art());
        let (gid, cid) = (d.alloc_id(), d.alloc_id());
        let mut group = Node::group(gid, vec![Arc::new(shown)]);
        change(&mut group, &mut child);
        if let NodeKind::Group { children, .. } = &mut group.kind {
            children.push(Arc::new(child));
        }
        let mut clip = Node::group(cid, vec![Arc::new(outer_mask), Arc::new(group)]);
        clip.set_clips(true);
        d.insert(Some(l), 0, clip).unwrap();
        (d, gid)
    };
    let (hidden, opt) = (Point::new(150.0, 50.0), HitOptions::default());
    let (d, g) = build(&|_, _| {});
    assert_eq!(selected_hidden_at(&d, hidden, opt, &[g]), Some(g));
    let (d, g) = build(&|_, child| child.visible = false);
    assert_eq!(selected_hidden_at(&d, hidden, opt, &[g]), None, "a hidden child");
    let (d, g) = build(&|_, child| child.locked = true);
    assert_eq!(selected_hidden_at(&d, hidden, opt, &[g]), None, "a locked child");
    // The selected group is a clip group too, whose own mask (its first child, 10..40) leaves
    // `child` out.
    let (d, g) = build(&|group, _| group.set_clips(true));
    assert_eq!(selected_hidden_at(&d, hidden, opt, &[g]), None, "its own mask");
}

/// A selected guide path where a mask hides it counts only while guides can be clicked: not
/// while they are locked or hidden (`skip_guides`), as a click on it then hits nothing.
#[test]
fn a_selected_guide_hidden_by_a_mask_counts_only_while_guides_can_be_clicked() {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let (mask, mut art) = half_clipped(&mut d);
    if let NodeKind::Path { guide, .. } = &mut art.kind {
        *guide = true;
    }
    let (g, guide_id) = (d.alloc_id(), art.id);
    let mut clip = Node::group(g, vec![Arc::new(mask), Arc::new(art)]);
    clip.set_clips(true);
    d.insert(Some(l), 0, clip).unwrap();
    let (hidden, opt) = (Point::new(150.0, 50.0), HitOptions::default());
    assert_eq!(selected_hidden_at(&d, hidden, opt, &[guide_id]), Some(guide_id));
    assert_eq!(selected_hidden_at(&d, hidden, HitOptions { skip_guides: true, ..opt }, &[guide_id]), None);
}

/// #8538: while guides are locked or hidden (`skip_guides`), a marquee touches no guide path, at
/// the top of a layer or in a group (the group is not taken for it), nor in a group in a group,
/// with leaves or without. The group's other members still take it, and with the guides unlocked
/// the guide does.
#[test]
fn a_marquee_skips_guide_paths_in_groups_while_guides_are_locked() {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let guide = |d: &mut Document, r: Rect| {
        let mut n = Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::default_art());
        if let NodeKind::Path { guide, .. } = &mut n.kind {
            *guide = true;
        }
        n
    };
    let top = guide(&mut d, Rect::new(0.0, 0.0, 20.0, 20.0));
    let inner = guide(&mut d, Rect::new(100.0, 0.0, 120.0, 20.0));
    let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(200.0, 0.0, 220.0, 20.0)), Appearance::default_art());
    let (top_id, inner_id, art_id) = (top.id, inner.id, art.id);
    let (outer_id, nested_id) = (d.alloc_id(), d.alloc_id());
    let nested = Node::group(nested_id, vec![Arc::new(inner)]);
    d.insert(Some(l), 0, top).unwrap();
    d.insert(Some(l), 1, Node::group(outer_id, vec![Arc::new(nested), Arc::new(art)])).unwrap();
    let (on_top, on_inner, on_art) = (Rect::new(-5.0, 5.0, 5.0, 10.0), Rect::new(95.0, 5.0, 105.0, 10.0), Rect::new(195.0, 5.0, 205.0, 10.0));
    for leaves in [false, true] {
        let m = |r: Rect, skip: bool| marquee_in_view(&d, r, None, leaves, false, skip);
        assert!(m(on_top, true).is_empty(), "a guide at the top, leaves {leaves}");
        assert!(m(on_inner, true).is_empty(), "a guide in a group in a group, leaves {leaves}");
        assert!(m(Rect::new(0.0, 0.0, 300.0, 200.0), true).iter().all(|id| ![top_id, inner_id].contains(id)), "all, leaves {leaves}");
        assert_eq!(m(on_art, true), [if leaves { art_id } else { outer_id }], "the group's art, leaves {leaves}");
        // Unlocked: the guides are touched.
        assert_eq!(m(on_top, false), [top_id], "unlocked, leaves {leaves}");
        assert_eq!(m(on_inner, false), [if leaves { inner_id } else { outer_id }], "unlocked, leaves {leaves}");
    }
    // An enclosing loop's candidates leave them out alike.
    assert!(enclosure_candidates(&d, on_inner, None, false, true).is_empty());
    assert_eq!(enclosure_candidates(&d, on_inner, None, false, false), [inner_id]);
}

/// #8538: an enclosing loop's candidates leave out a compound path whose touched contours are all
/// locked or hidden guides, as the marquee does.
#[test]
fn guide_contours_do_not_make_a_compound_an_enclosure_candidate() {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let part = |d: &mut Document, r: Rect| {
        let mut n = Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::default_art());
        if let NodeKind::Path { guide, .. } = &mut n.kind {
            *guide = true;
        }
        Arc::new(n)
    };
    let (a, b) = (part(&mut d, Rect::new(10.0, 10.0, 50.0, 50.0)), part(&mut d, Rect::new(20.0, 20.0, 40.0, 40.0)));
    let id = d.alloc_id();
    let mut compound = Node::group(id, vec![]);
    compound.kind = NodeKind::Compound { children: vec![a, b], rule: Default::default() };
    d.insert(Some(l), 0, compound).unwrap();
    let r = Rect::new(0.0, 0.0, 60.0, 60.0);
    assert!(enclosure_candidates(&d, r, None, false, true).is_empty(), "locked or hidden");
    assert_eq!(enclosure_candidates(&d, r, None, false, false), [id], "shown and unlocked");
}

/// A selected path in a Shaper group's original shapes, where an outer mask hides it, counts only
/// while those shapes can be clicked (Construction mode, with them isolated), however deep it
/// lies in them.
#[test]
fn a_selected_path_in_a_shaper_groups_hidden_shapes_counts_only_in_construction_mode() {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    // The outer clip group masks to 0..100; the path, 100..200, lies in the Shaper group's
    // original shapes (its first child), under (150, 50).
    let mask = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default());
    let path = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(100.0, 0.0, 200.0, 100.0)), Appearance::default_art());
    let path_id = path.id;
    let mut sources = Node::group(d.alloc_id(), vec![Arc::new(path)]);
    sources.name = Some(crate::shaper::SOURCES.into());
    let sources_id = sources.id;
    let face = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 40.0, 40.0)), Appearance::default_art());
    let mut shaper = Node::group(d.alloc_id(), vec![Arc::new(sources), Arc::new(face)]);
    shaper.extras_mut().shaper = Some(Box::default());
    let mut clip = Node::group(d.alloc_id(), vec![Arc::new(mask), Arc::new(shaper)]);
    clip.set_clips(true);
    d.insert(Some(l), 0, clip).unwrap();
    let (hidden, opt) = (Point::new(150.0, 50.0), HitOptions::default());
    assert_eq!(selected_hidden_at(&d, hidden, opt, &[path_id]), None);
    assert_eq!(selected_hidden_at(&d, hidden, HitOptions { scope: Some(sources_id), ..opt }, &[path_id]), Some(path_id));
}

/// On a layer whose Preview is off, a click hits art in outline: a selected object's hidden art
/// counts on its outline, not inside its fill.
#[test]
fn a_selected_objects_hidden_art_on_a_preview_off_layer_counts_on_its_outline() {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let (mask, art) = half_clipped(&mut d);
    let (g, art_id) = (d.alloc_id(), art.id);
    let mut clip = Node::group(g, vec![Arc::new(mask), Arc::new(art)]);
    clip.set_clips(true);
    d.insert(Some(l), 0, clip).unwrap();
    if let NodeKind::Layer { preview, .. } = &mut d.node_mut(l).unwrap().kind {
        *preview = false;
    }
    let opt = HitOptions::default();
    assert_eq!(selected_hidden_at(&d, Point::new(150.0, 50.0), opt, &[art_id]), None, "inside the fill");
    assert_eq!(selected_hidden_at(&d, Point::new(200.0, 50.0), opt, &[art_id]), Some(art_id), "on the outline");
}

/// #2345: a marquee touches a clip group only where its art shows. Over the clipped-away
/// half alone it selects nothing, and around a rectangle drawn beside the group's mask it
/// takes that rectangle alone; reaching into the mask takes the group. The same holds for
/// Group Selection's leaves. In isolation mode (#3748) the isolated group's own mask does not
/// limit the marquee, as it does not limit clicks: with the group or the art isolated, the
/// marquee takes the hidden art.
#[test]
fn a_marquee_leaves_a_clip_group_by_its_clipped_away_art() {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let (mask, art) = half_clipped(&mut d);
    let (g, art_id, beside) = (d.alloc_id(), art.id, d.alloc_id());
    let mut clip = Node::group(g, vec![Arc::new(mask), Arc::new(art)]);
    clip.set_clips(true);
    d.insert(Some(l), 0, clip).unwrap();
    d.insert(Some(l), 1, Node::path(beside, shapes::rectangle(Rect::new(150.0, 80.0, 190.0, 130.0)), Appearance::default_art())).unwrap();
    // This marquee crosses only the hidden art's top and right edges.
    let hidden = Rect::new(150.0, -10.0, 210.0, 50.0);
    assert!(marquee(&d, hidden, None, false).is_empty());
    assert!(marquee(&d, hidden, None, true).is_empty());
    assert_eq!(marquee(&d, hidden, Some(g), false), vec![art_id]);
    assert_eq!(marquee(&d, hidden, Some(g), true), vec![art_id]);
    assert_eq!(marquee(&d, hidden, Some(art_id), false), vec![art_id]);
    // This marquee surrounds the rectangle beside and crosses the hidden art's bottom edge.
    assert_eq!(marquee(&d, Rect::new(140.0, 70.0, 195.0, 140.0), None, false), vec![beside]);
    // This marquee crosses the shown half's top edge and goes on into the hidden half.
    let shown = Rect::new(50.0, -10.0, 160.0, 10.0);
    assert_eq!(marquee(&d, shown, None, false), vec![g]);
    assert!(marquee(&d, shown, None, true).contains(&art_id));
    assert_eq!(marquee(&d, shown, Some(g), false).len(), 2, "the mask and the art");
}

/// #2345: in the corner of a diamond mask's box, outside the diamond, a marquee over the
/// clipped art's corner leaves the clip group. An open mask clips to its area closed by a
/// straight line, and a marquee across that line takes the art it reaches inside.
#[test]
fn a_marquee_must_meet_the_clipping_path_itself() {
    // A clip group of a mask through `pts` (closed or open) over the art `art`.
    let clipped = |pts: &[(f64, f64)], closed: bool, art: Rect| {
        let mut d = Document::new(200.0, 200.0);
        let l = d.layers[0].id;
        let mut mask = vectorcraft_geom::BezPath::new();
        mask.move_to(pts[0]);
        for &p in &pts[1..] {
            mask.line_to(p);
        }
        if closed {
            mask.close_path();
        }
        let mask = Node::path(d.alloc_id(), vectorcraft_geom::PathData::from_bezpath(&mask), Appearance::default());
        let art = Node::path(d.alloc_id(), shapes::rectangle(art), Appearance::default_art());
        let g = d.alloc_id();
        let mut clip = Node::group(g, vec![Arc::new(mask), Arc::new(art)]);
        clip.set_clips(true);
        d.insert(Some(l), 0, clip).unwrap();
        (d, g)
    };
    let (d, g) = clipped(&[(50.0, 0.0), (100.0, 50.0), (50.0, 100.0), (0.0, 50.0)], true, Rect::new(0.0, 0.0, 100.0, 100.0));
    assert!(marquee(&d, Rect::new(-10.0, -10.0, 10.0, 10.0), None, false).is_empty());
    assert_eq!(marquee(&d, Rect::new(-10.0, 40.0, 10.0, 60.0), None, false), vec![g], "over the diamond's left point");
    // The mask is the triangle above the line from (0, 0) to (100, 100), and the art lies inside it.
    let (d, g) = clipped(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)], false, Rect::new(55.0, 40.0, 70.0, 50.0));
    assert_eq!(marquee(&d, Rect::new(40.0, 45.0, 60.0, 70.0), None, false), vec![g]);
}

/// #2345: the objects of a layer or sublayer with a clipping mask are touched only where the
/// mask shows them, as in a clip group.
#[test]
fn a_marquee_leaves_art_a_layer_clips_away() {
    for sublayer in [false, true] {
        let mut d = Document::new(300.0, 200.0);
        let l = d.layers[0].id;
        let beside = d.alloc_id();
        d.insert(Some(l), 0, Node::path(beside, shapes::rectangle(Rect::new(150.0, 80.0, 190.0, 130.0)), Appearance::default_art())).unwrap();
        let masked = d.alloc_id();
        let mut layer = Node::layer(masked, "Masked", crate::LayerColor::Preset(2));
        layer.set_clips(true);
        d.insert(sublayer.then_some(l), 9, layer).unwrap();
        let (mask, art) = half_clipped(&mut d);
        let (mask_id, art_id) = (mask.id, art.id);
        d.insert(Some(masked), 0, mask).unwrap();
        d.insert(Some(masked), 1, art).unwrap();
        assert!(marquee(&d, Rect::new(150.0, -10.0, 210.0, 50.0), None, false).is_empty(), "sublayer: {sublayer}");
        assert_eq!(marquee(&d, Rect::new(140.0, 70.0, 195.0, 140.0), None, false), vec![beside], "sublayer: {sublayer}");
        assert_eq!(marquee(&d, Rect::new(50.0, -10.0, 160.0, 10.0), None, false), vec![mask_id, art_id], "sublayer: {sublayer}");
    }
}

/// A marquee touches a Shaper Group only by the art it shows, as clicks do ([`may_hit`]): over
/// the original shape of a face the group erased it selects nothing, with Group Selection's
/// leaves too, and over a face it shows it takes the group. With the original shapes isolated
/// (Construction mode) it takes the original shape it reaches.
#[test]
fn a_marquee_leaves_a_shaper_group_by_its_original_shapes() {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    // Two original squares, 0..100 and 50..150; the group shows only the first one's face.
    let square = |x: f64| shapes::rectangle(Rect::new(x, 0.0, x + 100.0, 100.0));
    let a = Node::path(d.alloc_id(), square(0.0), Appearance::default_art());
    let b = Node::path(d.alloc_id(), square(50.0), Appearance::default_art());
    let b_id = b.id;
    let mut sources = Node::group(d.alloc_id(), vec![Arc::new(a), Arc::new(b)]);
    sources.name = Some(crate::shaper::SOURCES.into());
    sources.opacity = 0.0;
    let sources_id = sources.id;
    let mut face = Node::path(d.alloc_id(), square(0.0), Appearance::default_art());
    face.name = Some(format!("{}0", crate::shaper::FILL_PREFIX));
    let face_id = face.id;
    let g = d.alloc_id();
    let mut group = Node::group(g, vec![Arc::new(sources), Arc::new(face)]);
    group.extras_mut().shaper = Some(Box::default());
    d.insert(Some(l), 0, group).unwrap();
    // This marquee crosses only the right edge of the second square, whose face is erased.
    let erased = Rect::new(140.0, 40.0, 160.0, 60.0);
    assert!(hit_test(&d, Point::new(150.0, 50.0), HitOptions::default()).is_none(), "a click there misses");
    assert!(marquee(&d, erased, None, false).is_empty());
    assert!(marquee(&d, erased, None, true).is_empty());
    assert!(marquee(&d, erased, Some(g), false).is_empty());
    // This marquee crosses the shown face's left edge.
    let shown = Rect::new(-10.0, 40.0, 10.0, 60.0);
    assert_eq!(marquee(&d, shown, None, false), vec![g]);
    assert_eq!(marquee(&d, shown, None, true), vec![face_id]);
    assert_eq!(marquee(&d, shown, Some(g), false), vec![face_id]);
    // Construction mode: the original shapes are the objects.
    assert_eq!(marquee(&d, erased, Some(sources_id), false), vec![b_id]);
}

/// A marquee touches an Intertwine group by its members, as it touches a plain group (#1103) and
/// as clicks do: in the gap between two members, or inside one member's fill, it selects nothing.
/// Across a member's edge it selects the group, and with the leaves that Direct and Group
/// Selection take, or in isolation mode, it selects the member.
#[test]
fn a_marquee_meets_an_intertwine_group_by_its_members() {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    // Two squares, 0..100 and 150..250, with a gap between them inside the group's box.
    let square = |x: f64| shapes::rectangle(Rect::new(x, 0.0, x + 100.0, 100.0));
    let mut a = Node::path(d.alloc_id(), square(0.0), Appearance::default_art());
    a.extras_mut().intertwine = Some(Box::new(shapes::rectangle(Rect::new(50.0, 0.0, 100.0, 100.0))));
    let b = Node::path(d.alloc_id(), square(150.0), Appearance::default_art());
    let (a_id, g) = (a.id, d.alloc_id());
    d.insert(Some(l), 0, Node::new(g, NodeKind::Intertwine { children: vec![Arc::new(a), Arc::new(b)] })).unwrap();
    let gap = Rect::new(110.0, 40.0, 140.0, 60.0);
    assert!(hit_test(&d, gap.center(), HitOptions::default()).is_none(), "a click in the gap misses");
    assert!(marquee(&d, gap, None, false).is_empty());
    assert!(marquee(&d, gap, None, true).is_empty());
    assert!(marquee(&d, gap, Some(g), false).is_empty());
    assert!(marquee(&d, Rect::new(20.0, 20.0, 40.0, 40.0), None, false).is_empty(), "inside a member's fill");
    // This marquee crosses the first square's left edge.
    let edge = Rect::new(-10.0, 40.0, 10.0, 60.0);
    assert_eq!(marquee(&d, edge, None, false), vec![g]);
    assert_eq!(marquee(&d, edge, None, true), vec![a_id]);
    assert_eq!(marquee(&d, edge, Some(g), false), vec![a_id]);
}

/// A marquee touches a repeat by the copies it draws, as clicks do: between two copies of a
/// grid repeat it selects nothing, and across a copy's edge it takes the repeat.
#[test]
fn a_marquee_meets_a_repeat_by_its_copies() {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let source = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::default_art());
    // Copies at 0..20, 40..60 and 80..100.
    let kind = crate::pattern::RepeatKind::Grid {
        h_spacing: 20.0,
        v_spacing: 20.0,
        rows: 1,
        cols: 3,
        grid_type: crate::pattern::TileType::Grid,
        flip_rows: false,
        flip_cols: false,
    };
    let r = d.alloc_id();
    d.insert(Some(l), 0, Node::new(r, NodeKind::Repeat(crate::pattern::RepeatSpec { kind, source: vec![Arc::new(source)] }))).unwrap();
    let gap = Rect::new(25.0, 5.0, 35.0, 15.0);
    assert!(hit_test(&d, gap.center(), HitOptions::default()).is_none(), "a click between copies misses");
    assert!(marquee(&d, gap, None, false).is_empty());
    assert!(marquee(&d, gap, None, true).is_empty());
    assert_eq!(marquee(&d, Rect::new(35.0, 5.0, 45.0, 15.0), None, false), vec![r], "across the second copy's left edge");
    assert_eq!(marquee(&d, Rect::new(35.0, 5.0, 45.0, 15.0), None, true), vec![r]);
}

/// A blend and an envelope are clicked anywhere in their box, and a marquee touches them there
/// too: inside the box, away from every key object's and the content's outline, it takes them.
#[test]
fn a_marquee_meets_blends_and_envelopes_by_their_box() {
    let mut d = Document::new(300.0, 300.0);
    let l = d.layers[0].id;
    let square = |id, x: f64, y: f64| Arc::new(Node::path(id, shapes::rectangle(Rect::new(x, y, x + 20.0, y + 20.0)), Appearance::default_art()));
    let blend = d.alloc_id();
    let keys = vec![square(d.alloc_id(), 0.0, 0.0), square(d.alloc_id(), 100.0, 0.0)];
    d.insert(Some(l), 0, Node::new(blend, NodeKind::Blend { children: keys, spec: Default::default() })).unwrap();
    let envelope = d.alloc_id();
    let content = vec![Arc::new(Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 150.0, 100.0, 250.0)), Appearance::default_art()))];
    let e = Node::new(
        envelope,
        NodeKind::Envelope {
            content,
            kind: Box::new(crate::live::EnvelopeKind::Warp { style: "arc".to_string(), bend: 0.0, h: 0.0, v: 0.0, horizontal: true, version: 0 }),
            fidelity: 100.0,
            editing: false,
            options: Default::default(),
            frame: Box::new(vectorcraft_geom::Affine::IDENTITY),
        },
    );
    d.insert(Some(l), 1, e).unwrap();
    for (r, id) in [(Rect::new(50.0, 5.0, 60.0, 15.0), blend), (Rect::new(40.0, 190.0, 60.0, 210.0), envelope)] {
        assert_eq!(hit_test(&d, r.center(), HitOptions::default()).map(|h| h.leaf), Some(id));
        assert_eq!(marquee(&d, r, None, false), vec![id]);
        assert_eq!(marquee(&d, r, None, true), vec![id]);
    }
}

/// A click just outside a blend's bounds, in the reach of its keys' strokes, tests only the steps
/// whose own reach holds the point, not every step of the evaluated blend.
#[test]
fn a_click_in_a_blends_stroke_band_tests_only_the_steps_that_reach_it() {
    let mut d = Document::new(1000.0, 1000.0);
    let l = d.layers[0].id;
    // 10 pt circles with 4 pt strokes, y 295 to 305, from x 10 to x 890: 200 steps about 4.3 pt apart.
    let stroked = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 4.0);
    let circle = |id, x: f64| Arc::new(Node::path(id, shapes::ellipse(Rect::new(x, 295.0, x + 10.0, 305.0)), stroked.clone()));
    let blend = d.alloc_id();
    let keys = vec![circle(d.alloc_id(), 10.0), circle(d.alloc_id(), 880.0)];
    let spec = crate::live::BlendSpec { spacing: crate::live::BlendSpacing::Steps(200), ..Default::default() };
    d.insert(Some(l), 0, Node::new(blend, NodeKind::Blend { children: keys, spec: Box::new(spec) })).unwrap();
    let count = |p: Point| {
        LEAVES_TESTED.with(|c| c.set(0));
        let h = hit_test(&d, p, HitOptions::default()).map(|h| h.leaf);
        (h, LEAVES_TESTED.with(std::cell::Cell::get))
    };
    // On the steps' strokes halfway between the keys, 4 pt above the bounds: a few steps reach it.
    let (h, tested) = count(Point::new(450.0, 291.0));
    assert_eq!(h, Some(blend));
    assert!(tested < 20, "{tested} objects tested");
    // In the corner of the reach, outside every stroke: nothing hits.
    let (h, tested) = count(Point::new(6.0, 291.0));
    assert_eq!(h, None);
    assert!(tested < 20, "{tested} objects tested");
}

/// An intertwine group in a live object's evaluated art (a blend key a spine moves) hits by its
/// members.
#[test]
fn evaluated_art_hits_an_intertwine_group_by_its_members() {
    let d = Document::new(200.0, 200.0);
    let square = Arc::new(Node::path(NodeId(7), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::default_art()));
    let group = Node::new(NodeId(8), NodeKind::Intertwine { children: vec![square] });
    let symbols = d.symbol_boxes();
    assert_eq!(hit_any(&group, Point::new(10.0, 10.0), HitOptions::default(), &symbols), Some(HitKind::Fill));
    assert_eq!(hit_any(&group, Point::new(50.0, 50.0), HitOptions::default(), &symbols), None);
}

/// The pointer moving in a blend's stroke band evaluates the blend once, and again after the blend
/// or a symbol of the document changes.
#[test]
fn clicks_in_a_blends_stroke_band_evaluate_it_once_until_it_changes() {
    let mut d = Document::new(1000.0, 1000.0);
    let l = d.layers[0].id;
    let stroked = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 4.0);
    let circle = |id, x: f64| Arc::new(Node::path(id, shapes::ellipse(Rect::new(x, 295.0, x + 10.0, 305.0)), stroked.clone()));
    let blend = d.alloc_id();
    let keys = vec![circle(d.alloc_id(), 10.0), circle(d.alloc_id(), 880.0)];
    let spec = crate::live::BlendSpec { spacing: crate::live::BlendSpacing::Steps(200), ..Default::default() };
    d.insert(Some(l), 0, Node::new(blend, NodeKind::Blend { children: keys, spec: Box::new(spec) })).unwrap();
    let evaluated = || LIVE_EVALUATED.with(std::cell::Cell::get);
    let before = evaluated();
    for x in [448.0, 450.0, 452.0] {
        assert_eq!(hit_test(&d, Point::new(x, 291.0), HitOptions::default()).map(|h| h.leaf), Some(blend));
    }
    assert_eq!(evaluated(), before + 1);
    // 100 steps 8.7 pt apart: a step still reaches (450, 291).
    if let Some(NodeKind::Blend { spec, .. }) = d.node_mut(blend).map(|n| &mut n.kind) {
        spec.spacing = crate::live::BlendSpacing::Steps(100);
    }
    assert_eq!(hit_test(&d, Point::new(450.0, 291.0), HitOptions::default()).map(|h| h.leaf), Some(blend));
    assert_eq!(evaluated(), before + 2);
    let art = circle(d.alloc_id(), 0.0);
    d.symbols.push(crate::Symbol { dynamic: false, name: "dot".into(), art });
    hit_test(&d, Point::new(450.0, 291.0), HitOptions::default());
    assert_eq!(evaluated(), before + 3);
    hit_test(&d, Point::new(450.0, 291.0), HitOptions::default());
    assert_eq!(evaluated(), before + 3);
}

/// Evaluated art larger than [`LIVE_KEPT_SIZE`] is not kept: each click in the blend's stroke band
/// evaluates it again.
#[test]
fn evaluated_art_past_the_kept_size_is_not_kept() {
    let mut d = Document::new(1000.0, 1000.0);
    let l = d.layers[0].id;
    let stroked = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 4.0);
    // A 10 pt circle drawn with 2,000 anchors: 202 objects of 2,000 anchors each with 200 steps.
    let circle = |id, x: f64| {
        let points: Vec<Point> = (0..2000)
            .map(|i| {
                let a = f64::from(i) * std::f64::consts::TAU / 2000.0;
                Point::new(x + 5.0 + 5.0 * a.cos(), 300.0 + 5.0 * a.sin())
            })
            .collect();
        let path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&points, true));
        Arc::new(Node::path(id, path, stroked.clone()))
    };
    let blend = d.alloc_id();
    let keys = vec![circle(d.alloc_id(), 10.0), circle(d.alloc_id(), 880.0)];
    let spec = crate::live::BlendSpec { spacing: crate::live::BlendSpacing::Steps(200), ..Default::default() };
    d.insert(Some(l), 0, Node::new(blend, NodeKind::Blend { children: keys, spec: Box::new(spec) })).unwrap();
    let evaluated = || LIVE_EVALUATED.with(std::cell::Cell::get);
    let before = evaluated();
    for x in [448.0, 450.0] {
        assert_eq!(hit_test(&d, Point::new(x, 291.0), HitOptions::default()).map(|h| h.leaf), Some(blend));
    }
    assert_eq!(evaluated(), before + 2);
}

/// While the document's symbols hold a placed document, whose art changes when its file is read,
/// each click in a blend's stroke band evaluates the blend again.
#[test]
fn evaluated_art_is_not_kept_while_a_placed_document_may_show_in_it() {
    let mut d = Document::new(1000.0, 1000.0);
    let l = d.layers[0].id;
    let stroked = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 4.0);
    let circle = |id, x: f64| Arc::new(Node::path(id, shapes::ellipse(Rect::new(x, 295.0, x + 10.0, 305.0)), stroked.clone()));
    let blend = d.alloc_id();
    let keys = vec![circle(d.alloc_id(), 10.0), circle(d.alloc_id(), 880.0)];
    let spec = crate::live::BlendSpec { spacing: crate::live::BlendSpacing::Steps(200), ..Default::default() };
    d.insert(Some(l), 0, Node::new(blend, NodeKind::Blend { children: keys, spec: Box::new(spec) })).unwrap();
    let placed = crate::PlacedDocument {
        link: crate::LinkInfo::new("/art/logo.vectorcraft"),
        key: "logo".into(),
        bounding: false,
        page_box: crate::PageBox::Crop,
        width: 80.0,
        height: 40.0,
        xf: vectorcraft_geom::Affine::IDENTITY,
        placement: Default::default(),
    };
    let art = Arc::new(Node::new(d.alloc_id(), NodeKind::PlacedDocument(Box::new(placed))));
    d.symbols.push(crate::Symbol { dynamic: false, name: "logo".into(), art });
    let evaluated = || LIVE_EVALUATED.with(std::cell::Cell::get);
    let before = evaluated();
    for x in [448.0, 450.0] {
        assert_eq!(hit_test(&d, Point::new(x, 291.0), HitOptions::default()).map(|h| h.leaf), Some(blend));
    }
    assert_eq!(evaluated(), before + 2);
}

/// For instances of the symbols whose names start with "Painted", a fill that paints the square
/// from (300, 0) to (320, 20); nothing for anything else.
fn painted_far_away(_: Option<&Document>, n: &Node) -> Vec<(BezPath, FillRule, AppearanceItem)> {
    match &n.kind {
        NodeKind::SymbolInstance { symbol, .. } if symbol.starts_with("Painted") => {
            let square = shapes::rectangle(Rect::new(300.0, 0.0, 320.0, 20.0)).to_bezpath();
            vec![(square, FillRule::NonZero, AppearanceItem::Fill(crate::FillLayer::new(Paint::solid(Color::rgb(1.0, 0.0, 0.0)))))]
        }
        _ => vec![],
    }
}

/// Clicks and hovers measure the shapes a symbol instance's fill and stroke effects paint once,
/// and again only after the instance or the symbols change.
#[test]
fn clicks_measure_a_symbol_instances_painted_shapes_once_until_it_changes() {
    set_image_paint(painted_far_away);
    let mut d = Document::new(1000.0, 1000.0);
    let l = d.layers[0].id;
    let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::default_art());
    d.symbols.push(crate::Symbol { dynamic: false, name: "Painted".into(), art: Arc::new(art) });
    let inst = d.alloc_id();
    let mut n = Node::new(inst, NodeKind::SymbolInstance { symbol: "Painted".into(), xf: vectorcraft_geom::Affine::IDENTITY });
    n.appearance = Appearance::default_art().into();
    let offset = crate::Effect { id: "path.offsetPath".into(), params: serde_json::json!({"offset": 10}), visible: true };
    n.appearance.items[0].effects_mut().push(offset);
    d.insert(Some(l), 0, n).unwrap();
    let measured = || PAINT_EVALUATED.with(std::cell::Cell::get);
    let before = measured();
    let at = |d: &Document, x: f64| hit_test(d, Point::new(x, 10.0), HitOptions::default()).map(|h| h.leaf);
    assert_eq!(at(&d, 310.0), Some(inst), "a click where the instance's fill paints");
    assert_eq!(at(&d, 500.0), None);
    assert_eq!(at(&d, 10.0), Some(inst));
    assert_eq!(measured(), before + 1);
    d.node_mut(inst).unwrap().appearance.items[0].effects_mut()[0].params = serde_json::json!({"offset": 12});
    assert_eq!(at(&d, 310.0), Some(inst));
    assert_eq!(measured(), before + 2, "a changed instance is measured again");
    let other = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 5.0, 5.0)), Appearance::default_art());
    d.symbols.push(crate::Symbol { dynamic: false, name: "Other".into(), art: Arc::new(other) });
    assert_eq!(at(&d, 310.0), Some(inst));
    assert_eq!(measured(), before + 3, "changed symbols measure it again");
    assert_eq!(at(&d, 310.0), Some(inst));
    assert_eq!(measured(), before + 3);
    // An image's pixels change when its file is read again, so with one in a symbol's art every
    // click measures the instance.
    let xf = vectorcraft_geom::Affine::IDENTITY;
    let im = crate::ImageObject { key: "k".into(), width: 10, height: 10, xf, link: None, placement: Default::default(), pixels: None };
    let image = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    d.symbols.push(crate::Symbol { dynamic: false, name: "Photo".into(), art: Arc::new(image) });
    assert_eq!(at(&d, 310.0), Some(inst));
    assert_eq!(at(&d, 310.0), Some(inst));
    assert!(measured() >= before + 5, "each click measures it: {}", measured() - before);
}

/// A kept evaluation is the evaluation of the object only while the fonts and plug-ins
/// ([`LiveInputs`]) are as they were.
#[test]
fn a_kept_evaluation_is_stale_after_the_live_inputs_change() {
    let d = Document::new(100.0, 100.0);
    let n = Node::path(NodeId(3), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
    let e = kept_evaluation(&d, &n);
    assert!(e.is_of(&d, &n, 7));
    assert!(!e.is_of(&d, &n, 8));
}

/// An evaluation of `n` in `d`, with inputs 7, holding no symbols, patterns or art.
fn kept_evaluation(d: &Document, n: &Node) -> Evaluated<Vec<Node>> {
    Evaluated {
        node: n.clone(),
        symbols: vec![],
        patterns: vec![],
        swatches: d.swatches.clone(),
        swatch_groups: d.swatch_groups.clone(),
        unknown: d.unknown.clone(),
        inputs: 7,
        art: std::rc::Rc::new(vec![]),
        size: 0,
    }
}

/// A kept evaluation is stale once a symbol's 9-slice guides or natural size change: they place
/// the symbol's art, and the document keeps them apart from the symbols.
#[test]
fn a_kept_evaluation_is_stale_after_a_symbols_placement_changes() {
    let mut d = Document::new(100.0, 100.0);
    let n = Node::path(NodeId(3), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
    let e = kept_evaluation(&d, &n);
    assert!(e.is_of(&d, &n, 7));
    d.unknown.insert(crate::SYMBOL_SIZES.into(), serde_json::json!({"Logo": [40.0, 20.0]}));
    assert!(!e.is_of(&d, &n, 7));
}

/// A kept evaluation is stale once a swatch changes, in a color group too: a painted live effect's
/// pieces name swatches.
#[test]
fn a_kept_evaluation_is_stale_after_a_swatch_changes() {
    let mut d = Document::new(100.0, 100.0);
    let n = Node::path(NodeId(3), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
    let e = kept_evaluation(&d, &n);
    assert!(e.is_of(&d, &n, 7));
    d.swatches.first_mut().unwrap().name = "Renamed".into();
    assert!(!e.is_of(&d, &n, 7));
    let mut d = Document::new(100.0, 100.0);
    let e = kept_evaluation(&d, &n);
    d.swatch_groups.push(vectorcraft_color::SwatchGroup { name: "Added".into(), swatches: vec![] });
    assert!(!e.is_of(&d, &n, 7));
}

/// A rectangle (40, 40)–(140, 140) clipping a filled 200 × 200 square drawn over its edges, in a
/// clip group (or, `layer`, as the first layer's clipping mask): the group (or the layer), the
/// clipping path and the art.
fn masked_square(layer: bool) -> (Document, NodeId, NodeId, NodeId) {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let (g, clip, art) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    let mut path = Node::path(clip, shapes::rectangle(Rect::new(40.0, 40.0, 140.0, 140.0)), Appearance::basic(Paint::None, Paint::None, 0.0));
    if let NodeKind::Path { clipping, .. } = &mut path.kind {
        *clipping = true;
    }
    let picture = Node::path(art, shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 200.0)), Appearance::default_art());
    if layer {
        d.insert(Some(l), 0, path).unwrap();
        d.insert(Some(l), 1, picture).unwrap();
        d.node_mut(l).unwrap().set_clips(true);
        (d, l, clip, art)
    } else {
        d.insert(Some(l), 0, Node::new(g, NodeKind::Group { children: vec![Arc::new(path), Arc::new(picture)], clip: true })).unwrap();
        (d, g, clip, art)
    }
}

const SELECTING: HitOptions = HitOptions {
    tol: 3.0,
    outline: false,
    path_only: false,
    type_path_only: false,
    skip_guides: false,
    clip_outlines: true,
    area_type_interior: false,
    ignore_layer_preview: false,
    unfilled_interiors: false,
    scope: None,
};

/// For the selection tools a clip group's clipping path is hit on its outline, within the
/// tolerance, before the art it clips and just outside it too; inside, the art is hit.
#[test]
fn a_clipping_path_is_hit_on_its_outline_before_its_art() {
    let (d, g, clip, art) = masked_square(false);
    for (x, y) in [(140.0, 90.0), (141.5, 90.0), (138.5, 90.0), (90.0, 40.0)] {
        let h = hit_test(&d, Point::new(x, y), SELECTING).unwrap();
        assert_eq!((h.leaf, h.kind, h.top_object(None)), (clip, HitKind::Outline, g), "at {x}, {y}");
    }
    let h = hit_test(&d, Point::new(90.0, 90.0), SELECTING).unwrap();
    assert_eq!((h.leaf, h.top_object(None)), (art, g));
    // Beyond the tolerance outside it, the clipped art is not there to hit.
    assert!(hit_test(&d, Point::new(150.0, 90.0), SELECTING).is_none());
}

/// Without `clip_outlines` (the tools that act on what is drawn) and for read-only sampling
/// (the eyedropper), the art is hit on the mask's edge, as before: the clipping path paints
/// nothing there.
#[test]
fn only_the_selection_tools_hit_a_clipping_path_on_its_outline() {
    let (d, _, clip, art) = masked_square(false);
    let p = Point::new(138.5, 90.0);
    assert_eq!(hit_test(&d, p, HitOptions::default()).map(|h| h.leaf), Some(art));
    assert_eq!(hit_test_visible(&d, p, HitOptions::default()).map(|h| h.leaf), Some(art));
    assert_eq!(hit_test_visible(&d, p, SELECTING).map(|h| h.leaf), Some(art));
    assert_eq!(hit_test(&d, p, SELECTING).map(|h| h.leaf), Some(clip));
    // A hidden or locked clipping path is passed over.
    for change in [|n: &mut Node| n.visible = false, |n: &mut Node| n.locked = true] {
        let mut d = d.clone();
        change(d.node_mut(clip).unwrap());
        assert_ne!(hit_test(&d, p, SELECTING).map(|h| h.leaf), Some(clip));
    }
}

/// A layer's clipping mask is hit on its outline too, and Select Behind lists what the
/// repeated hit test finds: the mask on and just outside its edge, then the art inside it;
/// well inside, the art alone (the mask paints nothing there).
#[test]
fn select_behind_takes_a_layer_masks_outline_first() {
    let (d, _, clip, art) = masked_square(true);
    for (x, want) in [(140.0, vec![clip]), (141.5, vec![clip]), (138.5, vec![clip, art]), (90.0, vec![art])] {
        let p = Point::new(x, 90.0);
        assert_eq!(objects_at(&d, p, SELECTING, None), want, "x = {x}");
        assert_eq!(objects_at_repeated(&d, p, SELECTING, None), want, "x = {x}");
    }
    // Without clip_outlines, as before.
    assert_eq!(objects_at(&d, Point::new(138.5, 90.0), HitOptions::default(), None), vec![art, clip]);
}

/// #3748: in isolation mode the selection tools hit the art that the isolated clip group (or
/// the isolated layer with a clipping mask) hides outside its mask, and the art when it is the
/// single object isolated; the mask's outline is still hit first. Outside isolation mode, and
/// for the tools that act on what is drawn, the hidden art is not hit. The Unlock submenu
/// lists the hidden art when it is locked.
#[test]
fn isolation_reaches_the_art_a_clipping_mask_hides() {
    let hidden = Point::new(170.0, 90.0);
    for layer in [false, true] {
        let (mut d, g, clip, art) = masked_square(layer);
        let isolated = HitOptions { scope: Some(g), ..SELECTING };
        assert!(hit_test(&d, hidden, SELECTING).is_none(), "layer {layer}: not isolated");
        let h = hit_test(&d, hidden, isolated);
        assert_eq!(h.map(|h| (h.leaf, h.top_object(Some(g)))), Some((art, art)), "layer {layer}");
        assert_eq!(objects_at(&d, hidden, SELECTING, Some(g)), vec![art], "layer {layer}");
        assert_eq!(hit_test(&d, Point::new(140.0, 90.0), isolated).map(|h| h.leaf), Some(clip), "layer {layer}: the outline first");
        let one = HitOptions { scope: Some(art), ..SELECTING };
        assert_eq!(hit_test(&d, hidden, one).map(|h| h.leaf), Some(art), "layer {layer}: the art isolated");
        assert!(hit_test(&d, hidden, HitOptions { scope: Some(g), ..HitOptions::default() }).is_none(), "layer {layer}: drawn art only");
        assert!(hit_test_visible(&d, hidden, isolated).is_none(), "layer {layer}: sampling");
        d.node_mut(art).unwrap().locked = true;
        assert_eq!(locked_objects_at(&d, hidden, HitOptions { scope: Some(g), ..HitOptions::default() }), vec![art], "layer {layer}");
        assert!(locked_objects_at(&d, hidden, HitOptions::default()).is_empty(), "layer {layer}: not isolated");
    }
}

/// #3748, nested clip groups: with the outer clip group isolated, a click or a marquee takes
/// the art its own mask hides, while the mask of a clip group inside it still limits that
/// group's art. With the inner clip group isolated, or its art, the inner mask does not.
#[test]
fn isolation_reaches_hidden_art_only_for_the_isolated_clip_group() {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let (outer, outer_clip, outer_art, inner, inner_clip, inner_art) =
        (d.alloc_id(), d.alloc_id(), d.alloc_id(), d.alloc_id(), d.alloc_id(), d.alloc_id());
    let mask = |id, r| {
        let mut path = Node::path(id, shapes::rectangle(r), Appearance::basic(Paint::None, Paint::None, 0.0));
        if let NodeKind::Path { clipping, .. } = &mut path.kind {
            *clipping = true;
        }
        Arc::new(path)
    };
    let art = |id, r| Arc::new(Node::path(id, shapes::rectangle(r), Appearance::default_art()));
    let inner_group = Node::new(
        inner,
        NodeKind::Group {
            children: vec![mask(inner_clip, Rect::new(60.0, 60.0, 120.0, 120.0)), art(inner_art, Rect::new(40.0, 40.0, 160.0, 160.0))],
            clip: true,
        },
    );
    let children =
        vec![mask(outer_clip, Rect::new(20.0, 20.0, 180.0, 180.0)), art(outer_art, Rect::new(190.0, 20.0, 260.0, 180.0)), Arc::new(inner_group)];
    d.insert(Some(l), 0, Node::new(outer, NodeKind::Group { children, clip: true })).unwrap();
    // `outer_hidden` is outside the outer mask, on the outer group's own art; `inner_hidden` is
    // inside the outer mask and outside the inner one, on the inner group's art.
    let (outer_hidden, inner_hidden, shown) = (Point::new(220.0, 100.0), Point::new(150.0, 100.0), Point::new(90.0, 90.0));
    let leaf = |p, scope| hit_test(&d, p, HitOptions { scope, ..SELECTING }).map(|h| h.leaf);
    assert_eq!((leaf(outer_hidden, None), leaf(inner_hidden, None)), (None, None), "not isolated");
    assert_eq!(leaf(outer_hidden, Some(outer)), Some(outer_art), "outer isolated: its own hidden art");
    assert_eq!(leaf(inner_hidden, Some(outer)), None, "outer isolated: the inner mask still clips");
    let h = hit_test(&d, shown, HitOptions { scope: Some(outer), ..SELECTING });
    assert_eq!(h.map(|h| (h.leaf, h.top_object(Some(outer)))), Some((inner_art, inner)), "outer isolated: shown art");
    assert_eq!(leaf(inner_hidden, Some(inner)), Some(inner_art), "inner isolated");
    assert_eq!(leaf(outer_hidden, Some(inner)), None, "inner isolated: the outer group's art");
    assert_eq!(leaf(inner_hidden, Some(inner_art)), Some(inner_art), "the inner art isolated");
    // These marquees cross the right edge of the outer art (outside the outer mask) and of the
    // inner art (inside the outer mask, outside the inner one).
    let (outer_edge, inner_edge) = (Rect::new(255.0, 95.0, 265.0, 105.0), Rect::new(155.0, 95.0, 165.0, 105.0));
    assert_eq!(marquee(&d, outer_edge, Some(outer), false), vec![outer_art]);
    assert!(marquee(&d, inner_edge, Some(outer), false).is_empty());
    assert!(marquee(&d, inner_edge, Some(outer), true).is_empty());
    assert_eq!(marquee(&d, inner_edge, Some(inner), false), vec![inner_art]);
    assert!(marquee(&d, inner_edge, None, false).is_empty());
    assert!(marquee(&d, outer_edge, None, false).is_empty());
}

/// Art in a sublayer is an object of its own: a click or a marquee takes it, not the
/// sublayer; hidden, locked and template sublayers block both.
#[test]
fn sublayers_are_not_objects() {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let sub = d.alloc_id();
    d.insert(Some(l), 0, Node::layer(sub, "Sub", crate::LayerColor::Preset(2))).unwrap();
    let inner = d.alloc_id();
    d.insert(Some(sub), 0, Node::layer(inner, "Inner", crate::LayerColor::Preset(3))).unwrap();
    let a = d.alloc_id();
    d.insert(Some(inner), 0, Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art())).unwrap();
    let g = d.alloc_id();
    let b = d.alloc_id();
    let pb = Node::path(b, shapes::rectangle(Rect::new(100.0, 0.0, 150.0, 50.0)), Appearance::default_art());
    d.insert(Some(sub), 9, Node::group(g, vec![Arc::new(pb)])).unwrap();
    let h = hit_test(&d, Point::new(10.0, 10.0), HitOptions::default()).unwrap();
    assert_eq!((h.leaf, h.top_object(None), h.layers), (a, a, 3));
    let h = hit_test(&d, Point::new(110.0, 10.0), HitOptions::default()).unwrap();
    assert_eq!((h.top_object(None), h.top_object(Some(g))), (g, b));
    let all = Rect::new(-5.0, -5.0, 200.0, 200.0);
    assert_eq!(marquee(&d, all, None, false), vec![a, g]);
    assert_eq!(d.layer_containing(a), Some(inner));
    assert_eq!(d.layer_color(a), crate::LayerColor::Preset(3).rgb(), "the sublayer's own colour");
    assert_eq!(d.selectable_art(), vec![a, g]);
    for f in [
        |n: &mut Node| n.visible = false,
        |n: &mut Node| n.locked = true,
        |n: &mut Node| {
            if let NodeKind::Layer { template, .. } = &mut n.kind {
                *template = true;
            }
        },
    ] {
        let mut d2 = d.clone();
        f(d2.node_mut(inner).unwrap());
        assert!(hit_test(&d2, Point::new(10.0, 10.0), HitOptions::default()).is_none());
        assert_eq!(marquee(&d2, all, None, false), vec![g]);
        assert_eq!(d2.selectable_art(), vec![g]);
    }
    // A sublayer whose Preview is off is clicked in outline: its fill no longer hits.
    if let NodeKind::Layer { preview, .. } = &mut d.node_mut(inner).unwrap().kind {
        *preview = false;
    }
    assert!(hit_test(&d, Point::new(25.0, 25.0), HitOptions::default()).is_none());
    assert!(hit_test(&d, Point::new(0.5, 25.0), HitOptions::default()).is_some());
}

/// Object Selection by Path Only: a filled path or compound path hits on its outline only.
#[test]
fn path_only_leaves_out_fills() {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let (a, c) = (d.alloc_id(), d.alloc_id());
    d.insert(Some(l), 0, Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art())).unwrap();
    let inner = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(100.0, 0.0, 150.0, 50.0)), Appearance::default());
    let mut compound = Node::new(c, NodeKind::Compound { children: vec![Arc::new(inner)], rule: vectorcraft_geom::FillRule::NonZero });
    compound.appearance = Appearance::default_art().into();
    d.insert(Some(l), 9, compound).unwrap();
    let path_only = HitOptions { path_only: true, ..Default::default() };
    for (id, inside, edge) in [(a, Point::new(25.0, 25.0), Point::new(0.5, 25.0)), (c, Point::new(125.0, 25.0), Point::new(100.5, 25.0))] {
        assert_eq!(hit_test(&d, inside, HitOptions::default()).map(|h| h.leaf), Some(id), "the fill hits");
        assert!(hit_test(&d, inside, path_only).is_none(), "path only: the fill of {id:?} doesn't hit");
        assert_eq!(hit_test(&d, edge, path_only).map(|h| h.leaf), Some(id), "path only: the outline does");
    }
}

/// #3291: with Object Selection by Path Only, a placed image is picked on its frame's edge, so
/// a path under it can be picked through it.
#[test]
fn path_only_picks_paths_through_images() {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let (path, image) = (d.alloc_id(), d.alloc_id());
    d.insert(Some(l), 0, Node::path(path, shapes::rectangle(Rect::new(20.0, 20.0, 60.0, 60.0)), Appearance::default_art())).unwrap();
    // 100×50 pixels, scaled to 100×100 and moved to (10, 10): over the whole path.
    let xf = vectorcraft_geom::Affine::translate((10.0, 10.0)) * vectorcraft_geom::Affine::scale_non_uniform(1.0, 2.0);
    let im = crate::ImageObject { key: "k".into(), width: 100, height: 50, xf, link: None, placement: Default::default(), pixels: None };
    d.insert(Some(l), 1, Node::new(image, NodeKind::Image(Box::new(im)))).unwrap();
    let path_only = HitOptions { path_only: true, ..Default::default() };
    let hit = |p: Point, opt| hit_test(&d, p, opt).map(|h| h.leaf);
    let (path_edge, inside, image_edge) = (Point::new(20.5, 40.0), Point::new(80.0, 80.0), Point::new(80.0, 109.5));
    assert_eq!(hit(path_edge, HitOptions::default()), Some(image), "the image covers the path");
    assert_eq!(hit(inside, HitOptions::default()), Some(image));
    assert_eq!(hit(path_edge, path_only), Some(path), "path only: the path through the image");
    assert_eq!(hit(inside, path_only), None, "path only: not inside the image");
    assert_eq!(hit(image_edge, path_only), Some(image), "path only: the image's frame");
}

/// #6437: in outline mode an image is drawn as its frame and a gradient mesh as its mesh lines,
/// so a click inside them, on the outline of a path behind, picks that path, and a click on the
/// frame or a mesh line picks the image or the mesh. In Preview they cover the path as before,
/// and an image whose pixels show in outline mode (Show Images in Outline Mode) still does.
#[test]
fn outline_mode_picks_paths_through_images_and_meshes() {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let (path, image, under, mesh) = (d.alloc_id(), d.alloc_id(), d.alloc_id(), d.alloc_id());
    d.insert(Some(l), 0, Node::path(path, shapes::rectangle(Rect::new(20.0, 20.0, 60.0, 60.0)), Appearance::default_art())).unwrap();
    let xf = vectorcraft_geom::Affine::translate((10.0, 10.0));
    let im = crate::ImageObject { key: "k".into(), width: 80, height: 80, xf, link: None, placement: Default::default(), pixels: None };
    d.insert(Some(l), 1, Node::new(image, NodeKind::Image(Box::new(im)))).unwrap();
    d.insert(Some(l), 2, Node::path(under, shapes::rectangle(Rect::new(120.0, 120.0, 160.0, 160.0)), Appearance::default_art())).unwrap();
    let area = shapes::rectangle(Rect::new(110.0, 110.0, 190.0, 190.0));
    let m = crate::GradientMesh::for_path_with(&area, 2, 2, &|_| (Color::BLACK, 1.0)).unwrap();
    d.insert(Some(l), 3, Node::new(mesh, NodeKind::Mesh(m))).unwrap();
    let outline = HitOptions { outline: true, ..Default::default() };
    let hit = |d: &Document, p: Point, opt| hit_test(d, p, opt).map(|h| h.leaf);
    // The image: the path's edge and the inside, then the frame.
    let (path_edge, inside, frame) = (Point::new(20.5, 40.0), Point::new(80.0, 80.0), Point::new(89.5, 50.0));
    assert_eq!(hit(&d, path_edge, HitOptions::default()), Some(image), "Preview: the image covers the path");
    assert_eq!(hit(&d, inside, HitOptions::default()), Some(image));
    assert_eq!(hit(&d, path_edge, outline), Some(path), "outline: the path through the image");
    assert_eq!(hit(&d, inside, outline), None, "outline: not inside the image");
    assert_eq!(hit(&d, frame, outline), Some(image), "outline: the image's frame");
    // The mesh: the path's edge and the inside, then its edge and a mesh line inside it.
    let (under_edge, inside, edge, line) = (Point::new(120.5, 130.0), Point::new(130.0, 170.0), Point::new(189.5, 130.0), Point::new(150.0, 170.0));
    assert_eq!(hit(&d, under_edge, HitOptions::default()), Some(mesh), "Preview: the mesh covers the path");
    assert_eq!(hit(&d, inside, HitOptions::default()), Some(mesh));
    assert_eq!(hit(&d, under_edge, outline), Some(under), "outline: the path through the mesh");
    assert_eq!(hit(&d, inside, outline), None, "outline: not between the mesh lines");
    assert_eq!(hit(&d, edge, outline), Some(mesh), "outline: the mesh's edge");
    assert_eq!(hit(&d, line, outline), Some(mesh), "outline: a mesh line");
    // Show Images in Outline Mode: the image's pixels are drawn, and it covers the path again.
    d.setup.outline_images = true;
    assert_eq!(hit(&d, path_edge, outline), Some(image));
}

/// Type Object Selection by Path Only: point type is picked on its baseline, area type on its
/// frame and type on a path on its path, no longer anywhere in its bounds.
#[test]
fn type_path_only_picks_type_on_its_path() {
    use crate::{CharStyle, TextKind, TextObject};
    let mut d = Document::new(400.0, 400.0);
    let l = d.layers[0].id;
    let mut add = |t: TextObject| {
        let id = d.alloc_id();
        d.insert(Some(l), 9, Node::new(id, NodeKind::Text(Box::new(t)))).unwrap();
        id
    };
    // Point type at (10, 100), 20 pt: its glyphs rise above the baseline, its layout is 60 wide.
    let mut point = TextObject::point(Point::new(10.0, 100.0), "Hello", CharStyle { size: 20.0, ..CharStyle::default() });
    point.cached_bounds = Some(Rect::new(0.0, -16.0, 60.0, 4.0));
    let point = add(point);
    let mut area = TextObject::point(Point::new(200.0, 200.0), "Area", CharStyle::default());
    area.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 50.0)) };
    let area = add(area);
    let mut on_path = TextObject::point(Point::new(10.0, 300.0), "Path", CharStyle::default());
    on_path.kind = TextKind::OnPath { path: shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 40.0)), start: 0.0, end: None };
    let on_path = add(on_path);
    let path_only = HitOptions { type_path_only: true, ..Default::default() };
    let hit = |p: Point, opt: HitOptions| hit_test(&d, p, opt).map(|h| (h.leaf, h.kind));
    // Off: anywhere in the bounds.
    assert_eq!(hit(Point::new(40.0, 90.0), HitOptions::default()), Some((point, HitKind::Bounds)));
    assert_eq!(hit(Point::new(250.0, 225.0), HitOptions::default()), Some((area, HitKind::Bounds)));
    assert_eq!(hit(Point::new(60.0, 320.0), HitOptions::default()), Some((on_path, HitKind::Bounds)));
    // On: the glyphs and the inside of the frame or path no longer pick the type.
    assert_eq!(hit(Point::new(40.0, 90.0), path_only), None, "point type: among its glyphs");
    assert_eq!(hit(Point::new(250.0, 225.0), path_only), None, "area type: inside its frame");
    assert_eq!(hit(Point::new(60.0, 320.0), path_only), None, "type on a path: inside its path");
    // The baseline (within the tolerance, across the layout's width), frame and path do.
    assert_eq!(hit(Point::new(40.0, 101.5), path_only), Some((point, HitKind::Outline)), "point type: its baseline");
    assert_eq!(hit(Point::new(75.0, 100.0), path_only), None, "point type: past the end of its layout");
    assert_eq!(hit(Point::new(250.0, 200.5), path_only), Some((area, HitKind::Outline)), "area type: its frame");
    assert_eq!(hit(Point::new(60.0, 339.5), path_only), Some((on_path, HitKind::Outline)), "type on a path: its path");
}

/// Area type is hit on its frame's edge and where its lines are laid out, not in the empty rest of
/// its frame: a click there reaches the art behind, in Preview and in Outline mode (#7274). The
/// Type tool still finds the type anywhere inside its frame.
#[test]
fn area_type_passes_clicks_through_its_empty_frame() {
    use crate::{CharStyle, TextKind, TextObject};
    let mut d = Document::new(400.0, 400.0);
    let l = d.layers[0].id;
    // A filled square behind the type, its edge at x = 150 inside the frame.
    let square = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(150.0, 150.0, 350.0, 350.0)), Appearance::default_art());
    let square_id = square.id;
    d.insert(Some(l), 9, square).unwrap();
    // Area type at (100, 100): a 200 × 200 frame with one 80 wide line at its top.
    let mut area = TextObject::point(Point::new(100.0, 100.0), "Area", CharStyle { size: 20.0, ..CharStyle::default() });
    area.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 200.0)) };
    area.cached_bounds = Some(Rect::new(0.0, 0.0, 80.0, 24.0));
    area.cached_baselines = vec![(Point::new(0.0, 18.0), Point::new(80.0, 18.0))];
    let area_id = d.alloc_id();
    d.insert(Some(l), 9, Node::new(area_id, NodeKind::Text(Box::new(area)))).unwrap();
    let outline = HitOptions { outline: true, ..Default::default() };
    for (opt, mode) in [(HitOptions::default(), "preview"), (outline, "outline")] {
        let hit = |p: Point| hit_test(&d, p, opt).map(|h| h.leaf);
        assert_eq!(hit(Point::new(140.0, 110.0)), Some(area_id), "{mode}: on the line of type");
        assert_eq!(hit(Point::new(200.0, 100.5)), Some(area_id), "{mode}: on the frame's top edge");
        assert_eq!(hit(Point::new(299.0, 250.0)), Some(area_id), "{mode}: on the frame's right edge");
        assert_eq!(hit(Point::new(220.0, 130.0)), None, "{mode}: the empty frame, nothing behind");
        assert_eq!(hit(Point::new(120.0, 250.0)), None, "{mode}: the empty frame, nothing behind");
        assert_eq!(hit(Point::new(150.0, 250.0)), Some(square_id), "{mode}: the square's edge behind the empty frame");
    }
    assert_eq!(hit_test(&d, Point::new(250.0, 250.0), HitOptions::default()).map(|h| h.leaf), Some(square_id), "the square's fill");
    // The Type tool: anywhere inside the frame.
    let typing = HitOptions { area_type_interior: true, ..Default::default() };
    assert_eq!(hit_test(&d, Point::new(250.0, 250.0), typing).map(|h| h.leaf), Some(area_id), "type tool: inside the frame");
}

/// Type Object Selection by Path Only with the layout's baselines: every line's for point type,
/// area type's besides its frame, a vertical column's centre line, and type in perspective
/// where it is drawn.
#[test]
fn type_path_only_picks_every_baseline() {
    use crate::{CharStyle, PerspectiveAttachment, TextKind, TextObject};
    use vectorcraft_geom::{Affine, Homography};
    let mut d = Document::new(400.0, 400.0);
    let l = d.layers[0].id;
    let mut add = |t: TextObject, perspective: Option<Affine>| {
        let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
        n.extras_mut().perspective = perspective.map(|a| {
            Box::new(PerspectiveAttachment { projection: Some(Homography::from_affine(a).to_array()), ..PerspectiveAttachment::new("left", 0.0) })
        });
        let id = n.id;
        d.insert(Some(l), 9, n).unwrap();
        id
    };
    let style = CharStyle { size: 20.0, ..CharStyle::default() };
    // Two lines of point type at (10, 100), 24 pt apart.
    let mut point = TextObject::point(
        Point::new(10.0, 100.0),
        "Hello
world",
        style.clone(),
    );
    point.cached_bounds = Some(Rect::new(0.0, -16.0, 60.0, 28.0));
    point.cached_baselines = vec![(Point::ZERO, Point::new(60.0, 0.0)), (Point::new(0.0, 24.0), Point::new(50.0, 24.0))];
    let point = add(point, None);
    // Area type: a line inside its frame.
    let mut area = TextObject::point(Point::new(200.0, 50.0), "Area", style.clone());
    area.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 50.0)) };
    area.cached_baselines = vec![(Point::new(0.0, 16.0), Point::new(40.0, 16.0))];
    let area = add(area, None);
    // Vertical point type at (300, 200) with no baselines cached: down its first column.
    let mut vertical = TextObject::point(Point::new(300.0, 200.0), "Tate", style.clone());
    vertical.vertical = true;
    vertical.cached_bounds = Some(Rect::new(-10.0, 0.0, 10.0, 80.0));
    let vertical = add(vertical, None);
    // Point type drawn 100 to the right of where it is laid out.
    let mut moved = TextObject::point(Point::new(10.0, 350.0), "Moved", style);
    moved.cached_bounds = Some(Rect::new(0.0, -16.0, 60.0, 4.0));
    let moved = add(moved, Some(Affine::translate((100.0, 0.0))));
    let path_only = HitOptions { type_path_only: true, ..Default::default() };
    let hit = |p: Point| hit_test(&d, p, path_only).map(|h| h.leaf);
    assert_eq!(hit(Point::new(40.0, 100.0)), Some(point), "the first line's baseline");
    assert_eq!(hit(Point::new(40.0, 125.0)), Some(point), "the second line's");
    assert_eq!(hit(Point::new(40.0, 112.0)), None, "between the lines");
    assert_eq!(hit(Point::new(65.0, 124.0)), None, "past the end of the shorter second line");
    assert_eq!(hit(Point::new(220.0, 66.0)), Some(area), "area type: a line's baseline");
    assert_eq!(hit(Point::new(220.0, 80.0)), None, "area type: inside the frame, off the baselines");
    assert_eq!(hit(Point::new(299.0, 260.0)), Some(vertical), "vertical type: down the column");
    assert_eq!(hit(Point::new(306.0, 260.0)), None, "vertical type: beside it");
    assert_eq!(hit(Point::new(140.0, 351.0)), Some(moved), "in perspective: where it is drawn");
    assert_eq!(hit(Point::new(40.0, 351.0)), None, "in perspective: not where it is laid out");
}

/// `objects_at`: every object under the point, topmost first; a group counts once.
#[test]
fn objects_at_lists_the_stack() {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let rect = |d: &mut Document, x: f64| Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 0.0, x + 50.0, 50.0)), Appearance::default_art());
    let a = rect(&mut d, 0.0);
    let (b, c) = (rect(&mut d, 10.0), rect(&mut d, 20.0));
    let (a_id, b_id, c_id, g) = (a.id, b.id, c.id, d.alloc_id());
    d.insert(Some(l), 0, a).unwrap();
    d.insert(Some(l), 9, Node::group(g, vec![Arc::new(b), Arc::new(c)])).unwrap();
    let far = rect(&mut d, 120.0);
    d.insert(Some(l), 9, far).unwrap();
    let at = |x: f64, scope| objects_at(&d, Point::new(x, 25.0), HitOptions::default(), scope);
    assert_eq!(at(30.0, None), vec![g, a_id]);
    assert_eq!(at(30.0, Some(g)), [c_id, b_id], "isolated: the group's own objects");
    // Isolated, the art around the group is locked: clicks pass through it (#833).
    assert!(at(5.0, Some(g)).is_empty());
    assert!(at(130.0, Some(g)).is_empty());
    assert!(hit_test(&d, Point::new(5.0, 25.0), HitOptions { scope: Some(g), ..HitOptions::default() }).is_none());
    assert_eq!(at(5.0, None), vec![a_id]);
    assert!(at(90.0, None).is_empty());
}

/// Many overlapping paths: Select Behind must list each exactly once, front to back.
/// This is quadratic with repeated full-document hit tests, but linear in tree size with
/// the per-object walk.
#[test]
fn objects_at_large_overlapping_stack() {
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    let mut ids = Vec::new();
    for _ in 0..256 {
        let id = d.alloc_id();
        d.insert(Some(layer), usize::MAX, Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art())).unwrap();
        ids.push(id);
    }
    ids.reverse();
    assert_eq!(objects_at(&d, Point::new(25.0, 25.0), HitOptions::default(), None), ids);
}

/// Match the original skip-and-retest algorithm for normal groups, sublayers, clipping,
/// invisibility, and path/outline-only settings.
#[test]
fn objects_at_one_pass_matches_repeated_hits() {
    let mut d = Document::new(200.0, 200.0);
    let layer = d.layers[0].id;
    let add = |d: &mut Document, parent, bounds| {
        let id = d.alloc_id();
        d.insert(Some(parent), usize::MAX, Node::path(id, shapes::rectangle(bounds), Appearance::default_art())).unwrap();
        id
    };
    add(&mut d, layer, Rect::new(0.0, 0.0, 60.0, 60.0));
    let sub = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::layer(sub, "Sublayer", crate::LayerColor::Preset(2))).unwrap();
    add(&mut d, sub, Rect::new(0.0, 0.0, 60.0, 60.0));
    let group = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::group(group, Vec::new())).unwrap();
    add(&mut d, group, Rect::new(0.0, 0.0, 60.0, 60.0));
    add(&mut d, group, Rect::new(5.0, 5.0, 60.0, 60.0));
    let clip = d.alloc_id();
    let mask = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(5.0, 5.0, 45.0, 45.0)), Appearance::default_art());
    let content = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 80.0, 80.0)), Appearance::default_art());
    let mut clipping = Node::group(clip, vec![Arc::new(mask), Arc::new(content)]);
    clipping.set_clips(true);
    d.insert(Some(layer), usize::MAX, clipping).unwrap();
    for (point, opt) in [
        (Point::new(25.0, 25.0), HitOptions::default()),
        (Point::new(3.0, 25.0), HitOptions::default()),
        (Point::new(25.0, 25.0), HitOptions { outline: true, ..Default::default() }),
        (Point::new(6.0, 25.0), HitOptions { path_only: true, ..Default::default() }),
        (Point::new(100.0, 100.0), HitOptions::default()),
    ] {
        assert_eq!(objects_at(&d, point, opt, None), objects_at_repeated(&d, point, opt, None), "point={point:?}, opt={opt:?}");
    }
    if let NodeKind::Layer { preview, .. } = &mut d.node_mut(sub).unwrap().kind {
        *preview = false;
    }
    let p = Point::new(25.0, 25.0);
    let opt = HitOptions::default();
    assert_eq!(objects_at(&d, p, opt, None), objects_at_repeated(&d, p, opt, None));
    d.node_mut(group).unwrap().locked = true;
    assert_eq!(objects_at(&d, p, opt, None), objects_at_repeated(&d, p, opt, None));
}

/// Documents built at random (nested groups, sublayers, clipping groups, compound paths, blends
/// of nothing, hidden and locked objects, outline layers): Select Behind lists the same objects
/// in the same order as the hit test repeated over what is left.
#[test]
fn objects_at_matches_repeated_hits_on_random_documents() {
    let mut seed = 0x9e37_79b9_u32;
    let mut next = move |n: u32| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed % n
    };
    fn fill(d: &mut Document, parent: NodeId, depth: u32, next: &mut impl FnMut(u32) -> u32) {
        for _ in 0..2 + next(4) {
            let id = d.alloc_id();
            let (x, y) = (f64::from(next(60)), f64::from(next(60)));
            let rect = Rect::new(x, y, x + 10.0 + f64::from(next(50)), y + 10.0 + f64::from(next(50)));
            let appearance = if next(4) == 0 { Appearance::default() } else { Appearance::default_art() };
            let mut n = match next(if depth < 3 { 8 } else { 4 }) {
                4 | 5 => Node::group(id, Vec::new()),
                6 => Node::layer(id, "Sublayer", crate::LayerColor::Preset(2)),
                7 => {
                    let mut g = Node::group(id, Vec::new());
                    g.set_clips(true);
                    g
                }
                _ => Node::path(id, shapes::rectangle(rect), appearance),
            };
            n.visible = next(8) != 0;
            n.locked = next(8) == 0;
            if let NodeKind::Layer { preview, .. } = &mut n.kind {
                *preview = next(3) != 0;
            }
            let container = n.is_container();
            // A sublayer belongs in a layer; in a group it is a group like another to the hit test.
            d.insert(Some(parent), usize::MAX, n).unwrap();
            if container {
                fill(d, id, depth + 1, next);
            }
        }
    }
    for round in 0..200 {
        let mut d = Document::new(120.0, 120.0);
        let layer = d.layers[0].id;
        fill(&mut d, layer, 0, &mut next);
        // A clipping mask on the layer, or on a sublayer, now and then.
        let sublayers: Vec<NodeId> = d.layers[0].children().into_iter().flatten().filter(|n| n.is_layer()).map(|n| n.id).collect();
        for id in std::iter::once(layer).chain(sublayers) {
            if next(3) == 0 {
                d.node_mut(id).unwrap().set_clips(true);
            }
        }
        for _ in 0..12 {
            let p = Point::new(f64::from(next(120)), f64::from(next(120)));
            let opt = HitOptions { outline: next(5) == 0, path_only: next(5) == 0, clip_outlines: next(2) == 0, ..Default::default() };
            assert_eq!(objects_at(&d, p, opt, None), objects_at_repeated(&d, p, opt, None), "round {round}, {p:?}, {opt:?}");
            assert_eq!(leaf_chains(leaves_at(&d, p, opt)), leaf_chains(leaves_at_repeated(&d, p, opt)), "leaves: round {round}, {p:?}, {opt:?}");
            // Isolated in a group of the document, now and then.
            let groups: Vec<NodeId> =
                d.layers[0].children().into_iter().flatten().filter(|n| n.is_container() && !n.is_layer()).map(|n| n.id).collect();
            if let Some(&g) = groups.first().filter(|_| next(3) == 0) {
                let opt = HitOptions { scope: Some(g), ..opt };
                assert_eq!(leaf_chains(leaves_at(&d, p, opt)), leaf_chains(leaves_at_repeated(&d, p, opt)), "isolated: round {round}, {p:?}");
            }
        }
    }
}

/// A hit's leaf, ancestry, kind, layer count and edited envelope.
type LeafChain = (NodeId, Vec<NodeId>, HitKind, usize, Option<NodeId>);

/// Each hit's [`LeafChain`], to compare two lists.
fn leaf_chains(hits: Vec<Hit>) -> Vec<LeafChain> {
    hits.into_iter().map(|h| (h.leaf, h.ancestry, h.kind, h.layers, h.contents_of)).collect()
}

/// Select Behind steps through the members of groups one by one, front to back, in one traversal
/// for a large stack, as the hit test repeated over what is left finds them.
#[test]
fn leaves_at_lists_group_members_one_by_one() {
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    let square = |d: &mut Document| Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art());
    let below = square(&mut d);
    let below_id = below.id;
    d.insert(Some(layer), usize::MAX, below).unwrap();
    let members: Vec<Node> = (0..300).map(|_| square(&mut d)).collect();
    let mut ids: Vec<NodeId> = members.iter().map(|n| n.id).collect();
    let g = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::group(g, members.into_iter().map(Arc::new).collect())).unwrap();
    ids.reverse();
    ids.push(below_id);
    let p = Point::new(25.0, 25.0);
    let hits = leaves_at(&d, p, HitOptions::default());
    assert_eq!(hits.len(), MAX_LEAVES_AT, "the list stops at its limit");
    assert_eq!(hits.iter().map(|h| h.leaf).collect::<Vec<_>>(), ids[..MAX_LEAVES_AT], "the group's members, topmost first");
    assert!(hits.iter().all(|h| h.ancestry.contains(&g)));
    assert_eq!(leaf_chains(hits), leaf_chains(leaves_at_repeated(&d, p, HitOptions::default())));
    // Hidden past the limit, the object below the group comes after the members that are left.
    for id in &ids[3..300] {
        d.node_mut(*id).unwrap().visible = false;
    }
    let shown: Vec<NodeId> = leaves_at(&d, p, HitOptions::default()).iter().map(|h| h.leaf).collect();
    assert_eq!(shown, [ids[0], ids[1], ids[2], below_id]);
    let isolated = leaves_at(&d, p, HitOptions { scope: Some(g), ..Default::default() });
    assert!(!isolated.iter().any(|h| h.leaf == below_id), "isolated: only the group's members");
}

/// [`objects_at_repeated_after`] with each found object looked up in the ordered list itself: the
/// reference the tests below hold the set lookup to, and the base that `select_behind_isolation_timing` times.
fn objects_at_repeated_after_by_list(doc: &Document, p: Point, opt: HitOptions, scope: Option<NodeId>, mut tops: Vec<NodeId>) -> Vec<NodeId> {
    while let Some(top) = hit_test_skipping(doc, p, opt, &|id| tops.contains(&id), true).map(|h| h.top_object(scope)) {
        if tops.contains(&top) {
            break;
        }
        tops.push(top);
    }
    tops
}

/// A layer holding a group of `count` overlapping 50 × 50 squares, member `i` at x = 5 × (`i` mod
/// 10), with a square at x = 0 below the group and above it, and one at x = 100 above it: the
/// document, the group and its members, topmost first.
fn isolated_stack(count: u32) -> (Document, NodeId, Vec<NodeId>) {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let square = |d: &mut Document, x: f64| Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 0.0, x + 50.0, 50.0)), Appearance::default_art());
    let below = square(&mut d, 0.0);
    d.insert(Some(l), usize::MAX, below).unwrap();
    let members: Vec<Node> = (0..count).map(|i| square(&mut d, f64::from(i % 10) * 5.0)).collect();
    let mut ids: Vec<NodeId> = members.iter().map(|n| n.id).collect();
    let g = d.alloc_id();
    d.insert(Some(l), usize::MAX, Node::group(g, members.into_iter().map(Arc::new).collect())).unwrap();
    for x in [0.0, 100.0] {
        let above = square(&mut d, x);
        d.insert(Some(l), usize::MAX, above).unwrap();
    }
    ids.reverse();
    (d, g, ids)
}

/// Select Behind in an isolated group of 40 overlapping squares, with art around the group above
/// and below them: each member once, front to back, over 41 rounds of hit tests, and nothing where
/// only the art around the group lies.
#[test]
fn select_behind_in_isolation_lists_a_large_stack_once() {
    let (d, g, ids) = isolated_stack(40);
    let at = |x: f64, scope| objects_at(&d, Point::new(x, 25.0), HitOptions::default(), scope);
    assert_eq!(at(47.5, Some(g)), ids);
    assert_eq!(at(47.5, Some(g)).iter().collect::<HashSet<_>>().len(), 40, "no object twice");
    assert_eq!(at(47.5, None).len(), 3, "not isolated: the square above, the group and the square below");
    // At x = 17.5 only the members at x = 0 to 20 are hit (the one at 20 on its stroke, within
    // the tolerance): `ids[k]` is member 39 - k.
    let left: Vec<NodeId> = ids.iter().enumerate().filter(|(k, _)| (39 - k) % 10 <= 4).map(|(_, id)| *id).collect();
    assert_eq!(left.len(), 20);
    assert_eq!(at(17.5, Some(g)), left);
    assert!(at(125.0, Some(g)).is_empty(), "only the square around the group");
    assert_eq!(at(125.0, None).len(), 1);
    let isolated = HitOptions { scope: Some(g), ..HitOptions::default() };
    let p = Point::new(47.5, 25.0);
    assert_eq!(objects_at_repeated_after(&d, p, isolated, Some(g), ids[..15].to_vec()), ids, "the objects passed in stay first");
    let (d, g, clip, art) = masked_square(false);
    let isolated = HitOptions { scope: Some(g), ..SELECTING };
    for p in [Point::new(140.0, 90.0), Point::new(170.0, 90.0), Point::new(100.0, 90.0)] {
        let want = objects_at_repeated_after_by_list(&d, p, isolated, Some(g), vec![]);
        assert_eq!(objects_at(&d, p, SELECTING, Some(g)), want, "clip group isolated, {p:?}");
    }
    assert_eq!(objects_at(&d, Point::new(170.0, 90.0), SELECTING, Some(g)), vec![art], "the art its mask hides");
    assert!(objects_at(&d, Point::new(140.0, 90.0), SELECTING, Some(g)).contains(&clip), "the mask on its outline");
}

/// Documents built at random (nested groups, clipping groups, envelopes in Edit Contents, hidden
/// and locked objects), each object in turn isolated, and each list started with some of the
/// objects already found: the set lookup returns what the ordered-list lookup returns, field for
/// field.
#[test]
fn select_behind_in_isolation_matches_the_list_lookup_on_random_documents() {
    let mut seed = 0x2545_f491_u32;
    let mut next = move |n: u32| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed % n
    };
    fn fill(d: &mut Document, parent: NodeId, depth: u32, next: &mut impl FnMut(u32) -> u32) {
        for _ in 0..2 + next(5) {
            let id = d.alloc_id();
            let rect = |next: &mut dyn FnMut(u32) -> u32| {
                let (x, y) = (f64::from(next(60)), f64::from(next(60)));
                Rect::new(x, y, x + 10.0 + f64::from(next(50)), y + 10.0 + f64::from(next(50)))
            };
            let mut n = match next(if depth < 3 { 9 } else { 4 }) {
                4 | 5 => Node::group(id, Vec::new()),
                6 => {
                    let mut g = Node::group(id, Vec::new());
                    g.set_clips(true);
                    g
                }
                7 => {
                    let content =
                        (0..2).map(|_| Arc::new(Node::path(d.alloc_id(), shapes::rectangle(rect(next)), Appearance::default_art()))).collect();
                    Node::new(
                        id,
                        NodeKind::Envelope {
                            content,
                            kind: Box::new(crate::live::EnvelopeKind::Warp {
                                style: "arc".to_string(),
                                bend: 0.0,
                                h: 0.0,
                                v: 0.0,
                                horizontal: true,
                                version: 0,
                            }),
                            fidelity: 100.0,
                            editing: next(2) == 0,
                            options: Default::default(),
                            frame: Box::new(vectorcraft_geom::Affine::IDENTITY),
                        },
                    )
                }
                _ => Node::path(id, shapes::rectangle(rect(next)), if next(4) == 0 { Appearance::default() } else { Appearance::default_art() }),
            };
            n.visible = next(8) != 0;
            n.locked = next(8) == 0;
            let group = matches!(n.kind, NodeKind::Group { .. });
            d.insert(Some(parent), usize::MAX, n).unwrap();
            if group {
                fill(d, id, depth + 1, next);
            }
        }
    }
    let mut compared = 0;
    for round in 0..60 {
        let mut d = Document::new(120.0, 120.0);
        let layer = d.layers[0].id;
        fill(&mut d, layer, 0, &mut next);
        let mut scopes = vec![None];
        d.walk(|n| {
            if n.id != layer {
                scopes.push(Some(n.id));
            }
        });
        for _ in 0..6 {
            let p = Point::new(f64::from(next(120)), f64::from(next(120)));
            let opt = HitOptions { outline: next(5) == 0, path_only: next(5) == 0, clip_outlines: next(2) == 0, ..Default::default() };
            for &scope in &scopes {
                let opt = HitOptions { scope, ..opt };
                let want = objects_at_repeated_after_by_list(&d, p, opt, scope, vec![]);
                let context = format!("round {round}, {p:?}, {opt:?}");
                assert_eq!(objects_at_repeated_after(&d, p, opt, scope, vec![]), want, "{context}");
                if scope.is_some() {
                    assert_eq!(objects_at(&d, p, opt, scope), want, "{context}");
                }
                for found in 1..want.len() {
                    let start = want[..found].to_vec();
                    assert_eq!(objects_at_repeated_after(&d, p, opt, scope, start.clone()), want, "{context}, {found} found");
                    assert_eq!(objects_at_repeated_after_by_list(&d, p, opt, scope, start), want, "{context}, {found} found");
                }
                compared += usize::from(!want.is_empty());
            }
        }
    }
    assert!(compared > 500, "only {compared} non-empty lists compared");
}

/// Times Select Behind in an isolated group of 200 and of 1,000 overlapping squares, the set lookup
/// against the ordered-list lookup, alternating the two in one process:
/// `cargo test --release -p vectorcraft-doc --lib -- --ignored --nocapture select_behind_isolation_timing`.
#[test]
#[ignore = "a timing, not a check"]
fn select_behind_isolation_timing() {
    /// This thread's CPU time in nanoseconds on Linux; the elapsed time since `start` elsewhere.
    fn clock(start: std::time::Instant) -> u128 {
        let cpu = std::fs::read_to_string("/proc/thread-self/schedstat").ok();
        cpu.and_then(|s| s.split_whitespace().next()?.parse().ok()).unwrap_or_else(|| start.elapsed().as_nanos())
    }
    type Lookup = fn(&Document, Point, HitOptions, Option<NodeId>, Vec<NodeId>) -> Vec<NodeId>;
    let start = std::time::Instant::now();
    println!("clock: {}", if std::fs::metadata("/proc/thread-self/schedstat").is_ok() { "thread CPU time" } else { "wall clock" });
    let p = Point::new(47.5, 25.0);
    for count in [200, 1_000] {
        let (d, g, ids) = isolated_stack(count);
        let opt = HitOptions { scope: Some(g), ..HitOptions::default() };
        assert_eq!(objects_at_repeated_after(&d, p, opt, Some(g), vec![]), ids);
        assert_eq!(objects_at_repeated_after_by_list(&d, p, opt, Some(g), vec![]), ids);
        // The time per call of `f`: batches of calls double until one takes at least 100 ms.
        let per_call = |f: Lookup| {
            let mut calls = 1u32;
            loop {
                let t = clock(start);
                for _ in 0..calls {
                    std::hint::black_box(f(&d, std::hint::black_box(p), opt, Some(g), vec![]));
                }
                let spent = clock(start) - t;
                if spent >= 100_000_000 || calls >= 1 << 20 {
                    return (spent as f64 / f64::from(calls), calls);
                }
                calls *= 2;
            }
        };
        let (mut old, mut new) = (vec![], vec![]);
        for _ in 0..7 {
            old.push(per_call(objects_at_repeated_after_by_list));
            new.push(per_call(objects_at_repeated_after));
        }
        old.sort_by(|a, b| a.0.total_cmp(&b.0));
        new.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (o, n) = (old[3], new[3]);
        println!(
            "{count} objects: list lookup {:.2} ms per call ({} calls), set lookup {:.2} ms per call ({} calls), median of 7 runs",
            o.0 / 1e6,
            o.1,
            n.0 / 1e6,
            n.1
        );
    }
}

/// Select Behind finds a symbol instance by its symbol's art, as a click does: art from
/// (100, 60) to (140, 80), outside the 20 × 20 square around the origin, drawn where it lies by
/// one instance and 100 pt lower by an instance in a group.
#[test]
fn objects_at_finds_symbol_instances_by_their_art() {
    let mut d = Document::new(200.0, 200.0);
    let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(100.0, 60.0, 140.0, 80.0)), Appearance::default_art());
    d.symbols.push(crate::Symbol { dynamic: false, name: "Far".into(), art: Arc::new(art) });
    let (layer, top, inner, group) = (d.layers[0].id, d.alloc_id(), d.alloc_id(), d.alloc_id());
    let instance = |id, dy: f64| Node::new(id, NodeKind::SymbolInstance { symbol: "Far".into(), xf: vectorcraft_geom::Affine::translate((0.0, dy)) });
    d.insert(Some(layer), 0, instance(top, 0.0)).unwrap();
    d.insert(Some(layer), 1, Node::group(group, vec![Arc::new(instance(inner, 100.0))])).unwrap();
    let opt = HitOptions::default();
    for (x, y, want) in [(120.0, 70.0, vec![top]), (120.0, 170.0, vec![group]), (0.0, 0.0, vec![]), (0.0, 100.0, vec![])] {
        let p = Point::new(x, y);
        assert_eq!(objects_at(&d, p, opt, None), want, "{p:?}");
        assert_eq!(objects_at_repeated(&d, p, opt, None), want, "{p:?}");
    }
}

/// Edit Contents can expose descendants of a top-level group: preserve the original
/// selection order by falling back to the general hit traversal.
#[test]
fn objects_at_edit_contents_uses_general_order() {
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    let inner = |id| Arc::new(Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art()));
    let (a, b) = (d.alloc_id(), d.alloc_id());
    let e = Node::new(
        d.alloc_id(),
        NodeKind::Envelope {
            content: vec![inner(a), inner(b)],
            kind: Box::new(crate::live::EnvelopeKind::Warp { style: "arc".to_string(), bend: 0.0, h: 0.0, v: 0.0, horizontal: true, version: 0 }),
            fidelity: 100.0,
            editing: true,
            options: Default::default(),
            frame: Box::new(vectorcraft_geom::Affine::IDENTITY),
        },
    );
    d.insert(Some(layer), usize::MAX, e).unwrap();
    let p = Point::new(25.0, 25.0);
    let opt = HitOptions::default();
    assert_eq!(objects_at(&d, p, opt, None), objects_at_repeated(&d, p, opt, None));
    assert_eq!(objects_at(&d, p, opt, None), vec![b, a]);
}

/// Select Behind keeps one limit of symbol art to walk when it falls back to the general
/// traversal, for an envelope in Edit Contents and for a root that is not a layer: the fallback
/// walks no more art than what the first pass left, and keeps what the first pass found.
#[test]
fn select_behind_keeps_one_art_limit_when_it_falls_back() {
    // 60 instances of `Clipped` with the rectangle hidden, which a click at (150, 50) walks the art
    // of and misses: each walk takes 3 objects of the 100.
    let stack = |d: &mut Document, count: usize| {
        let l = d.layers[0].id;
        for _ in 0..count {
            let inst = instance_of(d, "Clipped");
            d.insert(Some(l), usize::MAX, Arc::unwrap_or_clone(inst)).unwrap();
        }
    };
    // An envelope in Edit Contents at the bottom of the layer, its contents under (150, 50).
    let envelope = |d: &mut Document| {
        let inner = |id| Arc::new(Node::path(id, shapes::rectangle(Rect::new(120.0, 20.0, 180.0, 80.0)), Appearance::default_art()));
        let (a, b) = (d.alloc_id(), d.alloc_id());
        let e = Node::new(
            d.alloc_id(),
            NodeKind::Envelope {
                content: vec![inner(a), inner(b)],
                kind: Box::new(crate::live::EnvelopeKind::Warp { style: "arc".to_string(), bend: 0.0, h: 0.0, v: 0.0, horizontal: true, version: 0 }),
                fidelity: 100.0,
                editing: true,
                options: Default::default(),
                frame: Box::new(vectorcraft_geom::Affine::IDENTITY),
            },
        );
        let l = d.layers[0].id;
        d.insert(Some(l), 0, e).unwrap();
        (a, b)
    };
    let p = Point::new(150.0, 50.0);
    let mut d = Document::new(600.0, 400.0);
    clipped_symbol(&mut d, false);
    let (a, b) = envelope(&mut d);
    stack(&mut d, 60);
    let (behind, _, arts) = with_art_limit(100, || objects_at(&d, p, HitOptions::default(), None));
    assert_eq!(behind, vec![b, a]);
    assert!(arts <= 34, "envelope in Edit Contents: {arts} instances' art walked");
    // An instance that the first pass hits by its art, the limit then used up, stays in the list:
    // `Shown` is `Clipped` with its rectangle shown, below 32 instances of `Clipped`.
    let mut d = Document::new(600.0, 400.0);
    clipped_symbol(&mut d, false);
    let (a, b) = envelope(&mut d);
    let (mut mask, art) = half_clipped(&mut d);
    mask.appearance.effects.push(test_move());
    let mut clip = Node::group(d.alloc_id(), vec![Arc::new(mask), Arc::new(art)]);
    clip.set_clips(true);
    group_symbol(&mut d, "Shown", vec![Arc::new(clip)]);
    let hit = instance_of(&mut d, "Shown");
    let hit_id = hit.id;
    let l = d.layers[0].id;
    d.insert(Some(l), usize::MAX, Arc::unwrap_or_clone(hit)).unwrap();
    stack(&mut d, 32);
    let (behind, _, arts) = with_art_limit(100, || objects_at(&d, p, HitOptions::default(), None));
    assert_eq!(behind, vec![hit_id, b, a], "hit before the fallback");
    assert!(arts <= 34, "hit before the fallback: {arts} instances' art walked");
    // A root that is not a layer, below the layer.
    let mut d = Document::new(600.0, 400.0);
    clipped_symbol(&mut d, false);
    stack(&mut d, 60);
    let root = Arc::new(Node::group(d.alloc_id(), vec![]));
    d.layers.insert(0, root);
    let (behind, _, arts) = with_art_limit(100, || objects_at(&d, p, HitOptions::default(), None));
    assert!(behind.is_empty());
    assert!(arts <= 34, "root that is not a layer: {arts} instances' art walked");
}

/// [`leaves_at`] falls back to repeated hit tests at a clip group, after the hits its single
/// traversal found above it: an instance hit by its art, with the limit of symbol art then used
/// up, stays in the list.
#[test]
fn leaves_at_keeps_the_hits_above_a_clip_group_it_falls_back_at() {
    let p = Point::new(150.0, 50.0);
    let mut d = Document::new(600.0, 400.0);
    clipped_symbol(&mut d, false);
    // A clip group at the bottom of the layer, its art under (150, 50).
    let square = |id| Arc::new(Node::path(id, shapes::rectangle(Rect::new(120.0, 20.0, 180.0, 80.0)), Appearance::default_art()));
    let (mask, art) = (d.alloc_id(), d.alloc_id());
    let mut clip_group = Node::group(d.alloc_id(), vec![square(mask), square(art)]);
    clip_group.set_clips(true);
    let l = d.layers[0].id;
    d.insert(Some(l), 0, clip_group).unwrap();
    let (mut shown_mask, shown_art) = half_clipped(&mut d);
    shown_mask.appearance.effects.push(test_move());
    let mut shown = Node::group(d.alloc_id(), vec![Arc::new(shown_mask), Arc::new(shown_art)]);
    shown.set_clips(true);
    group_symbol(&mut d, "Shown", vec![Arc::new(shown)]);
    let hit = instance_of(&mut d, "Shown");
    let hit_id = hit.id;
    d.insert(Some(l), usize::MAX, Arc::unwrap_or_clone(hit)).unwrap();
    for _ in 0..32 {
        let inst = instance_of(&mut d, "Clipped");
        d.insert(Some(l), usize::MAX, Arc::unwrap_or_clone(inst)).unwrap();
    }
    let (hits, _, arts) = with_art_limit(100, || leaves_at(&d, p, HitOptions::default()));
    let leaves: Vec<NodeId> = hits.iter().map(|h| h.leaf).collect();
    assert_eq!(leaves.first(), Some(&hit_id), "hit before the fallback: {leaves:?}");
    assert!(leaves.contains(&art), "the clipped art after it: {leaves:?}");
    assert!(arts <= 34, "{arts} instances' art walked");
}

#[test]
fn locked_and_hidden_skip() {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let a = d.alloc_id();
    d.insert(Some(l), 0, Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art())).unwrap();
    d.node_mut(a).unwrap().locked = true;
    assert!(hit_test(&d, Point::new(10.0, 10.0), HitOptions::default()).is_none());
    d.node_mut(a).unwrap().locked = false;
    d.node_mut(l).unwrap().visible = false;
    assert!(hit_test(&d, Point::new(10.0, 10.0), HitOptions::default()).is_none());
}

/// Insert `n` locked at the top of `parent`.
fn insert_locked(d: &mut Document, parent: NodeId, mut n: Node) -> NodeId {
    let id = n.id;
    n.locked = true;
    d.insert(Some(parent), usize::MAX, n).unwrap();
    id
}

/// `locked_objects_at`: the locked objects under the point, topmost first. A locked group is
/// listed rather than its contents, and a locked object in an unlocked group is listed itself.
/// Hidden objects, locked layers and the art outside an isolated group are left out.
#[test]
fn locked_objects_at_lists_the_locked_stack() {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let rect = |d: &mut Document, x: f64| Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 0.0, x + 50.0, 50.0)), Appearance::default_art());
    let (a, b) = (rect(&mut d, 0.0), rect(&mut d, 10.0));
    let (a, b) = (insert_locked(&mut d, l, a), insert_locked(&mut d, l, b));
    let (mut inner, locked_group) = (rect(&mut d, 20.0), d.alloc_id());
    inner.locked = true;
    let inner_id = inner.id;
    let mut g = Node::group(locked_group, vec![Arc::new(inner)]);
    g.locked = true;
    d.insert(Some(l), usize::MAX, g).unwrap();
    let open_group = d.alloc_id();
    let mut child = rect(&mut d, 25.0);
    child.locked = true;
    let child_id = child.id;
    let free = rect(&mut d, 25.0);
    d.insert(Some(l), usize::MAX, Node::group(open_group, vec![Arc::new(child), Arc::new(free)])).unwrap();
    let at = |d: &Document, x: f64, scope| locked_objects_at(d, Point::new(x, 25.0), HitOptions { scope, ..HitOptions::default() });
    assert_eq!(at(&d, 30.0, None), [child_id, locked_group, b, a]);
    assert!(!at(&d, 30.0, None).contains(&inner_id), "the locked group stands for what it holds");
    assert_eq!(at(&d, 5.0, None), [a]);
    assert!(at(&d, 150.0, None).is_empty());
    // Isolated, only the group's own objects are looked at.
    assert_eq!(at(&d, 30.0, Some(open_group)), [child_id]);
    // A hidden object is left out, and nothing on a locked or hidden layer is listed.
    d.node_mut(b).unwrap().visible = false;
    assert_eq!(at(&d, 30.0, None), [child_id, locked_group, a]);
    d.node_mut(l).unwrap().locked = true;
    assert!(at(&d, 30.0, None).is_empty());
    d.node_mut(l).unwrap().locked = false;
    d.node_mut(l).unwrap().visible = false;
    assert!(at(&d, 30.0, None).is_empty());
}

/// `locked_objects_at` lists at most `MAX_LOCKED_AT` objects, the topmost ones.
#[test]
fn locked_objects_at_stops_at_its_limit() {
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    let ids: Vec<NodeId> = (0..MAX_LOCKED_AT + 5)
        .map(|_| {
            let n = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art());
            insert_locked(&mut d, l, n)
        })
        .collect();
    let found = locked_objects_at(&d, Point::new(25.0, 25.0), HitOptions::default());
    assert_eq!(found, ids.iter().rev().take(MAX_LOCKED_AT).copied().collect::<Vec<_>>());
}

#[test]
fn visible_sampling_reads_locked_art_and_layers_but_excludes_hidden_art() {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let a = d.alloc_id();
    d.insert(Some(l), 0, Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default_art())).unwrap();
    let p = Point::new(10.0, 10.0);
    for lock in [a, l] {
        d.node_mut(lock).unwrap().locked = true;
        assert!(hit_test(&d, p, HitOptions::default()).is_none());
        assert_eq!(hit_test_visible(&d, p, HitOptions::default()).unwrap().leaf, a);
        assert!(d.node(lock).unwrap().locked);
        d.node_mut(lock).unwrap().locked = false;
    }
    d.node_mut(a).unwrap().visible = false;
    assert!(hit_test_visible(&d, p, HitOptions::default()).is_none());
    d.node_mut(a).unwrap().visible = true;
    d.node_mut(l).unwrap().visible = false;
    assert!(hit_test_visible(&d, p, HitOptions::default()).is_none());
}

/// Deterministic xorshift in 0..1.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % 1_000_000) as f64 / 1_000_000.0
    }
}

/// A closed blob of `n` smooth anchors round `c` (radius about `r`), turning either way, with
/// handles long enough that its control points reach past its curves.
fn blob(rng: &mut Rng, c: Point, r: f64, n: usize, clockwise: bool) -> vectorcraft_geom::SubPath {
    use vectorcraft_geom::{Anchor, Vec2};
    let anchors = (0..n)
        .map(|i| {
            let k = if clockwise { i as f64 } else { (n - i) as f64 };
            let a = k / n as f64 * std::f64::consts::TAU;
            let rr = r * (0.6 + 0.8 * rng.next());
            let p = c + Vec2::new(a.cos(), a.sin()) * rr;
            let t = Vec2::new(-a.sin(), a.cos()) * (if clockwise { 1.0 } else { -1.0 }) * rr * (0.2 + 1.3 * rng.next());
            Anchor::smooth(p, p + t)
        })
        .collect();
    vectorcraft_geom::SubPath::new(anchors, true)
}

/// A document with one compound path of `members` blobs (some of them holes), its id and the
/// members' paths.
fn compound_doc(members: usize, rule: vectorcraft_geom::FillRule, appearance: Appearance) -> (Document, NodeId, Vec<vectorcraft_geom::PathData>) {
    let mut d = Document::new(800.0, 800.0);
    let l = d.layers[0].id;
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut paths = vec![];
    let mut children = vec![];
    for i in 0..members {
        let c = Point::new(20.0 + rng.next() * 600.0, 20.0 + rng.next() * 600.0);
        let r = 3.0 + rng.next() * 9.0;
        let pd = vectorcraft_geom::PathData::single(blob(&mut rng, c, r, 3 + i % 4, i % 3 != 0));
        children.push(Arc::new(Node::path(d.alloc_id(), pd.clone(), Appearance::default())));
        paths.push(pd);
    }
    let id = d.alloc_id();
    let mut compound = Node::new(id, NodeKind::Compound { children, rule });
    compound.appearance = appearance.into();
    d.insert(Some(l), 0, compound).unwrap();
    (d, id, paths)
}

/// What a click on compound path `n` gave before compound paths were culled by their control
/// points and their members near the click: the whole path assembled (`full`), culled by its
/// reach measured along the curves.
fn compound_hit_before(n: &Node, full: &vectorcraft_geom::BezPath, p: Point, opt: HitOptions) -> Option<HitKind> {
    let NodeKind::Compound { rule, .. } = &n.kind else { return None };
    if !n.reach_bounds().is_none_or(|b| b.inflate(opt.tol, opt.tol).contains(p)) {
        return None;
    }
    if !opt.outline && n.appearance.stroke_hit(full, *rule, p, opt.tol) {
        return Some(HitKind::Stroke);
    }
    if stroke_contains(full, 0.0, opt.tol, p) {
        return Some(HitKind::Outline);
    }
    (!opt.outline && !opt.path_only && fill_contains(full, *rule, p)).then_some(HitKind::Fill)
}

/// The points a test clicks for `paths`: random ones over them and past them, and, for some
/// members, their anchors, points just over the tolerance from them, and the corners and edge
/// midpoints of their control and curve boxes, on them and grown by the tolerance.
fn probe_points(paths: &[vectorcraft_geom::PathData], tol: f64, random: usize, every: usize) -> Vec<Point> {
    let all = paths.iter().filter_map(vectorcraft_geom::PathData::control_bounds).reduce(|a, b| a.union(b)).unwrap().inflate(30.0, 30.0);
    let mut rng = Rng(7);
    let mut pts: Vec<Point> = (0..random).map(|_| Point::new(all.x0 + rng.next() * all.width(), all.y0 + rng.next() * all.height())).collect();
    for pd in paths.iter().step_by(every) {
        for a in pd.subpaths.iter().flat_map(|sp| &sp.anchors) {
            pts.extend([a.p, Point::new(a.p.x + tol, a.p.y), Point::new(a.p.x, a.p.y - tol), Point::new(a.p.x + tol * 1.0001, a.p.y)]);
        }
        for b in [pd.control_bounds().unwrap(), pd.bounds().unwrap()] {
            for b in [b, b.inflate(tol, tol)] {
                let (cx, cy) = (b.center().x, b.center().y);
                pts.extend(
                    [(b.x0, b.y0), (b.x1, b.y0), (b.x1, b.y1), (b.x0, b.y1), (cx, b.y0), (cx, b.y1), (b.x0, cy), (b.x1, cy)]
                        .map(|(x, y)| Point::new(x, y)),
                );
            }
        }
    }
    pts
}

/// A compound path of thousands of members hits exactly as it did when it was assembled
/// whole for every click: at random points, on and next to anchors, and on the edges of the
/// members' boxes (control points' and curves', with and without the tolerance), for both fill
/// rules, in outline mode and with Object Selection by Path Only.
#[test]
fn culled_compound_hits_as_the_whole_path() {
    use vectorcraft_geom::FillRule;
    for (rule, opt) in [
        (FillRule::NonZero, HitOptions::default()),
        (FillRule::EvenOdd, HitOptions { tol: 0.5, ..Default::default() }),
        (FillRule::NonZero, HitOptions { outline: true, tol: 1.0, ..Default::default() }),
        (FillRule::EvenOdd, HitOptions { path_only: true, ..Default::default() }),
    ] {
        let (d, id, paths) = compound_doc(2000, rule, Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
        let n = d.node(id).unwrap();
        let full = n.stroke_path().unwrap();
        // The whole path's box grown by the tolerance. Before, a point on its far edges (right
        // and bottom: `Rect::contains` leaves them out) never hit, even exactly `tol` from the
        // outline; now it hits there as on the near edges. Those points are clicked too, and
        // checked against the distance alone.
        let far = vectorcraft_geom::Shape::bounding_box(&full).inflate(opt.tol, opt.tol);
        let mut pts = probe_points(&paths, opt.tol, 200, 120);
        let rightmost = paths.iter().filter_map(vectorcraft_geom::PathData::bounds).max_by(|a, b| a.x1.total_cmp(&b.x1)).unwrap();
        pts.push(Point::new(far.x1, rightmost.center().y));
        let (mut hits, mut kinds) = (0, HashSet::new());
        for p in pts {
            let before = compound_hit_before(n, &full, p, opt);
            let now = hit_test(&d, p, opt);
            if p.x == far.x1 || p.y == far.y1 {
                let outline = vectorcraft_geom::hit::distance_to_outline(&full, p) <= opt.tol;
                assert_eq!(now.is_some(), outline || before.is_some(), "{rule:?} {opt:?} on the far edge at {p:?}");
                continue;
            }
            assert_eq!(now.as_ref().map(|h| (h.leaf, h.kind)), before.map(|k| (id, k)), "{rule:?} {opt:?} at {p:?}");
            hits += usize::from(before.is_some());
            kinds.extend(before.map(|k| format!("{k:?}")));
        }
        // The points reach both hits and misses, and the outline as well as the fill.
        assert!(hits > 100, "{rule:?} {opt:?}: {hits} hits");
        assert!(kinds.contains("Outline"), "{rule:?} {opt:?}: {kinds:?}");
    }
}

/// A wide stroke paints far past a path's control points: clicks on it there still hit, for a
/// path and for a compound path (whose strokes are measured along all of it), exactly as when
/// they were culled by their reach measured along the curves.
#[test]
fn wide_strokes_hit_past_the_control_points() {
    use vectorcraft_geom::{Anchor, PathData, SubPath};
    let mut d = Document::new(400.0, 400.0);
    let l = d.layers[0].id;
    let stroke = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 40.0);
    // A zigzag 10 pt high whose 40 pt stroke reaches 20 pt (more at its mitred turns) past it.
    let zig = PathData::single(SubPath::new(
        [(100.0, 100.0), (110.0, 110.0), (120.0, 100.0), (130.0, 110.0)].map(|(x, y)| Anchor::corner(Point::new(x, y))).to_vec(),
        false,
    ));
    let path = d.alloc_id();
    d.insert(Some(l), 0, Node::path(path, zig.clone(), stroke.clone())).unwrap();
    // A compound path of two small rings with the same stroke, further along.
    let mut rng = Rng(3);
    let members: Vec<Arc<Node>> = [Point::new(250.0, 250.0), Point::new(262.0, 250.0)]
        .into_iter()
        .map(|c| Arc::new(Node::path(d.alloc_id(), PathData::single(blob(&mut rng, c, 4.0, 4, true)), Appearance::default())))
        .collect();
    let compound = d.alloc_id();
    let mut node = Node::new(compound, NodeKind::Compound { children: members, rule: vectorcraft_geom::FillRule::NonZero });
    node.appearance = stroke.into();
    d.insert(Some(l), 9, node).unwrap();
    let symbols = d.symbol_boxes();
    let opt = HitOptions::default();
    for id in [path, compound] {
        let n = d.node(id).unwrap();
        let full = n.stroke_path().unwrap();
        let control = match &n.kind {
            NodeKind::Path { path, .. } => path.control_bounds().unwrap(),
            _ => n.children().unwrap().iter().filter_map(|m| m.path_data()?.control_bounds()).reduce(|a, b| a.union(b)).unwrap(),
        };
        let (mut outside, mut checked) = (0, 0);
        let area = control.inflate(40.0, 40.0);
        for i in 0..=80 {
            for j in 0..=80 {
                let p = Point::new(area.x0 + area.width() * f64::from(i) / 80.0, area.y0 + area.height() * f64::from(j) / 80.0);
                let before = match &n.kind {
                    NodeKind::Path { .. } => n
                        .reach_bounds_with(&symbols)
                        .is_none_or(|b| b.inflate(opt.tol, opt.tol).contains(p))
                        .then(|| hit_leaf(n, p, opt, &symbols))
                        .flatten(),
                    _ => compound_hit_before(n, &full, p, opt),
                };
                let now = hit_test(&d, p, opt).filter(|h| h.leaf == id).map(|h| h.kind);
                assert_eq!(now, before, "{id:?} at {p:?}");
                checked += 1;
                outside += usize::from(before == Some(HitKind::Stroke) && !control.contains(p));
            }
        }
        assert!(outside > 50, "{id:?}: {outside} of {checked} points hit the stroke outside the control points' box");
    }
}

/// The box a click is culled by holds the object's reach for paths and compound paths (it is
/// measured by their control points, which hold their curves, and grown by the same stroke reach),
/// so culling by it never turns away a click the reach let through. (Up to rounding: where a curve's
/// extreme is an anchor, the two boxes measure the same point in different ways.)
#[test]
fn cull_box_holds_the_reach_of_paths_and_compound_paths() {
    const E: f64 = 1e-9;
    let holds =
        |outer: Rect, inner: Rect| outer.x0 - E <= inner.x0 && outer.y0 - E <= inner.y0 && inner.x1 <= outer.x1 + E && inner.y1 <= outer.y1 + E;
    let stroke = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 12.0);
    let (mut d, id, _) = compound_doc(200, vectorcraft_geom::FillRule::NonZero, stroke.clone());
    let path_id = d.alloc_id();
    let symbols = d.symbol_boxes();
    let n = d.node(id).unwrap();
    let mut checked = 0;
    for node in std::iter::once(n).chain(n.children().unwrap().iter().map(|m| &**m)) {
        let (cull, reach) = (cull_reach(node, &symbols).unwrap(), node.reach_bounds_with(&symbols).unwrap());
        assert!(holds(cull, reach), "{:?}: {cull:?} does not hold {reach:?}", node.id);
        checked += 1;
    }
    // A stroked plain path, its handles reaching past its curves.
    let mut rng = Rng(11);
    let path = Node::path(path_id, vectorcraft_geom::PathData::single(blob(&mut rng, Point::new(50.0, 50.0), 20.0, 5, true)), stroke);
    let (cull, reach) = (cull_reach(&path, &symbols).unwrap(), path.reach_bounds_with(&symbols).unwrap());
    assert!(holds(cull, reach), "{cull:?} does not hold {reach:?}");
    assert_eq!(checked, 201);
}

/// In a dynamic symbol's art, a layer with a clipping mask clips its objects as a clip group does:
/// Direct Selection picks an object only where the mask shows it, and outlines it there.
#[test]
fn an_object_a_clipping_layer_in_symbol_art_hides_is_not_picked_there() {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let (mask, art) = half_clipped(&mut d);
    let mut layer = Node::layer(d.alloc_id(), "Masked", crate::LayerColor::Preset(2));
    if let NodeKind::Layer { children, .. } = &mut layer.kind {
        *children = vec![Arc::new(mask), Arc::new(art)];
    }
    layer.set_clips(true);
    let root = Node::group(d.alloc_id(), vec![Arc::new(layer)]);
    d.symbols.push(crate::Symbol { dynamic: true, name: "Clipped".into(), art: Arc::new(root) });
    let inst = d.alloc_id();
    d.insert(Some(l), 0, Node::new(inst, NodeKind::SymbolInstance { symbol: "Clipped".into(), xf: vectorcraft_geom::Affine::IDENTITY })).unwrap();
    let n = d.node(inst).unwrap();
    let opt = HitOptions::default();
    assert_eq!(symbol_part_at(&d, n, Point::new(50.0, 50.0), opt), Some(vec![0, 1]), "where the mask shows the art");
    assert_eq!(symbol_part_at(&d, n, Point::new(150.0, 50.0), opt), None, "where the mask hides it");
    let b = vectorcraft_geom::kurbo::Shape::bounding_box(&symbol_part_outline(&d, n, &[0, 1]).unwrap());
    assert!((b.x1 - 100.0).abs() < 1e-6, "the outline is cut to the mask: {b:?}");
    // Its outline inside the group around the layer is cut too.
    let b = vectorcraft_geom::kurbo::Shape::bounding_box(&symbol_part_outline(&d, n, &[0]).unwrap());
    assert!((b.x1 - 100.0).abs() < 1e-6, "the layer's outline is cut to the mask: {b:?}");
}

/// In a dynamic symbol's art, an object in a clipping layer whose clipping path has a live effect
/// is picked and outlined in the region the effect gives the clipping path. In Outline view, and
/// when Preview is off for the layer in the art or for the layer the instance is on, the plain
/// clipping path counts, as the canvas draws it. An effect that leaves no region leaves no outline.
#[test]
fn a_symbol_part_follows_the_region_its_clipping_layers_effects_give_it() {
    set_clip_region(moved_clip_region);
    // The instance of a symbol whose art is a clipping layer; `preview` is the Preview of that
    // layer and of the document layer the instance is on.
    let instance = |effect: &str, preview: [bool; 2]| {
        let mut d = Document::new(300.0, 200.0);
        if let NodeKind::Layer { preview: p, .. } = &mut Arc::make_mut(&mut d.layers[0]).kind {
            *p = preview[1];
        }
        let l = d.layers[0].id;
        let (mut mask, art) = half_clipped(&mut d);
        mask.appearance.effects.push(crate::Effect { id: effect.into(), ..test_move() });
        let mut layer = Node::layer(d.alloc_id(), "Masked", crate::LayerColor::Preset(2));
        if let NodeKind::Layer { children, preview: p, .. } = &mut layer.kind {
            (*children, *p) = (vec![Arc::new(mask), Arc::new(art)], preview[0]);
        }
        layer.set_clips(true);
        let root = Node::group(d.alloc_id(), vec![Arc::new(layer)]);
        d.symbols.push(crate::Symbol { dynamic: true, name: "Clipped".into(), art: Arc::new(root) });
        let inst = d.alloc_id();
        d.insert(Some(l), 0, Node::new(inst, NodeKind::SymbolInstance { symbol: "Clipped".into(), xf: vectorcraft_geom::Affine::IDENTITY })).unwrap();
        (d, inst)
    };
    let outline_box = |d: &Document, inst: NodeId, opt: HitOptions| {
        let o = symbol_part_outline_in_view(d, d.node(inst).unwrap(), &[0, 1], opt).unwrap();
        vectorcraft_geom::kurbo::Shape::bounding_box(&o)
    };
    let opt = HitOptions::default();
    let (d, inst) = instance("test.move", [true, true]);
    let n = d.node(inst).unwrap();
    assert_eq!(symbol_part_at(&d, n, Point::new(150.0, 50.0), opt), Some(vec![0, 1]), "in the moved region");
    assert_eq!(symbol_part_at(&d, n, Point::new(50.0, 50.0), opt), None, "where the clipping path was");
    let b = outline_box(&d, inst, opt);
    assert!((b.x0 - 100.0).abs() < 1e-6 && (b.x1 - 200.0).abs() < 1e-6, "the outline is cut to the moved region: {b:?}");
    let b = outline_box(&d, inst, HitOptions { outline: true, ..opt });
    assert!(b.x0.abs() < 1e-6 && (b.x1 - 100.0).abs() < 1e-6, "in Outline view, to the plain clipping path: {b:?}");
    // With Preview off, the art's top edge counts within the plain clipping path.
    for (preview, what) in [([false, true], "the layer in the art"), ([true, false], "the layer the instance is on")] {
        let (d, inst) = instance("test.move", preview);
        let n = d.node(inst).unwrap();
        assert_eq!(symbol_part_at(&d, n, Point::new(50.0, 0.5), opt), Some(vec![0, 1]), "on the edge the canvas draws, Preview off for {what}");
        assert_eq!(symbol_part_at(&d, n, Point::new(150.0, 0.5), opt), None, "past the plain clipping path, Preview off for {what}");
        let b = outline_box(&d, inst, opt);
        assert!(b.x0.abs() < 1e-6 && (b.x1 - 100.0).abs() < 1e-6, "Preview off for {what}: the plain clipping path: {b:?}");
    }
    // An effect that leaves the clipping path no region hides the art: no pick and no outline.
    let (d, inst) = instance("test.empty", [true, true]);
    let n = d.node(inst).unwrap();
    assert_eq!(symbol_part_at(&d, n, Point::new(50.0, 50.0), opt), None);
    assert!(symbol_part_outline(&d, n, &[0, 1]).unwrap().elements().is_empty(), "nothing is drawn, so nothing is outlined");
}

/// A symbol instance whose art holds a clip group or a clipping layer with a live effect that moves
/// its clipping path is clicked, and touched by a marquee, where the moved region shows the art past
/// the art's box: inside a group, nested in another symbol's art, moved and scaled. In Outline view,
/// below a layer whose Preview is off, where the art is hidden and where no region is left, only the
/// box counts.
#[test]
fn a_symbol_instance_is_hit_where_a_moved_clipping_path_in_its_art_shows_the_art() {
    use vectorcraft_geom::Affine;
    set_clip_region(moved_clip_region);
    // `effect` on the clipping path of a clip group (a clipping layer with `layer`), whose art is
    // shown when `shown`; `xf` places the instance, which with `nest` shows a symbol whose art holds
    // an instance of the clipped symbol. The instance is in a group: the group, the instance.
    let build = |effect: &str, xf: Affine, nest: bool, shown: bool, layer: bool| {
        let mut d = Document::new(600.0, 400.0);
        let l = d.layers[0].id;
        let (mut mask, mut art) = half_clipped(&mut d);
        mask.appearance.effects.push(crate::Effect { id: effect.into(), ..test_move() });
        art.visible = shown;
        let mut clip = if layer {
            let mut layer = Node::layer(d.alloc_id(), "Masked", crate::LayerColor::Preset(2));
            if let NodeKind::Layer { children, .. } = &mut layer.kind {
                *children = vec![Arc::new(mask), Arc::new(art)];
            }
            layer
        } else {
            Node::group(d.alloc_id(), vec![Arc::new(mask), Arc::new(art)])
        };
        clip.set_clips(true);
        let root = Node::group(d.alloc_id(), vec![Arc::new(clip)]);
        d.symbols.push(crate::Symbol { dynamic: false, name: "Clipped".into(), art: Arc::new(root) });
        let mut symbol = "Clipped";
        if nest {
            let inner = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Clipped".into(), xf: Affine::IDENTITY });
            let root = Node::group(d.alloc_id(), vec![Arc::new(inner)]);
            d.symbols.push(crate::Symbol { dynamic: false, name: "Outer".into(), art: Arc::new(root) });
            symbol = "Outer";
        }
        let (g, inst) = (d.alloc_id(), d.alloc_id());
        let instance = Node::new(inst, NodeKind::SymbolInstance { symbol: symbol.into(), xf });
        d.insert(Some(l), 0, Node::group(g, vec![Arc::new(instance)])).unwrap();
        (d, g, inst)
    };
    let opt = HitOptions::default();
    let outline = HitOptions { outline: true, ..opt };
    let at = |d: &Document, x: f64, y: f64, opt: HitOptions| hit_test(d, Point::new(x, y), opt).map(|h| h.leaf);
    // Across the art's top edge in the moved region, and past the art's right end.
    let (edge, past) = (Rect::new(140.0, -5.0, 160.0, 5.0), Rect::new(240.0, -5.0, 260.0, 5.0));
    for layer in [false, true] {
        let (d, g, inst) = build("test.move", Affine::IDENTITY, false, true, layer);
        assert_eq!(at(&d, 150.0, 50.0, opt), Some(inst), "in the moved region, layer {layer}");
        assert_eq!(hit_test(&d, Point::new(150.0, 50.0), opt).map(|h| h.top_object(None)), Some(g));
        assert_eq!(at(&d, 50.0, 50.0, opt), Some(inst), "in the box");
        assert_eq!(at(&d, 250.0, 50.0, opt), None, "past the art");
        assert_eq!(at(&d, 150.0, 150.0, opt), None, "below the art");
        assert_eq!(at(&d, 150.0, 50.0, outline), None, "Outline view draws the plain clipping path");
        assert_eq!(marquee(&d, edge, None, false), [g], "layer {layer}");
        assert_eq!(marquee(&d, edge, None, true), [inst], "layer {layer}");
        assert!(marquee(&d, past, None, false).is_empty());
        assert!(marquee_in_view(&d, edge, None, false, true, false).is_empty(), "Outline view");
    }
    // Preview off for the layer the instance is on.
    let (mut d, _, _) = build("test.move", Affine::IDENTITY, false, true, false);
    if let NodeKind::Layer { preview, .. } = &mut Arc::make_mut(&mut d.layers[0]).kind {
        *preview = false;
    }
    assert_eq!(at(&d, 150.0, 50.0, opt), None, "Preview off");
    assert!(marquee(&d, edge, None, false).is_empty(), "Preview off");
    // Moved, scaled (the region moves 100 pt from the placed clipping path) and nested.
    let (d, _, inst) = build("test.move", Affine::translate((10.0, 20.0)), false, true, false);
    assert_eq!(at(&d, 160.0, 70.0, opt), Some(inst), "moved");
    let (d, _, inst) = build("test.move", Affine::scale(2.0), false, true, false);
    assert_eq!(at(&d, 250.0, 100.0, opt), Some(inst), "scaled");
    assert_eq!(at(&d, 350.0, 100.0, opt), None, "scaled, past the region");
    let (d, g, inst) = build("test.move", Affine::IDENTITY, true, true, false);
    assert_eq!(at(&d, 150.0, 50.0, opt), Some(inst), "nested");
    assert_eq!(marquee(&d, edge, None, false), [g], "nested");
    // Hidden art, and an effect that leaves no region: clicks find only the box. The marquee still
    // touches the moved clipping path on the edge of its region, as in a clip group, unless no
    // region is left.
    for (effect, shown) in [("test.move", false), ("test.empty", true)] {
        let (d, g, inst) = build(effect, Affine::IDENTITY, false, shown, false);
        assert_eq!(at(&d, 150.0, 50.0, opt), None, "{effect}, shown {shown}");
        assert_eq!(at(&d, 50.0, 50.0, opt), Some(inst), "{effect}, shown {shown}: in the box");
        let touched: &[NodeId] = if shown { &[] } else { &[g] };
        assert_eq!(marquee(&d, edge, None, false), touched, "{effect}, shown {shown}");
    }
}

/// A click on the stroke of a symbol's art where it paints past the instance's box selects the
/// instance; past the stroke, and in Outline view, it does not.
#[test]
fn a_symbol_instance_is_hit_on_the_stroke_of_its_art_past_its_box() {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    // A 20 pt stroke reaches 10 pt past the 0..100 square.
    let look = Appearance::basic(Paint::solid(Color::BLACK), Paint::solid(Color::BLACK), 20.0);
    let square = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), look);
    let art = Node::group(d.alloc_id(), vec![Arc::new(square)]);
    d.symbols.push(crate::Symbol { dynamic: false, name: "Stroked".into(), art: Arc::new(art) });
    let inst = d.alloc_id();
    let xf = vectorcraft_geom::Affine::IDENTITY;
    d.insert(Some(l), 0, Node::new(inst, NodeKind::SymbolInstance { symbol: "Stroked".into(), xf })).unwrap();
    let at = |x: f64, opt: HitOptions| hit_test(&d, Point::new(x, 50.0), opt).map(|h| h.leaf);
    assert_eq!(at(108.0, HitOptions::default()), Some(inst), "on the stroke past the box");
    assert_eq!(at(50.0, HitOptions::default()), Some(inst), "in the box");
    assert_eq!(at(116.0, HitOptions::default()), None, "past the stroke");
    assert_eq!(at(108.0, HitOptions { outline: true, ..HitOptions::default() }), None, "Outline view");
}

/// A symbol's art on a layer of the symbol whose Preview is off is hit in outline, unless the
/// click sets [`HitOptions::ignore_layer_preview`]: then the stroke past the instance's box hits.
#[test]
fn ignore_layer_preview_reaches_symbol_art_on_a_preview_off_layer() {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    // A 20 pt stroke reaches 10 pt past the 0..100 square.
    let look = Appearance::basic(Paint::solid(Color::BLACK), Paint::solid(Color::BLACK), 20.0);
    let square = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), look);
    let mut layer = Node::layer(d.alloc_id(), "Art", crate::LayerColor::Preset(2));
    if let NodeKind::Layer { children, preview, .. } = &mut layer.kind {
        *children = vec![Arc::new(square)];
        *preview = false;
    }
    let art = Node::group(d.alloc_id(), vec![Arc::new(layer)]);
    d.symbols.push(crate::Symbol { dynamic: false, name: "Stroked".into(), art: Arc::new(art) });
    let inst = d.alloc_id();
    let xf = vectorcraft_geom::Affine::IDENTITY;
    d.insert(Some(l), 0, Node::new(inst, NodeKind::SymbolInstance { symbol: "Stroked".into(), xf })).unwrap();
    let at = |x: f64, opt: HitOptions| hit_test(&d, Point::new(x, 50.0), opt).map(|h| h.leaf);
    let painted = HitOptions { ignore_layer_preview: true, ..HitOptions::default() };
    assert_eq!(at(108.0, HitOptions::default()), None, "Preview off: in outline");
    assert_eq!(at(108.0, painted), Some(inst), "ignore_layer_preview: on the stroke past the box");
    assert_eq!(at(116.0, painted), None, "past the stroke");
}

/// Outside a symbol instance's box, an instance nested in its art counts where its own art is, not
/// anywhere in its box: a click or a marquee on empty space in the nested box, where a moved
/// clipping path shows the art past the outer box, selects nothing, and on art drawn there it
/// selects the outer instance.
#[test]
fn a_nested_symbol_instances_box_past_the_outer_box_is_not_a_hit() {
    set_clip_region(moved_clip_region);
    // `Two` draws squares at the corners of 0..200 × 0..100, and with `middle` one at 140..160 ×
    // 40..60. The outer symbol clips an instance of it to 0..100 × 0..100, which `test.move` moves
    // to 100..200: the outer box is 0..100, the nested box 0..200.
    let build = |middle: bool| {
        let mut d = Document::new(600.0, 400.0);
        let l = d.layers[0].id;
        let square = |d: &mut Document, r: Rect| Arc::new(Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::default_art()));
        let mut squares = vec![square(&mut d, Rect::new(0.0, 0.0, 10.0, 10.0)), square(&mut d, Rect::new(190.0, 90.0, 200.0, 100.0))];
        if middle {
            squares.push(square(&mut d, Rect::new(140.0, 40.0, 160.0, 60.0)));
        }
        group_symbol(&mut d, "Two", squares);
        let (mut mask, _) = half_clipped(&mut d);
        mask.appearance.effects.push(test_move());
        let nested = instance_of(&mut d, "Two");
        let mut clip = Node::group(d.alloc_id(), vec![Arc::new(mask), nested]);
        clip.set_clips(true);
        group_symbol(&mut d, "Outer", vec![Arc::new(clip)]);
        let inst = d.alloc_id();
        d.insert(Some(l), 0, Node::new(inst, NodeKind::SymbolInstance { symbol: "Outer".into(), xf: vectorcraft_geom::Affine::IDENTITY })).unwrap();
        (d, inst)
    };
    let at = |d: &Document, x: f64, y: f64| hit_test(d, Point::new(x, y), HitOptions::default()).map(|h| h.leaf);
    let inside = Rect::new(145.0, 45.0, 155.0, 55.0);
    let (d, inst) = build(false);
    assert_eq!(at(&d, 150.0, 50.0), None, "empty space in the nested box");
    assert!(marquee(&d, inside, None, false).is_empty(), "marquee on empty space in the nested box");
    assert_eq!(at(&d, 195.0, 95.0), Some(inst), "on the nested art past the outer box");
    assert_eq!(at(&d, 50.0, 50.0), Some(inst), "in the outer box");
    let (d, inst) = build(true);
    assert_eq!(at(&d, 150.0, 50.0), Some(inst), "on the nested art");
    // Across the middle square's left edge.
    assert_eq!(marquee(&d, Rect::new(135.0, 45.0, 145.0, 55.0), None, false), [inst], "marquee on the nested art");
}

/// Adds the symbol `Clipped`: a clip group whose clipping path's region `test.move` moves to
/// 100..200 × 0..100, past the 0..100 box, over a 0..200 × 0..100 rectangle shown when `shown`.
/// A click at (150, 50) hits an instance of it on the rectangle, or, with the rectangle hidden,
/// walks its art and finds nothing.
fn clipped_symbol(d: &mut Document, shown: bool) {
    set_clip_region(moved_clip_region);
    let (mut mask, mut art) = half_clipped(d);
    mask.appearance.effects.push(test_move());
    art.visible = shown;
    let mut clip = Node::group(d.alloc_id(), vec![Arc::new(mask), Arc::new(art)]);
    clip.set_clips(true);
    let root = Node::group(d.alloc_id(), vec![Arc::new(clip)]);
    d.symbols.push(crate::Symbol { dynamic: false, name: "Clipped".into(), art: Arc::new(root) });
}

/// A symbol named `name` whose art is a group of `members`.
fn group_symbol(d: &mut Document, name: &str, members: Vec<Arc<Node>>) {
    let root = Node::group(d.alloc_id(), members);
    d.symbols.push(crate::Symbol { dynamic: false, name: name.into(), art: Arc::new(root) });
}

/// An instance of the symbol named `symbol`, not moved.
fn instance_of(d: &mut Document, symbol: &str) -> Arc<Node> {
    Arc::new(Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: symbol.into(), xf: vectorcraft_geom::Affine::IDENTITY }))
}

/// 30 small squares at the origin, which a click at (150, 50) passes by.
fn far_squares(d: &mut Document) -> Vec<Arc<Node>> {
    (0..30).map(|_| Arc::new(Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), Appearance::default_art()))).collect()
}

/// Runs `f` with [`ArtWalk`] allowing `limit` objects, and returns its result with how many
/// leaves [`hit_leaf`] tested and how many instances' art was walked meanwhile.
fn with_art_limit<R>(limit: u64, f: impl FnOnce() -> R) -> (R, usize, usize) {
    ART_WALK_LIMIT.with(|c| c.set(Some(limit)));
    LEAVES_TESTED.with(|c| c.set(0));
    ARTS_WALKED.with(|c| c.set(0));
    let r = f();
    ART_WALK_LIMIT.with(|c| c.set(None));
    (r, LEAVES_TESTED.with(std::cell::Cell::get), ARTS_WALKED.with(std::cell::Cell::get))
}

/// One click visits at most the art walk's limit of objects in a chain of nested symbol
/// instances, though each level's art alone is well within it. A click on the art of an
/// instance nested in another is still found.
#[test]
fn a_click_walks_a_chain_of_nested_symbol_instances_within_one_limit() {
    // Level 0 is `Clipped`; each level above holds an instance of the level below and 30 squares:
    // 32 objects to visit at each level.
    let chain = |shown: bool| {
        let mut d = Document::new(600.0, 400.0);
        let l = d.layers[0].id;
        clipped_symbol(&mut d, shown);
        for level in 1..12 {
            let below = if level == 1 { "Clipped".to_string() } else { format!("Level {}", level - 1) };
            let mut members = vec![instance_of(&mut d, &below)];
            members.extend(far_squares(&mut d));
            group_symbol(&mut d, &format!("Level {level}"), members);
        }
        let inst = instance_of(&mut d, "Level 11");
        let id = inst.id;
        d.insert(Some(l), 0, Arc::unwrap_or_clone(inst)).unwrap();
        (d, id)
    };
    let at = |d: &Document, x: f64| hit_test(d, Point::new(x, 50.0), HitOptions::default()).map(|h| h.leaf);
    // Without one limit for the click, the miss walks the art of all 12 levels.
    let (d, inst) = chain(false);
    let (hit, _, arts) = with_art_limit(100, || at(&d, 150.0));
    assert_eq!(hit, None);
    assert!(arts <= 4, "{arts} levels walked");
    assert_eq!(with_art_limit(100, || at(&d, 50.0)).0, Some(inst), "in the box");
    // Past the limit, a click that the art would take finds only the box.
    let (d, inst) = chain(true);
    assert_eq!(with_art_limit(100, || at(&d, 150.0)).0, None, "past the limit");
    assert_eq!(at(&d, 150.0), Some(inst), "within the limit");
    // Two levels: a click on the inner instance's art is found within the limit.
    let mut d = Document::new(600.0, 400.0);
    let l = d.layers[0].id;
    clipped_symbol(&mut d, true);
    let inner = instance_of(&mut d, "Clipped");
    group_symbol(&mut d, "Outer", vec![inner]);
    let inst = instance_of(&mut d, "Outer");
    let id = inst.id;
    d.insert(Some(l), 0, Arc::unwrap_or_clone(inst)).unwrap();
    assert_eq!(with_art_limit(100, || at(&d, 150.0)).0, Some(id), "nested");
}

/// One click, and one marquee, visit at most the art walk's limit of objects in art that repeats
/// one nested instance many times, though the art of each instance alone is well within it. A
/// click on one of the nested instances' art is still found.
#[test]
fn a_click_and_a_marquee_walk_repeated_nested_symbol_instances_within_one_limit() {
    // 50 instances of `Clipped` in one symbol's art, its rectangle shown when `shown`.
    let repeated = |shown: bool| {
        let mut d = Document::new(600.0, 400.0);
        let l = d.layers[0].id;
        clipped_symbol(&mut d, shown);
        let members = (0..50).map(|_| instance_of(&mut d, "Clipped")).collect();
        group_symbol(&mut d, "Repeated", members);
        let inst = instance_of(&mut d, "Repeated");
        let id = inst.id;
        d.insert(Some(l), 0, Arc::unwrap_or_clone(inst)).unwrap();
        (d, id)
    };
    let (d, inst) = repeated(false);
    // Without one limit for the click, the miss walks the art of all 51 instances. The outer art
    // takes 1 object of the 100, and each nested instance with its art 4.
    let (hit, _, arts) = with_art_limit(100, || hit_test(&d, Point::new(150.0, 50.0), HitOptions::default()).map(|h| h.leaf));
    assert_eq!(hit, None);
    assert!(arts <= 26, "{arts} instances' art walked");
    // Select Behind over 60 instances of `Clipped` on the layer shares one limit too: each walk
    // takes 3 objects of the 100.
    let mut stack = Document::new(600.0, 400.0);
    let l = stack.layers[0].id;
    clipped_symbol(&mut stack, false);
    for _ in 0..60 {
        let inst = instance_of(&mut stack, "Clipped");
        stack.insert(Some(l), 0, Arc::unwrap_or_clone(inst)).unwrap();
    }
    let (behind, _, arts) = with_art_limit(100, || objects_at(&stack, Point::new(150.0, 50.0), HitOptions::default(), None));
    assert!(behind.is_empty());
    assert!(arts <= 34, "Select Behind: {arts} instances' art walked");
    // Without one limit for the marquee, it places the art of all 51 instances, inside the moved
    // region (within each instance's reach and outside its box). Placing the outer art takes 51
    // objects of the 100, and each nested art 4.
    let inside = Rect::new(140.0, 40.0, 160.0, 60.0);
    let (touched, _, arts) = with_art_limit(100, || marquee(&d, inside, None, false));
    assert!(touched.is_empty());
    assert!(arts <= 14, "{arts} instances' art placed");
    assert_eq!(with_art_limit(100, || marquee(&d, Rect::new(40.0, 40.0, 60.0, 60.0), None, false)).0, [inst], "in the box");
    // A click and a marquee on the nested instances' art are found within the limit.
    let (d, inst) = repeated(true);
    assert_eq!(with_art_limit(100, || hit_test(&d, Point::new(150.0, 50.0), HitOptions::default()).map(|h| h.leaf)).0, Some(inst), "click");
    let edge = Rect::new(140.0, -5.0, 160.0, 5.0);
    assert_eq!(with_art_limit(100, || marquee(&d, edge, None, false)).0, [inst], "marquee across the art's edge");
}

/// The members of an Intertwine group in symbol art count toward the art walk's limit of objects
/// for a click, as the members of a plain group do. A click on a member's path past the instance's
/// box is still found within the limit.
#[test]
fn a_click_counts_the_members_of_an_intertwine_group_in_symbol_art_toward_one_limit() {
    set_clip_region(moved_clip_region);
    // A clip group whose clipping region `test.move` moves to 100..200 × 0..100, past the 0..100
    // box, over an Intertwine group of 200 squares at 140..160 × 40..60.
    let mut d = Document::new(600.0, 400.0);
    let l = d.layers[0].id;
    let (mut mask, _) = half_clipped(&mut d);
    mask.appearance.effects.push(test_move());
    let squares = (0..200)
        .map(|_| Arc::new(Node::path(d.alloc_id(), shapes::rectangle(Rect::new(140.0, 40.0, 160.0, 60.0)), Appearance::default_art())))
        .collect();
    let woven = Arc::new(Node::new(d.alloc_id(), NodeKind::Intertwine { children: squares }));
    let mut clip = Node::group(d.alloc_id(), vec![Arc::new(mask), woven]);
    clip.set_clips(true);
    group_symbol(&mut d, "Woven", vec![Arc::new(clip)]);
    let inst = instance_of(&mut d, "Woven");
    let id = inst.id;
    d.insert(Some(l), 0, Arc::unwrap_or_clone(inst)).unwrap();
    let opt = HitOptions { path_only: true, ..HitOptions::default() };
    let at = |x: f64| hit_test(&d, Point::new(x, 50.0), opt).map(|h| h.leaf);
    // Inside the squares and away from their paths, every member is tested and none is hit.
    let (hit, leaves, _) = with_art_limit(100, || at(150.0));
    assert_eq!(hit, None);
    assert!(leaves <= 100, "{leaves} leaves tested");
    assert_eq!(with_art_limit(100, || at(140.0)).0, Some(id), "on the squares' paths");
}

/// Whether a symbol instance's fill or stroke effects reshape its art, for these tests: when
/// [`painted_far_away`] gives it shapes.
fn reshapes_when_painted(n: &Node) -> bool {
    !painted_far_away(None, n).is_empty()
}

/// An instance of the symbol named `symbol`, not moved, whose fill has a geometry effect:
/// [`reshapes_when_painted`] holds for it when the name starts with "Painted", so a part pick and a
/// part outline walk its art as drawn with its fill's effects ([`InstanceArt::drawn`]).
fn painted_instance_of(d: &mut Document, symbol: &str) -> Arc<Node> {
    let mut n = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: symbol.into(), xf: vectorcraft_geom::Affine::IDENTITY });
    n.appearance = Appearance::default_art().into();
    let offset = crate::Effect { id: "path.offsetPath".into(), params: serde_json::json!({"offset": 10}), visible: true };
    n.appearance.items[0].effects_mut().push(offset);
    Arc::new(n)
}

/// Five levels of symbols: "Painted 0" is a 1 pt square, and each level above holds 8 instances
/// of the level below whose fills have a geometry effect. Returns the id of an instance of
/// "Painted 4" on the layer.
fn painted_chain(d: &mut Document) -> NodeId {
    let l = d.layers[0].id;
    let dot = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), Appearance::default_art());
    d.symbols.push(crate::Symbol { dynamic: false, name: "Painted 0".into(), art: Arc::new(dot) });
    for level in 1..5 {
        let members = (0..8).map(|_| painted_instance_of(d, &format!("Painted {}", level - 1))).collect();
        group_symbol(d, &format!("Painted {level}"), members);
    }
    let inst = instance_of(d, "Painted 4");
    let id = inst.id;
    d.insert(Some(l), 0, Arc::unwrap_or_clone(inst)).unwrap();
    id
}

/// The outline of a part of a dynamic symbol's instance visits at most the art walk's limit of
/// objects, though the part is an instance whose art, drawn with its fill's effects, nests
/// instances 3 levels deep, 8 at each level. Within the limit every square is outlined.
#[test]
fn a_parts_outline_walks_nested_reshaped_symbol_instances_within_one_limit() {
    set_image_paint(painted_far_away);
    set_instance_reshapes(reshapes_when_painted);
    let mut d = Document::new(600.0, 400.0);
    let id = painted_chain(&mut d);
    let n = d.node(id).unwrap();
    let squares = |o: &BezPath| o.elements().iter().filter(|e| matches!(e, vectorcraft_geom::kurbo::PathEl::MoveTo(_))).count();
    // Part [0] is an instance of "Painted 3": 8 × 8 × 8 squares.
    assert_eq!(squares(&symbol_part_outline(&d, n, &[0]).unwrap()), 512, "within the limit");
    let (outline, _, arts) = with_art_limit(100, || symbol_part_outline(&d, n, &[0]).unwrap());
    assert!(squares(&outline) < 100, "{} squares outlined", squares(&outline));
    assert!(arts < 100, "{arts} instances' art walked");
}

/// A part pick in a dynamic symbol's instance visits at most the art walk's limit of objects of
/// nested instances whose art, drawn with their fills' effects, is walked: in a chain of nested
/// instances, and in instances whose clip groups the click misses, where only the clip group
/// itself is tested. A click on the clipped art of a nested instance is still found.
#[test]
fn a_part_pick_walks_nested_reshaped_symbol_instances_within_one_limit() {
    set_image_paint(painted_far_away);
    set_instance_reshapes(reshapes_when_painted);
    let mut d = Document::new(600.0, 400.0);
    let id = painted_chain(&mut d);
    let opt = HitOptions::default();
    let (hit, _, arts) = with_art_limit(100, || symbol_part_at(&d, d.node(id).unwrap(), Point::new(150.0, 50.0), opt));
    assert_eq!(hit, None);
    assert!(arts <= 40, "chain: {arts} instances' art walked");
    // 30 instances of a clip group whose 10 pt clipping path holds 40 squares: 42 objects each.
    let mut d = Document::new(600.0, 400.0);
    let l = d.layers[0].id;
    let mask = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default());
    let mut members = vec![Arc::new(mask)];
    members.extend((0..40).map(|_| Arc::new(Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 5.0, 5.0)), Appearance::default_art()))));
    let mut clip = Node::group(d.alloc_id(), members);
    clip.set_clips(true);
    d.symbols.push(crate::Symbol { dynamic: false, name: "Painted clip".into(), art: Arc::new(clip) });
    let members = (0..30).map(|_| painted_instance_of(&mut d, "Painted clip")).collect();
    group_symbol(&mut d, "Clips", members);
    let inst = instance_of(&mut d, "Clips");
    let id = inst.id;
    d.insert(Some(l), 0, Arc::unwrap_or_clone(inst)).unwrap();
    let n = d.node(id).unwrap();
    let (hit, _, arts) = with_art_limit(300, || symbol_part_at(&d, n, Point::new(150.0, 50.0), opt));
    assert_eq!(hit, None);
    assert!(arts <= 8, "clip groups: {arts} instances' art walked");
    assert_eq!(with_art_limit(300, || symbol_part_at(&d, n, Point::new(2.0, 2.0), opt)).0, Some(vec![29]), "the clipped art");
}

/// The outline of a part of a dynamic symbol's instance leaves out the hidden objects in the art of
/// an instance nested in the part whose fill's effects reshape it, as a part pick passes over them.
#[test]
fn a_parts_outline_leaves_out_hidden_art_of_nested_reshaped_symbol_instances() {
    use vectorcraft_geom::kurbo::Shape;
    set_image_paint(painted_far_away);
    set_instance_reshapes(reshapes_when_painted);
    let mut d = Document::new(600.0, 400.0);
    let l = d.layers[0].id;
    let shown = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), Appearance::default_art());
    let mut hidden = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(50.0, 50.0, 60.0, 60.0)), Appearance::default_art());
    hidden.visible = false;
    group_symbol(&mut d, "Painted hidden", vec![Arc::new(shown), Arc::new(hidden)]);
    let member = painted_instance_of(&mut d, "Painted hidden");
    group_symbol(&mut d, "Hides", vec![member]);
    let inst = instance_of(&mut d, "Hides");
    let id = inst.id;
    d.insert(Some(l), 0, Arc::unwrap_or_clone(inst)).unwrap();
    let n = d.node(id).unwrap();
    assert_eq!(symbol_part_at(&d, n, Point::new(55.0, 55.0), HitOptions::default()), None, "the hidden square is not picked");
    let outline = symbol_part_outline(&d, n, &[0]).unwrap();
    let squares = outline.elements().iter().filter(|e| matches!(e, vectorcraft_geom::kurbo::PathEl::MoveTo(_))).count();
    assert_eq!(squares, 1, "only the shown square is outlined");
    assert!(outline.bounding_box().x1 <= 1.0, "the outline reaches {:?}", outline.bounding_box());
}

/// In a dynamic symbol's art that placing through its 9-slice guides could make more than
/// [`crate::MAX_SYMBOL_NODES`] objects of, Direct Selection picks and outlines nothing, as the
/// canvas draws nothing of it. At the art's natural size the same art is picked.
#[test]
fn symbol_art_past_the_object_limit_picks_nothing() {
    use vectorcraft_color::{Gradient, GradientPaint};
    let mut d = Document::new(400.0, 400.0);
    let l = d.layers[0].id;
    let leaf = Arc::new(Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), Appearance::default_art()));
    let inner = Arc::new(Node::group(d.alloc_id(), vec![leaf; 1_800]));
    let mut mask = crate::OpacityMask::new(Node::group(d.alloc_id(), vec![inner; 1_800]), true);
    mask.linked = false;
    let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
    // Drawn in six cells, each with its own copy of the mask's 3,240,000 squares.
    let mut big = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 50.0)), Appearance::basic(gradient, Paint::None, 0.0));
    big.extras_mut().mask = Some(Box::new(mask));
    let plain = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 50.0, 100.0, 100.0)), Appearance::default_art());
    let root = Node::group(d.alloc_id(), vec![Arc::new(plain), Arc::new(big)]);
    d.symbols.push(crate::Symbol { dynamic: true, name: "Big".into(), art: Arc::new(root) });
    d.set_symbol_nine_slice("Big", Some(crate::SymbolNineSlice::default()));
    let scaled = d.alloc_id();
    let natural = d.alloc_id();
    let xf = vectorcraft_geom::Affine::scale_non_uniform(2.0, 1.5);
    d.insert(Some(l), 0, Node::new(scaled, NodeKind::SymbolInstance { symbol: "Big".into(), xf })).unwrap();
    d.insert(Some(l), 0, Node::new(natural, NodeKind::SymbolInstance { symbol: "Big".into(), xf: vectorcraft_geom::Affine::IDENTITY })).unwrap();
    assert!(d.place_symbol_art("Big", &d.symbols[0].art, xf).is_none(), "the art is past the limit");
    let opt = HitOptions::default();
    let n = d.node(scaled).unwrap();
    assert_eq!(symbol_part_at(&d, n, Point::new(100.0, 110.0), opt), None, "the plain square");
    assert_eq!(symbol_part_at(&d, n, Point::new(100.0, 40.0), opt), None, "the masked path");
    assert!(symbol_part_outline(&d, n, &[0]).is_none());
    assert!(symbol_part_outline(&d, n, &[1]).is_none());
    let n = d.node(natural).unwrap();
    assert_eq!(symbol_part_at(&d, n, Point::new(50.0, 75.0), opt), Some(vec![0]));
    assert!(symbol_part_outline(&d, n, &[0]).is_some());
}

/// Picking and outlining an object of a 9-slice instance's art, and placing that art, read the
/// symbol list once for each call, however many instances the art holds.
#[test]
fn picking_in_9_slice_art_reads_the_symbol_list_once() {
    let mut d = Document::new(400.0, 400.0);
    for i in 0..2_000 {
        let dot = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), Appearance::default_art());
        d.symbols.push(crate::Symbol { dynamic: false, name: format!("Dot {i}"), art: Arc::new(dot) });
    }
    // A frame, a group of 20 squares in its top left cell, and 20 instances of the dots across
    // the guides.
    let square = |d: &mut Document, x: f64, y: f64| {
        Arc::new(Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, y, x + 4.0, y + 4.0)), Appearance::default_art()))
    };
    let corner: Vec<_> = (0..20).map(|i| square(&mut d, f64::from(i % 5) * 4.0, f64::from(i / 5) * 4.0)).collect();
    let dot = |d: &mut Document, i: u32| {
        let xf = vectorcraft_geom::Affine::translate((f64::from(i) * 4.5 + 5.0, 48.0)) * vectorcraft_geom::Affine::scale(4.0);
        Arc::new(Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: format!("Dot {i}"), xf }))
    };
    let across: Vec<_> = (0..20).map(|i| dot(&mut d, i)).collect();
    let frame = Arc::new(Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default_art()));
    let mut kids = vec![frame, Arc::new(Node::group(d.alloc_id(), corner))];
    kids.extend(across);
    let root = Node::group(d.alloc_id(), kids);
    d.symbols.push(crate::Symbol { dynamic: true, name: "Panel".into(), art: Arc::new(root) });
    d.set_symbol_nine_slice("Panel", Some(crate::SymbolNineSlice::default()));
    let xf = vectorcraft_geom::Affine::scale_non_uniform(2.0, 1.5);
    let inst = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Panel".into(), xf });
    let art = d.symbols[2_000].art.clone();
    let symbols = d.symbols.len();
    let read = || crate::symbols::SYMBOLS_READ.with(std::cell::Cell::get);
    let reads = |f: &dyn Fn()| {
        let before = read();
        f();
        read() - before
    };
    // Picking also tests the placed art with boxes of its own, as a click outside symbols does.
    let n = reads(&|| assert_eq!(symbol_part_at(&d, &inst, Point::new(2.0, 2.0), HitOptions::default()).map(|p| p[0]), Some(1)));
    assert!(n <= 2 * symbols, "picking read {n} symbols");
    let n = reads(&|| assert!(symbol_part_outline(&d, &inst, &[2]).is_some()));
    assert!(n <= symbols, "outlining read {n} symbols");
    let n = reads(&|| assert!(d.place_symbol_art("Panel", &art, xf).is_some()));
    assert!(n <= symbols, "placing read {n} symbols");
    let n = reads(&|| assert!(d.place_symbol_art_unclipped("Panel", &art, xf).is_some()));
    assert!(n <= symbols, "placing without clips read {n} symbols");
}

/// A clip group that lies in one 9-slice cell is moved by that cell as one piece, everything in it
/// included, so an object in it is picked and outlined there even when placing that object through
/// the guides on its own would pass [`crate::MAX_SYMBOL_NODES`] objects.
#[test]
fn objects_of_a_clip_group_in_one_cell_are_placed_with_it() {
    use vectorcraft_color::{Gradient, GradientPaint};
    let mut d = Document::new(400.0, 400.0);
    let l = d.layers[0].id;
    let leaf = Arc::new(Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), Appearance::default_art()));
    let inner = Arc::new(Node::group(d.alloc_id(), vec![leaf; 1_800]));
    let mut mask = crate::OpacityMask::new(Node::group(d.alloc_id(), vec![inner; 1_800]), true);
    mask.linked = false;
    let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
    let mut big = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(gradient, Paint::None, 0.0));
    big.extras_mut().mask = Some(Box::new(mask));
    let clip = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::default_art());
    let mut group = Node::group(d.alloc_id(), vec![Arc::new(clip), Arc::new(big.clone())]);
    group.set_clips(true);
    let frame = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default_art());
    let root = Node::group(d.alloc_id(), vec![Arc::new(frame), Arc::new(group)]);
    d.symbols.push(crate::Symbol { dynamic: true, name: "Corner".into(), art: Arc::new(root) });
    d.set_symbol_nine_slice("Corner", Some(crate::SymbolNineSlice::default()));
    let inst = d.alloc_id();
    let xf = vectorcraft_geom::Affine::scale_non_uniform(2.0, 1.5);
    d.insert(Some(l), 0, Node::new(inst, NodeKind::SymbolInstance { symbol: "Corner".into(), xf })).unwrap();
    let m = d.symbol_slice_map("Corner", xf).unwrap();
    assert!(m.most_placed(&big, &d) > crate::MAX_SYMBOL_NODES, "the object alone is past the limit");
    assert!(d.place_symbol_art("Corner", &d.symbols[0].art, xf).is_some(), "the art is drawn");
    let n = d.node(inst).unwrap();
    assert_eq!(symbol_part_at(&d, n, Point::new(10.0, 10.0), HitOptions::default()), Some(vec![1, 1]));
    let b = vectorcraft_geom::kurbo::Shape::bounding_box(&symbol_part_outline(&d, n, &[1, 1]).unwrap());
    assert!((b.x1 - 20.0).abs() < 1e-6 && (b.y1 - 20.0).abs() < 1e-6, "cut to the clip in its corner: {b:?}");
}

/// In a dynamic symbol's art, Direct Selection does not pick a Shaper group's original shapes,
/// which the canvas does not draw: a click over an original shape whose face the group erased
/// picks nothing, and a click over the face it shows picks the face.
#[test]
fn a_shaper_groups_original_shapes_in_symbol_art_are_not_picked() {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let square = |x: f64| shapes::rectangle(Rect::new(x, 0.0, x + 100.0, 100.0));
    let a = Node::path(d.alloc_id(), square(0.0), Appearance::default_art());
    let b = Node::path(d.alloc_id(), square(50.0), Appearance::default_art());
    let mut sources = Node::group(d.alloc_id(), vec![Arc::new(a), Arc::new(b)]);
    sources.name = Some(crate::shaper::SOURCES.into());
    sources.opacity = 0.0;
    let mut face = Node::path(d.alloc_id(), square(0.0), Appearance::default_art());
    face.name = Some(format!("{}0", crate::shaper::FILL_PREFIX));
    let mut group = Node::group(d.alloc_id(), vec![Arc::new(sources), Arc::new(face)]);
    group.extras_mut().shaper = Some(Box::default());
    let root = Node::group(d.alloc_id(), vec![Arc::new(group)]);
    d.symbols.push(crate::Symbol { dynamic: true, name: "Shaped".into(), art: Arc::new(root) });
    let inst = d.alloc_id();
    d.insert(Some(l), 0, Node::new(inst, NodeKind::SymbolInstance { symbol: "Shaped".into(), xf: vectorcraft_geom::Affine::IDENTITY })).unwrap();
    let n = d.node(inst).unwrap();
    let opt = HitOptions::default();
    assert_eq!(symbol_part_at(&d, n, Point::new(130.0, 50.0), opt), None, "over the erased original shape");
    assert_eq!(symbol_part_at(&d, n, Point::new(50.0, 50.0), opt), Some(vec![0, 1]), "over the shown face");
}
