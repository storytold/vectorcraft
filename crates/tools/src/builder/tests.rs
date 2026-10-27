use super::*;
use crate::testutil::*;
use vectorcraft_doc::{Appearance, Selection};
use vectorcraft_geom::shapes;

fn two_rects() -> (Document, NodeId, NodeId) {
    two_rects_at(0.0)
}

/// Squares A (`x`..`x`+100) and B (`x`+50..`x`+150), 100 pt tall, overlapping by half.
fn two_rects_at(x: f64) -> (Document, NodeId, NodeId) {
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let a = d.alloc_id();
    d.insert(Some(l), 0, Node::path(a, shapes::rectangle(Rect::new(x, 0.0, x + 100.0, 100.0)), Appearance::default_art())).unwrap();
    let b = d.alloc_id();
    d.insert(Some(l), 1, Node::path(b, shapes::rectangle(Rect::new(x + 50.0, 0.0, x + 150.0, 100.0)), Appearance::default_art())).unwrap();
    (d, a, b)
}

fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
    PointerEvent::new(kind, x, y)
}

#[test]
fn shape_builder_hover_highlights_region() {
    let (d, a, b) = two_rects();
    let mut s = Selection::default();
    s.set([a, b]);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeBuilderTool::default();
    assert!(t.pointer(&cx, &ev(PointerKind::Move, 75.0, 50.0)).is_empty());
    let ov = t.overlays(&cx);
    assert_eq!(ov.len(), 2);
    let Overlay::Path { path, .. } = &ov[1] else { panic!() };
    let bb = path.bounding_box();
    assert!((bb.x0 - 50.0).abs() < 1e-6 && (bb.x1 - 100.0).abs() < 1e-6, "{bb:?}");
    t.pointer(&cx, &ev(PointerKind::Move, 400.0, 400.0));
    assert!(t.overlays(&cx).is_empty());
}

#[test]
fn shape_builder_drag_emits_merge() {
    let (d, a, b) = two_rects();
    let mut s = Selection::default();
    s.set([b, a]);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeBuilderTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, 25.0, 50.0));
    t.pointer(&cx, &ev(PointerKind::Drag, 75.0, 50.0));
    assert_eq!(t.touched.regions.len(), 2);
    assert!(t.busy());
    let acts = t.pointer(&cx, &ev(PointerKind::Up, 75.0, 50.0));
    let Action::Exec(id, v) = &acts[0] else { panic!("{acts:?}") };
    assert_eq!(id, "shapeBuilder.merge");
    assert_eq!(v["erase"], false);
    assert_eq!(v["ids"], json!([a.0, b.0]), "sorted back to front");
    assert!(v["points"].as_array().unwrap().len() >= 2);
}

#[test]
fn shape_builder_rebuilds_its_map_for_a_new_revision_at_the_same_address() {
    let (mut d, a, b) = two_rects();
    let mut s = Selection::default();
    s.set([a, b]);
    let p = paint();
    let mut t = ShapeBuilderTool::default();
    let cx1 = ToolContext { revision: (1, 1), ..cx(&d, &s, &p) };
    t.pointer(&cx1, &ev(PointerKind::Move, 75.0, 50.0));
    // A later revision of the document, with the squares 300 pt to the right, at the same address.
    d = two_rects_at(300.0).0;
    let cx2 = ToolContext { revision: (1, 2), ..cx(&d, &s, &p) };
    t.pointer(&cx2, &ev(PointerKind::Down, 325.0, 50.0));
    t.pointer(&cx2, &ev(PointerKind::Drag, 375.0, 50.0));
    let acts = t.pointer(&cx2, &ev(PointerKind::Up, 375.0, 50.0));
    assert!(matches!(acts.as_slice(), [Action::Exec(id, _)] if id == "shapeBuilder.merge"), "{acts:?}");
}

#[test]
fn shape_builder_builds_its_map_once_for_hover_alt_hover_and_a_merge() {
    let (d, s) = corner_rects();
    let p = paint();
    let mut t = ShapeBuilderTool::default();
    let alt = Mods { alt: true, ..Default::default() };
    let cx1 = ToolContext { revision: (1, 1), ..cx(&d, &s, &p) };
    t.pointer(&cx1, &ev(PointerKind::Move, 101.0, 75.0));
    assert!(matches!(t.hover, Some(Hit::Region(_))), "{:?}", t.hover);
    // Alt hovering over an edge uses the same map.
    t.pointer(&cx1, &ev(PointerKind::Move, 101.0, 75.0).with_mods(alt));
    assert!(matches!(t.hover, Some(Hit::Edge(_))), "{:?}", t.hover);
    assert_eq!(t.cache.builds, 1, "a hover then an Alt hover");
    t.pointer(&cx1, &ev(PointerKind::Down, 25.0, 25.0));
    t.pointer(&cx1, &ev(PointerKind::Drag, 75.0, 75.0));
    let acts = t.pointer(&cx1, &ev(PointerKind::Up, 75.0, 75.0));
    assert!(matches!(acts.as_slice(), [Action::Exec(id, _)] if id == "shapeBuilder.merge"), "{acts:?}");
    // The merge left the document as it was (it failed, say): the map still fits it.
    t.pointer(&cx1, &ev(PointerKind::Move, 75.0, 75.0));
    assert_eq!(t.cache.builds, 1, "a hover after a merge that changed nothing");
    let cx2 = ToolContext { revision: (1, 2), ..cx(&d, &s, &p) };
    t.pointer(&cx2, &ev(PointerKind::Move, 75.0, 75.0));
    assert_eq!(t.cache.builds, 2, "a hover on a new revision");
}

/// A row of `n` selected squares 20 pt wide, each overlapping the next by half: 4 segments each.
fn square_row(n: usize) -> (Document, Selection) {
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let mut s = Selection::default();
    for k in 0..n {
        let x = k as f64 * 10.0;
        let id = d.alloc_id();
        d.insert(Some(l), k, Node::path(id, shapes::rectangle(Rect::new(x, 0.0, x + 20.0, 20.0)), Appearance::default_art())).unwrap();
        s.add(id);
    }
    (d, s)
}

/// Polls `t`, as the desktop app does each frame, until it stops waiting.
fn poll_until_ready(t: &mut dyn Tool, cx: &ToolContext) {
    let start = std::time::Instant::now();
    while t.waiting() {
        assert!(start.elapsed() < std::time::Duration::from_secs(60), "the tool is still waiting");
        std::thread::sleep(std::time::Duration::from_millis(2));
        t.poll(cx, false);
    }
}

