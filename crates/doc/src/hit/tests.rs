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

/// #2345: a marquee touches a clip group only where its art shows. Over the clipped-away
/// half alone it selects nothing, and around a rectangle drawn beside the group's mask it
/// takes that rectangle alone; reaching into the mask takes the group. The same holds for
/// Group Selection's leaves and in isolation mode.
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
    assert!(marquee(&d, hidden, Some(g), false).is_empty());
    assert!(marquee(&d, hidden, Some(art_id), false).is_empty());
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
            kind: Box::new(crate::live::EnvelopeKind::Warp { style: "arc".to_string(), bend: 0.0, h: 0.0, v: 0.0, horizontal: true }),
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

const SELECTING: HitOptions =
    HitOptions { tol: 3.0, outline: false, path_only: false, type_path_only: false, skip_guides: false, clip_outlines: true, scope: None };

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
        }
    }
}

/// Select Behind finds a symbol instance by its symbol's art, as a click does: art from
/// (100, 60) to (140, 80), outside the 20 × 20 square around the origin, drawn where it lies by
/// one instance and 100 pt lower by an instance in a group.
#[test]
fn objects_at_finds_symbol_instances_by_their_art() {
    let mut d = Document::new(200.0, 200.0);
    let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(100.0, 60.0, 140.0, 80.0)), Appearance::default_art());
    d.symbols.push(crate::Symbol { name: "Far".into(), art: Arc::new(art) });
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
            kind: Box::new(crate::live::EnvelopeKind::Warp { style: "arc".to_string(), bend: 0.0, h: 0.0, v: 0.0, horizontal: true }),
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