#[test]
fn shape_builder_hover_builds_a_large_map_on_another_thread() {
    let (d, s) = square_row(40);
    let p = paint();
    let mut t = ShapeBuilderTool::default();
    let cx1 = ToolContext { revision: (1, 1), ..cx(&d, &s, &p) };
    let hover = ev(PointerKind::Move, 15.0, 10.0);
    t.pointer(&cx1, &hover);
    assert_eq!(t.cache.builds, 0, "the first hover returns before the map is built");
    assert!(t.waiting() && t.hover.is_none() && t.overlays(&cx1).is_empty(), "{:?}", t.hover);
    poll_until_ready(&mut t, &cx1);
    assert!(matches!(t.hover, Some(Hit::Region(_))), "{:?}", t.hover);
    assert_eq!((t.cache.builds, t.cache.background), (1, 1));
    assert_eq!(t.overlays(&cx1).len(), 2, "the region's hatch and outline");
    // A new revision while the map of the one before it is being built: only the newer one's
    // map is used.
    let cx2 = ToolContext { revision: (1, 2), ..cx(&d, &s, &p) };
    let cx3 = ToolContext { revision: (1, 3), ..cx(&d, &s, &p) };
    t.pointer(&cx2, &hover);
    t.pointer(&cx3, &hover);
    poll_until_ready(&mut t, &cx3);
    assert!(matches!(t.hover, Some(Hit::Region(_))), "{:?}", t.hover);
    assert_eq!((t.cache.builds, t.cache.background), (3, 3), "the earlier revision's map, then the newer one's");
    assert_eq!(t.cache.key.as_ref().map(|k| k.1), Some((1, 3)));
    // A host without frames waits for the map.
    let cx4 = ToolContext { revision: (1, 4), ..cx(&d, &s, &p) };
    t.pointer(&cx4, &hover);
    assert!(t.waiting() && t.hover.is_none());
    t.poll(&cx4, true);
    assert!(!t.waiting());
    assert!(matches!(t.hover, Some(Hit::Region(_))), "{:?}", t.hover);
    assert_eq!((t.cache.builds, t.cache.background), (4, 4));
    // A press while the map is being built waits for it, and the drag merges.
    let cx5 = ToolContext { revision: (1, 5), ..cx(&d, &s, &p) };
    t.pointer(&cx5, &hover);
    t.pointer(&cx5, &ev(PointerKind::Down, 15.0, 10.0));
    t.pointer(&cx5, &ev(PointerKind::Drag, 35.0, 10.0));
    let acts = t.pointer(&cx5, &ev(PointerKind::Up, 35.0, 10.0));
    assert!(matches!(acts.as_slice(), [Action::Exec(id, _)] if id == "shapeBuilder.merge"), "{acts:?}");
    assert_eq!((t.cache.builds, t.cache.background), (5, 5));
    assert!(!t.waiting());
    // Changing an option forgets a map being built with the old one.
    let cx6 = ToolContext { revision: (1, 6), ..cx(&d, &s, &p) };
    t.pointer(&cx6, &hover);
    t.set_option("gapDetection", &json!(true));
    assert!(!t.waiting());
}

#[test]
fn shape_builder_hover_builds_a_small_map_at_once() {
    let (d, s) = square_row(3);
    let p = paint();
    let mut t = ShapeBuilderTool::default();
    let cx = cx(&d, &s, &p);
    t.pointer(&cx, &ev(PointerKind::Move, 15.0, 10.0));
    assert!(!t.waiting() && matches!(t.hover, Some(Hit::Region(_))), "{:?}", t.hover);
    assert_eq!((t.cache.builds, t.cache.background), (1, 0));
}

#[test]
fn shape_builder_alt_drag_erases_and_empty_drag_does_nothing() {
    let (d, a, b) = two_rects();
    let mut s = Selection::default();
    s.set([a, b]);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeBuilderTool::default();
    let alt = Mods { alt: true, ..Default::default() };
    t.pointer(&cx, &ev(PointerKind::Down, 125.0, 50.0).with_mods(alt));
    let acts = t.pointer(&cx, &ev(PointerKind::Up, 125.0, 50.0).with_mods(alt));
    let Action::Exec(_, v) = &acts[0] else { panic!() };
    assert_eq!(v["erase"], true);
    t.pointer(&cx, &ev(PointerKind::Down, 300.0, 300.0));
    assert!(t.pointer(&cx, &ev(PointerKind::Up, 300.0, 300.0)).is_empty());
}

/// Squares A (0..100) and B (50..150 both ways), overlapping at a corner, selected.
fn corner_rects() -> (Document, Selection) {
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let mut s = Selection::default();
    for (k, r) in [Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(50.0, 50.0, 150.0, 150.0)].into_iter().enumerate() {
        let id = d.alloc_id();
        d.insert(Some(l), k, Node::path(id, shapes::rectangle(r), Appearance::default_art())).unwrap();
        s.add(id);
    }
    (d, s)
}

fn colors(ov: &[Overlay]) -> Vec<[u8; 3]> {
    ov.iter().map(|o| if let Overlay::Path { color, .. } = o { *color } else { panic!("{o:?}") }).collect()
}

#[test]
fn shape_builder_alt_switches_to_erase_mode_while_held() {
    let (d, s) = corner_rects();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeBuilderTool::default();
    let alt = Mods { alt: true, ..Default::default() };
    let at = Point::new(75.0, 75.0);
    // Merge mode: a grey mesh and the plus pointer.
    t.pointer(&cx, &ev(PointerKind::Move, at.x, at.y));
    assert_eq!(colors(&t.overlays(&cx)), [HATCH; 2]);
    assert_eq!(t.cursor(&cx, at, Mods::default()), Cursor::ShapeBuilder);
    // Alt held: what would go highlights red, and the pointer shows a minus.
    t.pointer(&cx, &ev(PointerKind::Move, at.x, at.y).with_mods(alt));
    assert_eq!(colors(&t.overlays(&cx)), [HIGHLIGHT_RED; 2]);
    assert_eq!(t.cursor(&cx, at, alt), Cursor::ShapeBuilderErase);
    // Released: merge mode again.
    t.pointer(&cx, &ev(PointerKind::Move, at.x, at.y));
    assert_eq!(colors(&t.overlays(&cx)), [HATCH; 2]);
    assert_eq!(t.cursor(&cx, at, Mods::default()), Cursor::ShapeBuilder);
    // A drag keeps the mode it started in.
    t.pointer(&cx, &ev(PointerKind::Down, at.x, at.y).with_mods(alt));
    assert_eq!(t.cursor(&cx, at, Mods::default()), Cursor::ShapeBuilderErase);
    let acts = t.pointer(&cx, &ev(PointerKind::Up, at.x, at.y));
    let Action::Exec(_, v) = &acts[0] else { panic!("{acts:?}") };
    assert_eq!(v["erase"], true);
}

#[test]
fn shape_builder_alt_on_an_edge_highlights_and_deletes_it() {
    let (d, s) = corner_rects();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeBuilderTool::default();
    let alt = Mods { alt: true, ..Default::default() };
    // A's side inside B, between the points where B's outline crosses it.
    t.pointer(&cx, &ev(PointerKind::Move, 101.0, 75.0).with_mods(alt));
    assert!(matches!(t.hover, Some(Hit::Edge(_))), "{:?}", t.hover);
    let ov = t.overlays(&cx);
    let [Overlay::Path { path, color: HIGHLIGHT_RED, width: 3.0, .. }] = ov.as_slice() else { panic!("{ov:?}") };
    assert_eq!(path.bounding_box(), Rect::new(50.0, 50.0, 100.0, 100.0));
    // Without Alt it's the region under the pointer.
    t.pointer(&cx, &ev(PointerKind::Move, 101.0, 75.0));
    assert!(matches!(t.hover, Some(Hit::Region(_))));
    t.pointer(&cx, &ev(PointerKind::Down, 101.0, 75.0).with_mods(alt));
    assert_eq!(t.touched.edges.len(), 1);
    let acts = t.pointer(&cx, &ev(PointerKind::Up, 101.0, 75.0).with_mods(alt));
    let Action::Exec(id, v) = &acts[0] else { panic!("{acts:?}") };
    assert_eq!((id.as_str(), &v["erase"], &v["tolerance"]), ("shapeBuilder.merge", &json!(true), &json!(4.0)));
}

#[test]
fn shape_builder_erase_drag_into_regions_shows_the_regions_not_the_edges_it_crosses() {
    let (d, s) = corner_rects();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeBuilderTool::default();
    let alt = Mods { alt: true, ..Default::default() };
    t.pointer(&cx, &ev(PointerKind::Down, 25.0, 25.0).with_mods(alt));
    t.pointer(&cx, &ev(PointerKind::Drag, 75.0, 75.0).with_mods(alt));
    assert_eq!(t.touched.regions.len(), 2);
    assert!(!t.touched.edges.is_empty(), "the drag crossed B's outline");
    assert!(t.cache.map.erased_edges(&t.touched).is_empty(), "which goes with the regions");
    // Two regions (mesh + outline each) and the drag's trail, all red.
    assert_eq!(colors(&t.overlays(&cx)), [HIGHLIGHT_RED; 5]);
}

/// Rect A (0..100) and rect B (98..198), both 100 tall, selected: they overlap in a 2 pt strip.
fn sliver_rects() -> (Document, Selection) {
    rects(&[Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(98.0, 0.0, 198.0, 100.0)])
}

/// The rectangles `rs`, back to front, selected.
fn rects(rs: &[Rect]) -> (Document, Selection) {
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let mut s = Selection::default();
    for (k, r) in rs.iter().enumerate() {
        let id = d.alloc_id();
        d.insert(Some(l), k, Node::path(id, shapes::rectangle(*r), Appearance::default_art())).unwrap();
        s.add(id);
    }
    (d, s)
}

fn map_of(d: &Document, s: &Selection) -> BuilderMap {
    BuilderMap::new(d, &sorted_roots(d, &s.objects), true, true).0
}

#[test]
fn shape_builder_erase_drag_across_an_overlap_narrower_than_twice_the_tolerance_touches_it() {
    let (d, s) = sliver_rects();
    let map = map_of(&d, &s);
    let strip = map.region_at(Point::new(99.0, 50.0)).unwrap();
    // Every point of the strip is within 4 pt of its edges.
    let t = map.builder_hits_along(&[Point::new(50.0, 50.0), Point::new(150.0, 50.0)], Some(4.0));
    assert_eq!(t.regions.len(), 3, "{t:?}");
    assert!(t.regions.contains(&strip), "{t:?}");
    assert!(map.erased_edges(&t).is_empty(), "the strip's edges go with it");
    // A drag at a shallow angle goes about 10 pt through the strip and touches it too.
    let t = map.builder_hits_along(&[Point::new(90.0, 10.0), Point::new(106.0, 90.0)], Some(4.0));
    assert_eq!(t.regions.len(), 3, "{t:?}");
    // So does a drag that starts in the strip, and one that goes into it and back out.
    let t = map.builder_hits_along(&[Point::new(99.0, 50.0), Point::new(150.0, 50.0)], Some(4.0));
    assert!(t.regions.contains(&strip), "{t:?}");
    let t = map.builder_hits_along(&[Point::new(50.0, 50.0), Point::new(99.0, 50.0), Point::new(50.0, 51.0)], Some(4.0));
    assert!(t.regions.contains(&strip), "{t:?}");
    // Merging is unchanged, and so is the plain hit test other tools use.
    assert_eq!(map.builder_hit(Point::new(99.0, 50.0), Some(4.0)), Some(Hit::Region(strip)));
    assert!(matches!(map.hit(Point::new(99.0, 50.0), Some(4.0)), Some(Hit::Edge(_))));
    assert_eq!(map.builder_hit(Point::new(99.0, 50.0), None), Some(Hit::Region(strip)));
}

#[test]
fn shape_builder_erase_drag_across_a_thin_bar_touches_it_from_either_side() {
    // A 1.5 pt bar inside a square, and the same bar on its own.
    let bar = Rect::new(49.0, 10.0, 50.5, 90.0);
    for rs in [vec![Rect::new(0.0, 0.0, 100.0, 100.0), bar], vec![bar]] {
        let (d, s) = rects(&rs);
        let map = map_of(&d, &s);
        let inner = map.region_at(Point::new(49.75, 50.0)).unwrap();
        let t = map.builder_hits_along(&[Point::new(20.0, 50.0), Point::new(80.0, 50.0)], Some(4.0));
        assert!(t.regions.contains(&inner), "{rs:?}: {t:?}");
        assert_eq!(t.regions.len(), rs.len(), "{rs:?}: {t:?}");
        // 0.1 pt from an end, the nearest outline is the end, and the bar is still 1.5 pt
        // across from its sides: a click there and a drag across the bar there touch it.
        assert_eq!(map.builder_hit(Point::new(49.75, 10.1), Some(4.0)), Some(Hit::Region(inner)), "{rs:?}");
        assert_eq!(map.builder_hit(Point::new(49.75, 89.9), Some(4.0)), Some(Hit::Region(inner)), "{rs:?}");
        let t = map.builder_hits_along(&[Point::new(20.0, 10.1), Point::new(80.0, 10.1)], Some(4.0));
        assert!(t.regions.contains(&inner), "{rs:?}: {t:?}");
    }
    // Near a corner of the square, both of its edges there are close, and the square is wide
    // across from each: the edge is touched.
    let (d, s) = rects(&[Rect::new(0.0, 0.0, 100.0, 100.0), bar]);
    let map = map_of(&d, &s);
    assert!(matches!(map.builder_hit(Point::new(1.0, 1.0), Some(4.0)), Some(Hit::Edge(_))));
    // So it is when the square's top edge has a point 2 pt from the corner: the end of the
    // segment past that point is not where the square is narrow.
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let pts = [(0.0, 0.0), (2.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)].map(|(x, y)| Point::new(x, y));
    let path = PathData::new(vec![SubPath::polyline(&pts, true)]);
    d.insert(Some(l), 0, Node::path(id, path, Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.add(id);
    let map = map_of(&d, &s);
    let square = map.region_at(Point::new(50.0, 50.0)).unwrap();
    assert_eq!(map.regions[square].1.segments().count(), 5, "the point on the top edge stays");
    assert!(matches!(map.builder_hit(Point::new(1.0, 1.0), Some(4.0)), Some(Hit::Edge(_))));
    // A bar whose sides each have a point level with a click 0.1 pt from its end: the click
    // and a drag across the bar there still touch the bar.
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let pts = [(49.0, 10.0), (50.5, 10.0), (50.5, 10.1), (50.5, 90.0), (49.0, 90.0), (49.0, 10.1)].map(|(x, y)| Point::new(x, y));
    let path = PathData::new(vec![SubPath::polyline(&pts, true)]);
    d.insert(Some(l), 0, Node::path(id, path, Appearance::default_art())).unwrap();
    let mut s = Selection::default();
    s.add(id);
    let map = map_of(&d, &s);
    let split = map.region_at(Point::new(49.75, 50.0)).unwrap();
    assert_eq!(map.regions[split].1.segments().count(), 6, "the points on the sides stay");
    assert_eq!(map.builder_hit(Point::new(49.75, 10.1), Some(4.0)), Some(Hit::Region(split)));
    let t = map.builder_hits_along(&[Point::new(20.0, 10.1), Point::new(80.0, 10.1)], Some(4.0));
    assert!(t.regions.contains(&split), "{t:?}");
}

#[test]
fn shape_builder_erase_drag_along_an_edge_does_not_touch_the_regions_beside_it() {
    let (d, s) = grid();
    let map = map_of(&d, &s);
    // 1 pt above the edges between the top row and the middle one, through all three squares.
    let t = map.builder_hits_along(&[Point::new(50.0, 99.0), Point::new(250.0, 99.0)], Some(4.0));
    assert!(t.regions.is_empty(), "{t:?}");
    assert!(!t.edges.is_empty());
    // Crossing the edge 2 pt before a junction clips the square below without touching it.
    let pts = [Point::new(50.0, 99.0), Point::new(97.0, 99.0), Point::new(99.0, 101.0), Point::new(150.0, 101.0)];
    let t = map.builder_hits_along(&pts, Some(4.0));
    let clipped = map.region_at(Point::new(99.0, 101.0)).unwrap();
    assert!(!t.regions.contains(&clipped), "{t:?}");
}

#[test]
fn shape_builder_alt_hover_over_a_narrow_overlap_highlights_it() {
    let (d, s) = sliver_rects();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeBuilderTool::default();
    let alt = Mods { alt: true, ..Default::default() };
    t.pointer(&cx, &ev(PointerKind::Move, 99.0, 50.0).with_mods(alt));
    let strip = t.cache.map.region_at(Point::new(99.0, 50.0));
    assert_eq!(t.hover, strip.map(Hit::Region));
    // Beside a wide region, the edge.
    t.pointer(&cx, &ev(PointerKind::Move, 50.0, 99.0).with_mods(alt));
    assert!(matches!(t.hover, Some(Hit::Edge(_))), "{:?}", t.hover);
}

#[test]
fn shape_builder_alt_drag_highlights_an_overlap_it_crosses_over_two_moves() {
    let (d, s) = sliver_rects();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeBuilderTool::default();
    let alt = Mods { alt: true, ..Default::default() };
    t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0).with_mods(alt));
    // One move ends inside the strip, and the next one leaves it.
    t.pointer(&cx, &ev(PointerKind::Drag, 99.0, 50.0).with_mods(alt));
    t.pointer(&cx, &ev(PointerKind::Drag, 150.0, 50.0).with_mods(alt));
    assert_eq!(t.touched.regions.len(), 3, "{:?}", t.touched);
    assert!(t.cache.map.erased_edges(&t.touched).is_empty());
    let acts = t.pointer(&cx, &ev(PointerKind::Up, 150.0, 50.0).with_mods(alt));
    let Action::Exec(id, v) = &acts[0] else { panic!("{acts:?}") };
    assert_eq!((id.as_str(), &v["erase"], &v["tolerance"]), ("shapeBuilder.merge", &json!(true), &json!(4.0)));
}

/// A 3×3 grid of touching 100 pt squares from (0, 0) to (300, 300), selected.
fn grid() -> (Document, Selection) {
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let mut s = Selection::default();
    for k in 0..9 {
        let (x, y) = ((k % 3) as f64 * 100.0, (k / 3) as f64 * 100.0);
        let id = d.alloc_id();
        d.insert(Some(l), k, Node::path(id, shapes::rectangle(Rect::new(x, y, x + 100.0, y + 100.0)), Appearance::default_art())).unwrap();
        s.add(id);
    }
    (d, s)
}

/// The paths of an overlay list (a shaded region draws two: its mesh and its outline), and its
/// marquees.
fn paths_and_marquees(ov: &[Overlay]) -> (usize, Vec<Rect>) {
    let paths = ov.iter().filter(|o| matches!(o, Overlay::Path { .. })).count();
    (paths, ov.iter().filter_map(|o| if let Overlay::Marquee(r) = o { Some(*r) } else { None }).collect())
}

#[test]
fn shape_builder_shift_drag_highlights_and_merges_every_region_in_its_marquee() {
    let (d, s) = grid();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeBuilderTool::default();
    let shift = Mods { shift: true, ..Default::default() };
    t.pointer(&cx, &ev(PointerKind::Down, 250.0, 250.0).with_mods(shift));
    // Shift may be let go once the drag is on.
    t.pointer(&cx, &ev(PointerKind::Drag, 150.0, 150.0));
    t.pointer(&cx, &ev(PointerKind::Drag, 50.0, 50.0));
    assert_eq!(t.touched.regions.len(), 9, "{:?}", t.touched);
    assert_eq!(t.touched.regions[0], t.cache.map.region_at(Point::new(250.0, 250.0)).unwrap(), "where the drag started comes first");
    assert_eq!(paths_and_marquees(&t.overlays(&cx)), (18, vec![Rect::new(50.0, 50.0, 250.0, 250.0)]), "nine regions, no trail");
    let acts = t.pointer(&cx, &ev(PointerKind::Up, 50.0, 50.0));
    let Action::Exec(id, v) = &acts[0] else { panic!("{acts:?}") };
    assert_eq!((id.as_str(), &v["rect"], &v["erase"]), ("shapeBuilder.merge", &json!([250.0, 250.0, -200.0, -200.0]), &json!(false)));
    assert!(v.get("points").is_none(), "{v}");
    assert!(t.overlays(&cx).iter().all(|o| !matches!(o, Overlay::Marquee(_))), "the marquee goes with the drag");
}

#[test]
fn shape_builder_alt_shift_drag_marks_the_regions_in_its_marquee_for_deleting() {
    let (d, s) = grid();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeBuilderTool::default();
    let alt_shift = Mods { alt: true, shift: true, ..Default::default() };
    // A narrow marquee down the middle column.
    t.pointer(&cx, &ev(PointerKind::Down, 140.0, 50.0).with_mods(alt_shift));
    t.pointer(&cx, &ev(PointerKind::Drag, 160.0, 250.0).with_mods(alt_shift));
    assert_eq!(t.touched.regions.len(), 3);
    let ov = t.overlays(&cx);
    assert!(ov.iter().all(|o| matches!(o, Overlay::Marquee(_) | Overlay::Path { color: HIGHLIGHT_RED, .. })), "{ov:?}");
    assert_eq!(paths_and_marquees(&ov).0, 6, "three regions");
    let acts = t.pointer(&cx, &ev(PointerKind::Up, 160.0, 250.0).with_mods(alt_shift));
    let Action::Exec(_, v) = &acts[0] else { panic!("{acts:?}") };
    assert_eq!((&v["rect"], &v["erase"]), (&json!([140.0, 50.0, 20.0, 200.0]), &json!(true)));
}

/// Without Shift a drag still touches what the pointer passes over; a Shift-click that does not
/// move is a click.
#[test]
fn shape_builder_drag_without_shift_and_shift_click_send_points() {
    let (d, s) = grid();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeBuilderTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, 250.0, 250.0));
    t.pointer(&cx, &ev(PointerKind::Drag, 150.0, 150.0));
    t.pointer(&cx, &ev(PointerKind::Drag, 50.0, 50.0));
    assert_eq!(t.touched.regions.len(), 3, "the diagonal");
    let (paths, marquees) = paths_and_marquees(&t.overlays(&cx));
    assert_eq!((paths, marquees.len()), (7, 0), "three regions and the trail");
    let acts = t.pointer(&cx, &ev(PointerKind::Up, 50.0, 50.0));
    let Action::Exec(_, v) = &acts[0] else { panic!("{acts:?}") };
    assert_eq!(v["points"], json!([[250.0, 250.0], [150.0, 150.0], [50.0, 50.0]]));
    assert!(v.get("rect").is_none(), "{v}");
    let shift = Mods { shift: true, ..Default::default() };
    t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0).with_mods(shift));
    assert_eq!(t.touched.regions.len(), 1);
    let acts = t.pointer(&cx, &ev(PointerKind::Up, 150.0, 150.0).with_mods(shift));
    let Action::Exec(_, v) = &acts[0] else { panic!("{acts:?}") };
    assert_eq!((&v["points"], v.get("rect")), (&json!([[150.0, 150.0]]), None));
}

#[test]
fn marquee_regions_touch_crossed_outlines_held_anchors_and_the_region_around_it() {
    let (d, s) = grid();
    let (map, _) = BuilderMap::new(&d, &sorted_roots(&d, &s.objects), false, true);
    let at = |x: f64, y: f64| map.region_at(Point::new(x, y)).unwrap();
    // Wholly inside the middle square, then crossing into the one to its right.
    assert_eq!(map.regions_in_marquee(Point::new(120.0, 120.0), Point::new(180.0, 180.0)), [at(150.0, 150.0)]);
    assert_eq!(map.regions_in_marquee(Point::new(120.0, 120.0), Point::new(220.0, 180.0)), [at(150.0, 150.0), at(250.0, 150.0)]);
    // Ending on a shared side does not reach the square past it.
    assert_eq!(map.regions_in_marquee(Point::new(120.0, 120.0), Point::new(200.0, 180.0)), [at(150.0, 150.0)]);
    // Around the whole grid, from outside it: every square, with no region under either corner.
    assert_eq!(map.regions_in_marquee(Point::new(-10.0, -10.0), Point::new(310.0, 310.0)).len(), 9);
    assert!(map.regions_in_marquee(Point::new(400.0, 400.0), Point::new(450.0, 450.0)).is_empty());
    // Where the drag started first, then where it ends, whichever way it runs.
    for (a, b) in [((50.0, 50.0), (250.0, 250.0)), ((250.0, 250.0), (50.0, 50.0)), ((250.0, 50.0), (50.0, 250.0))] {
        let got = map.regions_in_marquee(a.into(), b.into());
        assert_eq!((got.len(), got[0], got[1]), (9, at(a.0, a.1), at(b.0, b.1)), "{a:?} to {b:?}");
    }
}

/// With In Merge Mode, Clicking Stroke Splits the Path on, a Shift-drag that starts on a stroke
/// sends its marquee and no split, however little it moves; a Shift-click there splits the
/// stroke as a click does.
#[test]
fn shape_builder_shift_drag_on_a_stroke_merges_and_a_shift_click_splits() {
    let (d, s) = corner_rects();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeBuilderTool::default();
    t.set_option("clickStrokeSplitsPath", &json!(true));
    let shift = Mods { shift: true, ..Default::default() };
    // 1 pt outside A's left side, a move of less than 3 pixels, and one across the art.
    for (to, rect) in [((0.5, 26.0), json!([-1.0, 25.0, 1.5, 1.0])), ((75.0, 75.0), json!([-1.0, 25.0, 76.0, 50.0]))] {
        t.pointer(&cx, &ev(PointerKind::Move, -1.0, 25.0));
        t.pointer(&cx, &ev(PointerKind::Down, -1.0, 25.0).with_mods(shift));
        t.pointer(&cx, &ev(PointerKind::Drag, to.0, to.1).with_mods(shift));
        let acts = t.pointer(&cx, &ev(PointerKind::Up, to.0, to.1).with_mods(shift));
        let [Action::Exec(_, v)] = acts.as_slice() else { panic!("{acts:?}") };
        assert_eq!((&v["rect"], v.get("points"), v.get("splitStroke")), (&rect, None, None), "{v}");
    }
    t.pointer(&cx, &ev(PointerKind::Move, -1.0, 25.0));
    t.pointer(&cx, &ev(PointerKind::Down, -1.0, 25.0).with_mods(shift));
    let acts = t.pointer(&cx, &ev(PointerKind::Up, -1.0, 25.0).with_mods(shift));
    let [Action::Exec(_, v)] = acts.as_slice() else { panic!("{acts:?}") };
    assert_eq!((&v["splitStroke"], &v["points"], v.get("rect")), (&json!(true), &json!([[-1.0, 25.0]]), None), "{v}");
}

/// While a Shift-drag that started over a stroke a click would split draws its marquee, the
/// pointer shows the Shape Builder, not the scissors, and the stroke is not highlighted.
#[test]
fn shape_builder_shift_drag_shows_no_scissors_or_stroke_highlight() {
    let (d, s) = corner_rects();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeBuilderTool::default();
    t.set_option("clickStrokeSplitsPath", &json!(true));
    let shift = Mods { shift: true, ..Default::default() };
    let highlighted = |ov: &[Overlay]| ov.iter().any(|o| matches!(o, Overlay::Path { color: STROKE_HIGHLIGHT, width: 3.0, .. }));
    t.pointer(&cx, &ev(PointerKind::Move, -1.0, 25.0));
    assert_eq!(t.cursor(&cx, Point::new(-1.0, 25.0), shift), Cursor::Scissors);
    assert!(highlighted(&t.overlays(&cx)));
    t.pointer(&cx, &ev(PointerKind::Down, -1.0, 25.0).with_mods(shift));
    t.pointer(&cx, &ev(PointerKind::Drag, 75.0, 75.0).with_mods(shift));
    let ov = t.overlays(&cx);
    assert!(ov.iter().any(|o| matches!(o, Overlay::Marquee(_))) && !highlighted(&ov), "{ov:?}");
    assert_eq!(t.cursor(&cx, Point::new(75.0, 75.0), shift), Cursor::ShapeBuilder);
}

#[test]
fn shape_builder_stroke_options_round_trip_and_pick_the_side_under_the_pointer() {
    let (d, s) = corner_rects();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = ShapeBuilderTool::default();
    let o = t.options();
    assert_eq!((&o["clickStrokeSplitsPath"], &o["highlightStroke"], &o["highlightColor"]), (&json!(false), &json!(true), &json!("#ff4f4f")));
    // With the option off, the pointer 1 pt outside A's left side highlights nothing.
    t.pointer(&cx, &ev(PointerKind::Move, -1.0, 25.0));
    assert!(t.overlays(&cx).is_empty());
    assert_eq!(t.cursor(&cx, Point::new(-1.0, 25.0), Mods::default()), Cursor::ShapeBuilder);
    t.set_option("clickStrokeSplitsPath", &json!(true));
    t.set_option("highlightColor", &json!("#00ff00"));
    // A value that is not a color or not a switch is ignored.
    t.set_option("highlightColor", &json!("nope"));
    t.set_option("highlightStroke", &json!("off"));
    let o = t.options();
    assert_eq!((&o["clickStrokeSplitsPath"], &o["highlightStroke"], &o["highlightColor"]), (&json!(true), &json!(true), &json!("#00ff00")));
    // On, the tool draws A's left side from corner to corner in the highlight color and shows the scissors.
    t.pointer(&cx, &ev(PointerKind::Move, -1.0, 25.0));
    let ov = t.overlays(&cx);
    let [Overlay::Path { path, color: [0, 0xff, 0], width: 3.0, .. }] = ov.as_slice() else { panic!("{ov:?}") };
    assert_eq!(path.bounding_box(), Rect::new(0.0, 0.0, 0.0, 100.0));
    assert_eq!(t.cursor(&cx, Point::new(-1.0, 25.0), Mods::default()), Cursor::Scissors);
    // With Alt the tool is in erase mode, which deletes edges and does not split them.
    let alt = Mods { alt: true, ..Default::default() };
    t.pointer(&cx, &ev(PointerKind::Move, -1.0, 25.0).with_mods(alt));
    assert_eq!(t.cursor(&cx, Point::new(-1.0, 25.0), alt), Cursor::ShapeBuilderErase);
    assert!(t.stroke.is_none());
    // A click splits; the command gets the point, the switch and the tolerance.
    t.pointer(&cx, &ev(PointerKind::Down, -1.0, 25.0));
    let acts = t.pointer(&cx, &ev(PointerKind::Up, -1.0, 25.0));
    let [Action::Exec(id, v)] = acts.as_slice() else { panic!("{acts:?}") };
    assert_eq!((id.as_str(), &v["splitStroke"], &v["erase"], &v["tolerance"]), ("shapeBuilder.merge", &json!(true), &json!(false), &json!(4.0)));
    // A release up to 3 pixels from the press is still a click, which splits where the press was.
    t.pointer(&cx, &ev(PointerKind::Down, -1.0, 25.0));
    t.pointer(&cx, &ev(PointerKind::Drag, -1.5, 26.0));
    let acts = t.pointer(&cx, &ev(PointerKind::Up, 0.5, 27.0));
    let [Action::Exec(_, v)] = acts.as_slice() else { panic!("{acts:?}") };
    assert_eq!((&v["splitStroke"], &v["points"]), (&json!(true), &json!([[-1.0, 25.0]])));
    // A release farther away is a drag, which splits nothing.
    t.pointer(&cx, &ev(PointerKind::Down, -1.0, 25.0));
    assert!(t.pointer(&cx, &ev(PointerKind::Up, -1.0, 30.0)).is_empty());
    // So is a Straight Line drag that goes away and ends back near the press.
    t.set_option("selection", &json!("straightLine"));
    t.pointer(&cx, &ev(PointerKind::Down, -1.0, 25.0));
    t.pointer(&cx, &ev(PointerKind::Drag, 30.0, 25.0));
    let acts = t.pointer(&cx, &ev(PointerKind::Up, -0.5, 25.5));
    assert!(!acts.iter().any(|a| matches!(a, Action::Exec(_, v) if v.get("splitStroke").is_some())), "{acts:?}");
    t.set_option("selection", &json!("freeform"));
    // A drag merges as before and sends no switch.
    t.pointer(&cx, &ev(PointerKind::Down, 25.0, 25.0));
    t.pointer(&cx, &ev(PointerKind::Drag, 75.0, 75.0));
    let acts = t.pointer(&cx, &ev(PointerKind::Up, 75.0, 75.0));
    let [Action::Exec(_, v)] = acts.as_slice() else { panic!("{acts:?}") };
    assert!(v.get("splitStroke").is_none(), "{v}");
}

#[test]
fn hatch_stays_inside_region() {
    let bp = shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 20.0)).to_bezpath();
    let h = hatch(&bp, 4.0);
    let bb = h.bounding_box();
    assert!(bb.x0 >= -1e-6 && bb.y0 >= -1e-6 && bb.x1 <= 40.0 + 1e-6 && bb.y1 <= 20.0 + 1e-6);
    assert!(h.elements().len() > 10);
}

/// An empty hidden sources group for a Live Paint group made in a test.
fn empty_sources(d: &mut Document) -> Arc<Node> {
    let mut g = Node::group(d.alloc_id(), vec![]);
    g.name = Some(SOURCES_NAME.into());
    g.visible = false;
    Arc::new(g)
}

fn live_group_doc() -> (Document, NodeId, NodeId, NodeId) {
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let f1 = d.alloc_id();
    let f2 = d.alloc_id();
    let e = d.alloc_id();
    let mut face1 = Node::path(f1, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default());
    face1.name = Some(FACE_NAME.into());
    let mut face2 = Node::path(f2, shapes::rectangle(Rect::new(50.0, 0.0, 100.0, 50.0)), Appearance::default());
    face2.name = Some(FACE_NAME.into());
    let mut edge = Node::path(e, shapes::line(Point::new(50.0, 0.0), Point::new(50.0, 50.0)), Appearance::default());
    edge.name = Some(EDGE_NAME.into());
    let gid = d.alloc_id();
    let mut g = Node::group(gid, vec![empty_sources(&mut d), Arc::new(face1), Arc::new(face2), Arc::new(edge)]);
    g.name = Some(LIVE_PAINT_NAME.into());
    d.insert(Some(l), 0, g).unwrap();
    (d, gid, f2, e)
}

/// Adds a Live Paint group of one face `r` with its four sides as edges (left, top, right,
/// bottom) in front of the layer's art; returns the group and the edges.
fn live_square(d: &mut Document, r: Rect) -> (NodeId, [NodeId; 4]) {
    let l = d.layers[0].id;
    let mut face = Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::default());
    face.name = Some(FACE_NAME.into());
    let (a, b, c, e) = (Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1));
    let mut kids = vec![empty_sources(d), Arc::new(face)];
    let mut ids = [NodeId(0); 4];
    for (k, (from, to)) in [(e, a), (a, b), (b, c), (c, e)].into_iter().enumerate() {
        ids[k] = d.alloc_id();
        let mut edge = Node::path(ids[k], shapes::line(from, to), Appearance::default());
        edge.name = Some(EDGE_NAME.into());
        kids.push(Arc::new(edge));
    }
    let gid = d.alloc_id();
    let mut g = Node::group(gid, kids);
    g.name = Some(LIVE_PAINT_NAME.into());
    let at = d.layers[0].children().map_or(0, |c| c.len());
    d.insert(Some(l), at, g).unwrap();
    (gid, ids)
}

#[test]
fn bucket_click_fills_face_and_shift_click_strokes_edge() {
    let (d, gid, f2, _e) = live_group_doc();
    assert_eq!(live_paint_at(&d, Point::new(75.0, 25.0)), Some(gid));
    assert_eq!(face_at(d.node(gid).unwrap(), Point::new(75.0, 25.0)).unwrap().id, f2);
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = LivePaintBucketTool::default();
    t.pointer(&cx, &ev(PointerKind::Move, 75.0, 25.0));
    let ov = t.overlays(&cx);
    assert!(matches!(&ov[0], Overlay::Path { color: HIGHLIGHT_RED, .. }));
    let acts = t.pointer(&cx, &ev(PointerKind::Down, 75.0, 25.0));
    let Action::Exec(id, v) = &acts[0] else { panic!() };
    assert_eq!(id, "livePaint.fill");
    assert_eq!(v["group"], gid.0);
    let shift = Mods { shift: true, ..Default::default() };
    let acts = t.pointer(&cx, &ev(PointerKind::Down, 51.0, 25.0).with_mods(shift));
    let Action::Exec(id, _) = &acts[0] else { panic!() };
    assert_eq!(id, "livePaint.strokeEdge");
}

#[test]
fn bucket_on_plain_selection_makes_live_paint() {
    let (d, a, b) = two_rects();
    let mut s = Selection::default();
    s.set([a, b]);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = LivePaintBucketTool::default();
    let acts = t.pointer(&cx, &ev(PointerKind::Down, 75.0, 50.0));
    let Action::Exec(id, v) = &acts[0] else { panic!() };
    assert_eq!(id, "livePaint.fill");
    assert_eq!(v["ids"], json!([a.0, b.0]));
}

#[test]
fn bucket_rebuilds_its_map_for_a_new_revision_at_the_same_address() {
    let (mut d, a, b) = two_rects();
    let mut s = Selection::default();
    s.set([a, b]);
    let p = paint();
    let mut t = LivePaintBucketTool::default();
    let cx1 = ToolContext { revision: (1, 1), ..cx(&d, &s, &p) };
    t.pointer(&cx1, &ev(PointerKind::Move, 75.0, 50.0));
    // A later revision of the document, with the squares 300 pt to the right, at the same address.
    d = two_rects_at(300.0).0;
    let cx2 = ToolContext { revision: (1, 2), ..cx(&d, &s, &p) };
    let acts = t.pointer(&cx2, &ev(PointerKind::Down, 375.0, 50.0));
    assert_eq!(acts, vec![Action::Exec("livePaint.fill".into(), json!({ "ids": [a.0, b.0], "point": [375.0, 50.0] }))]);
}

#[test]
fn bucket_keeps_its_map_while_the_document_is_unchanged() {
    let (d, a, b) = two_rects();
    let mut s = Selection::default();
    s.set([a, b]);
    let p = paint();
    let mut t = LivePaintBucketTool::default();
    let cx1 = ToolContext { revision: (1, 1), ..cx(&d, &s, &p) };
    t.pointer(&cx1, &ev(PointerKind::Move, 75.0, 50.0));
    let acts = t.pointer(&cx1, &ev(PointerKind::Down, 75.0, 50.0));
    assert!(matches!(acts.as_slice(), [Action::Exec(id, _)] if id == "livePaint.fill"), "{acts:?}");
    // The fill left the document as it was: the map still fits it.
    t.pointer(&cx1, &ev(PointerKind::Move, 25.0, 50.0));
    assert_eq!(t.cache.builds, 1, "a hover after a fill that changed nothing");
    let cx2 = ToolContext { revision: (1, 2), ..cx(&d, &s, &p) };
    t.pointer(&cx2, &ev(PointerKind::Move, 25.0, 50.0));
    assert_eq!(t.cache.builds, 2, "a hover on a new revision");
}

#[test]
fn bucket_hover_builds_a_large_map_on_another_thread() {
    let (d, s) = square_row(40);
    let p = paint();
    let mut t = LivePaintBucketTool::default();
    let cx = ToolContext { revision: (1, 1), ..cx(&d, &s, &p) };
    let hover = ev(PointerKind::Move, 15.0, 10.0);
    t.pointer(&cx, &hover);
    assert_eq!(t.cache.builds, 0, "the first hover returns before the map is built");
    assert!(t.waiting() && t.hover == LpHover::None, "{:?}", t.hover);
    poll_until_ready(&mut t, &cx);
    assert!(matches!(t.hover, LpHover::Region(_)), "{:?}", t.hover);
    assert_eq!((t.cache.builds, t.cache.background), (1, 1));
    assert_eq!(t.overlays(&cx).len(), 1);
}

#[test]
fn bucket_takes_the_map_it_waits_for_while_hovering_a_live_paint_group() {
    let (mut d, s) = square_row(40);
    live_square(&mut d, Rect::new(100.0, 100.0, 200.0, 200.0));
    let p = paint();
    let mut t = LivePaintBucketTool::default();
    let cx = cx(&d, &s, &p);
    t.pointer(&cx, &ev(PointerKind::Move, 15.0, 10.0));
    assert!(t.waiting());
    t.pointer(&cx, &ev(PointerKind::Move, 150.0, 150.0));
    assert!(matches!(t.hover, LpHover::Part(_)), "{:?}", t.hover);
    poll_until_ready(&mut t, &cx);
    assert!(matches!(t.hover, LpHover::Part(_)), "{:?}", t.hover);
    assert_eq!((t.cache.builds, t.cache.background), (1, 1));
}

#[test]
fn live_paint_selection_hover_builds_no_map_of_the_selection() {
    let (d, s) = square_row(3);
    let p = paint();
    let mut t = LivePaintSelectionTool::default();
    let cx = cx(&d, &s, &p);
    t.pointer(&cx, &ev(PointerKind::Move, 15.0, 10.0));
    assert_eq!(t.hover, LpHover::None);
    assert_eq!(t.cache.builds, 0);
    assert!(!t.waiting());
}

#[test]
fn live_paint_selection_selects_faces() {
    let (d, _gid, f2, e) = live_group_doc();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = LivePaintSelectionTool::default();
    let acts = t.pointer(&cx, &ev(PointerKind::Down, 75.0, 25.0));
    assert_eq!(acts, vec![Action::Exec("select.set".into(), json!({ "ids": [f2.0] }))]);
    let alt = Mods { alt: true, ..Default::default() };
    let acts = t.pointer(&cx, &ev(PointerKind::Down, 51.0, 25.0).with_mods(alt));
    assert_eq!(acts, vec![Action::Exec("select.set".into(), json!({ "ids": [e.0] }))]);
    let acts = t.pointer(&cx, &ev(PointerKind::Down, 300.0, 300.0));
    assert_eq!(acts, vec![Action::Exec("select.none".into(), json!({}))]);
}

#[test]
fn live_paint_selection_options_choose_what_a_click_selects() {
    let (d, _gid, f2, e) = live_group_doc();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let alt = Mods { alt: true, ..Default::default() };
    let set = |ids: &[NodeId]| vec![Action::Exec("select.set".into(), json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))];
    let none = vec![Action::Exec("select.none".into(), json!({}))];
    let mut t = LivePaintSelectionTool::default();
    assert_eq!(t.options(), json!({"selectFills": true, "selectStrokes": true, "color": "#ff2a2a", "width": 4.0}));
    // Select Fills off: a click inside a face selects nothing, and a click by an edge selects it.
    t.set_option("selectFills", &json!(false));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 75.0, 25.0)), none);
    t.pointer(&cx, &ev(PointerKind::Move, 75.0, 25.0));
    assert!(t.overlays(&cx).is_empty());
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 51.0, 25.0)), set(&[e]));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 51.0, 25.0).with_mods(alt)), set(&[e]));
    // Select Strokes off: Alt-click by an edge selects the face under the pointer.
    t.set_option("selectFills", &json!(true));
    t.set_option("selectStrokes", &json!(false));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 51.0, 25.0).with_mods(alt)), set(&[f2]));
    t.pointer(&cx, &ev(PointerKind::Move, 51.0, 25.0).with_mods(alt));
    let ov = t.overlays(&cx);
    let [Overlay::Path { path, .. }] = ov.as_slice() else { panic!("{ov:?}") };
    assert_eq!(path.bounding_box(), Rect::new(50.0, 0.0, 100.0, 50.0), "the face, not the edge");
    // Both off: nothing is selected or highlighted.
    t.set_option("selectFills", &json!(false));
    for m in [Mods::default(), alt] {
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 51.0, 25.0).with_mods(m)), none);
        t.pointer(&cx, &ev(PointerKind::Move, 51.0, 25.0).with_mods(m));
        assert!(t.overlays(&cx).is_empty());
    }
    let shift = Mods { shift: true, ..Default::default() };
    assert!(t.pointer(&cx, &ev(PointerKind::Down, 75.0, 25.0).with_mods(shift)).is_empty());
}

#[test]
fn live_paint_selection_click_outside_the_faces_selects_the_edge_it_highlights() {
    let mut d = Document::new(500.0, 500.0);
    let (_, [_, _, right, _]) = live_square(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0));
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let alt = Mods { alt: true, ..Default::default() };
    let mut t = LivePaintSelectionTool::default();
    // Alt held, and Select Fills off without Alt: 2 pt right of the square, the pointer
    // highlights the square's right side, and a click selects that side (Shift-click adds it).
    for (fills, m) in [(true, alt), (false, Mods::default())] {
        t.set_option("selectFills", &json!(fills));
        t.pointer(&cx, &ev(PointerKind::Move, 102.0, 50.0).with_mods(m));
        let ov = t.overlays(&cx);
        let [Overlay::Path { path, .. }] = ov.as_slice() else { panic!("{ov:?}") };
        assert_eq!(path.bounding_box(), Rect::new(100.0, 0.0, 100.0, 100.0));
        let acts = t.pointer(&cx, &ev(PointerKind::Down, 102.0, 50.0).with_mods(m));
        assert_eq!(acts, vec![Action::Exec("select.set".into(), json!({ "ids": [right.0] }))]);
        let acts = t.pointer(&cx, &ev(PointerKind::Down, 102.0, 50.0).with_mods(Mods { shift: true, ..m }));
        assert_eq!(acts, vec![Action::Exec("select.toggle".into(), json!({ "id": right.0 }))]);
    }
    // Farther than 4 pixels from every edge: no highlight, and a click selects nothing.
    t.pointer(&cx, &ev(PointerKind::Move, 106.0, 50.0));
    assert!(t.overlays(&cx).is_empty());
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 106.0, 50.0)), vec![Action::Exec("select.none".into(), json!({}))]);
}

#[test]
fn live_paint_edges_outside_the_faces_come_from_the_nearest_visible_group() {
    let mut d = Document::new(500.0, 500.0);
    let (back, [_, _, back_right, _]) = live_square(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0));
    let (front, [front_left, ..]) = live_square(&mut d, Rect::new(104.0, 0.0, 204.0, 100.0));
    let s = Selection::default();
    let p = paint();
    let alt = Mods { alt: true, ..Default::default() };
    let shift = Mods { shift: true, ..Default::default() };
    // 1.5 pt from the back group's right side and 2.5 pt from the front group's left side.
    let (x, y) = (101.5, 50.0);
    let edge_x = |ov: &[Overlay]| match ov {
        [] => None,
        [Overlay::Path { path, .. }] => Some(path.bounding_box().x0),
        _ => panic!("{ov:?}"),
    };
    // Live Paint Selection (Alt) and Live Paint Bucket (Shift) highlight and pick the same edge.
    let check = |d: &Document, want: Option<(NodeId, NodeId, f64)>| {
        let cx = cx(d, &s, &p);
        let mut sel = LivePaintSelectionTool::default();
        sel.pointer(&cx, &ev(PointerKind::Move, x, y).with_mods(alt));
        assert_eq!(edge_x(&sel.overlays(&cx)), want.map(|w| w.2));
        let picked = match want {
            Some((_, e, _)) => Action::Exec("select.set".into(), json!({ "ids": [e.0] })),
            None => Action::Exec("select.none".into(), json!({})),
        };
        assert_eq!(sel.pointer(&cx, &ev(PointerKind::Down, x, y).with_mods(alt)), vec![picked]);
        let mut bucket = LivePaintBucketTool::default();
        bucket.pointer(&cx, &ev(PointerKind::Move, x, y).with_mods(shift));
        assert_eq!(edge_x(&bucket.overlays(&cx)), want.map(|w| w.2));
        let stroked = want.map(|(g, ..)| Action::Exec("livePaint.strokeEdge".into(), json!({ "group": g.0, "point": [x, y], "tolerance": 4.0 })));
        assert_eq!(bucket.pointer(&cx, &ev(PointerKind::Down, x, y).with_mods(shift)), stroked.into_iter().collect::<Vec<_>>());
    };
    check(&d, Some((back, back_right, 100.0)));
    // A hidden group's edges are not highlighted or picked.
    d.node_mut(back).unwrap().visible = false;
    check(&d, Some((front, front_left, 104.0)));
    d.node_mut(front).unwrap().visible = false;
    check(&d, None);
}

#[test]
fn live_paint_tools_skip_hidden_edges() {
    let mut d = Document::new(500.0, 500.0);
    let (_, [_, _, back_right, _]) = live_square(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0));
    let (front, [front_left, ..]) = live_square(&mut d, Rect::new(104.0, 0.0, 204.0, 100.0));
    // The back group's right side is hidden on its own; both groups stay visible.
    d.node_mut(back_right).unwrap().visible = false;
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let alt = Mods { alt: true, ..Default::default() };
    let shift = Mods { shift: true, ..Default::default() };
    let edge_x = |ov: &[Overlay]| match ov {
        [] => None,
        [Overlay::Path { path, .. }] => Some(path.bounding_box().x0),
        _ => panic!("{ov:?}"),
    };
    // Outside the faces, 1.5 pt from the hidden side and 2.5 pt from the front group's left
    // side, both tools highlight and pick the visible side. Inside the back face, 1.5 pt from
    // the hidden side, no edge is in reach, so neither tool highlights or picks one.
    for (x, want) in [(101.5, Some((front, front_left, 104.0))), (98.5, None)] {
        let mut sel = LivePaintSelectionTool::default();
        sel.pointer(&cx, &ev(PointerKind::Move, x, 50.0).with_mods(alt));
        assert_eq!(edge_x(&sel.overlays(&cx)), want.map(|w| w.2), "x {x}");
        let picked = match want {
            Some((_, e, _)) => Action::Exec("select.set".into(), json!({ "ids": [e.0] })),
            None => Action::Exec("select.none".into(), json!({})),
        };
        assert_eq!(sel.pointer(&cx, &ev(PointerKind::Down, x, 50.0).with_mods(alt)), vec![picked], "x {x}");
        let mut bucket = LivePaintBucketTool::default();
        bucket.pointer(&cx, &ev(PointerKind::Move, x, 50.0).with_mods(shift));
        assert_eq!(edge_x(&bucket.overlays(&cx)), want.map(|w| w.2), "x {x}");
        let stroked = want.map(|(g, ..)| Action::Exec("livePaint.strokeEdge".into(), json!({ "group": g.0, "point": [x, 50.0], "tolerance": 4.0 })));
        assert_eq!(bucket.pointer(&cx, &ev(PointerKind::Down, x, 50.0).with_mods(shift)), stroked.into_iter().collect::<Vec<_>>(), "x {x}");
    }
}

#[test]
fn live_paint_tools_skip_hidden_faces() {
    let mut d = Document::new(500.0, 500.0);
    let (back, [_, _, back_right, _]) = live_square(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0));
    let (front, _) = live_square(&mut d, Rect::new(50.0, 0.0, 150.0, 100.0));
    let face = |g: NodeId| faces(d.node(g).unwrap())[0].id;
    let (back_face, hidden) = (face(back), face(front));
    // The front group's face is hidden on its own; both groups stay visible.
    d.node_mut(hidden).unwrap().visible = false;
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let alt = Mods { alt: true, ..Default::default() };
    let part = |ov: &[Overlay]| match ov {
        [] => None,
        [Overlay::Path { path, .. }] => Some(path.bounding_box()),
        _ => panic!("{ov:?}"),
    };
    // Where the faces overlap, both tools take the back group's face under the hidden one.
    let mut bucket = LivePaintBucketTool::default();
    bucket.pointer(&cx, &ev(PointerKind::Move, 75.0, 50.0));
    assert_eq!(part(&bucket.overlays(&cx)), Some(Rect::new(0.0, 0.0, 100.0, 100.0)));
    let fill = Action::Exec("livePaint.fill".into(), json!({ "group": back.0, "point": [75.0, 50.0] }));
    assert_eq!(bucket.pointer(&cx, &ev(PointerKind::Down, 75.0, 50.0)), vec![fill]);
    let mut sel = LivePaintSelectionTool::default();
    sel.pointer(&cx, &ev(PointerKind::Move, 75.0, 50.0));
    assert_eq!(part(&sel.overlays(&cx)), Some(Rect::new(0.0, 0.0, 100.0, 100.0)));
    assert_eq!(sel.pointer(&cx, &ev(PointerKind::Down, 75.0, 50.0)), vec![Action::Exec("select.set".into(), json!({ "ids": [back_face.0] }))]);
    // Over the hidden face alone, neither tool highlights or picks a face.
    bucket.pointer(&cx, &ev(PointerKind::Move, 125.0, 50.0));
    assert_eq!(part(&bucket.overlays(&cx)), None);
    assert!(bucket.pointer(&cx, &ev(PointerKind::Down, 125.0, 50.0)).is_empty());
    sel.pointer(&cx, &ev(PointerKind::Move, 125.0, 50.0));
    assert_eq!(part(&sel.overlays(&cx)), None);
    assert_eq!(sel.pointer(&cx, &ev(PointerKind::Down, 125.0, 50.0)), vec![Action::Exec("select.none".into(), json!({}))]);
    // Inside the hidden face, 1.5 pt from the back group's right side, Alt-click takes that side
    // as it does outside every face.
    sel.pointer(&cx, &ev(PointerKind::Move, 101.5, 50.0).with_mods(alt));
    assert_eq!(part(&sel.overlays(&cx)), Some(Rect::new(100.0, 0.0, 100.0, 100.0)));
    let picked = sel.pointer(&cx, &ev(PointerKind::Down, 101.5, 50.0).with_mods(alt));
    assert_eq!(picked, vec![Action::Exec("select.set".into(), json!({ "ids": [back_right.0] }))]);
}

#[test]
fn live_paint_selection_options_set_the_highlight() {
    let (d, ..) = live_group_doc();
    let s = Selection::default();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = LivePaintSelectionTool::default();
    let highlight = |t: &mut LivePaintSelectionTool| {
        t.pointer(&cx, &ev(PointerKind::Move, 75.0, 25.0));
        match t.overlays(&cx).as_slice() {
            [Overlay::Path { color, width, .. }] => (*color, *width),
            ov => panic!("{ov:?}"),
        }
    };
    assert_eq!(highlight(&mut t), (HIGHLIGHT_RED, 4.0));
    t.set_option("color", &json!("#00FF00"));
    t.set_option("width", &json!(9));
    assert_eq!(highlight(&mut t), ([0, 0xff, 0], 9.0));
    // A layer color's name; a value that is not a color, or a width that is not a number, is ignored.
    t.set_option("color", &json!("light blue"));
    t.set_option("width", &json!("wide"));
    assert_eq!(highlight(&mut t), ([0x4f, 0x80, 0xff], 9.0));
    t.set_option("color", &json!("nope"));
    assert_eq!(t.options()["color"], "#4f80ff");
    // The width stays within its range.
    t.set_option("width", &json!(100));
    assert_eq!(t.options()["width"], json!(LP_SELECT_WIDTH_RANGE.1));
    t.set_option("width", &json!(0));
    assert_eq!(t.options()["width"], json!(LP_SELECT_WIDTH_RANGE.0));
}

#[test]
fn create_builds_the_three_tools() {
    for id in ["shapeBuilder", "livePaintBucket", "livePaintSelection"] {
        assert_eq!(create(id).unwrap().id(), id);
    }
    assert!(create("pen").is_none());
}
