//! Tests for `addon.cutting.weed`: a weed box around the selected art and weeding lines across it
//! that stop at the art's outlines.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{Appearance, Node, NodeKind};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, Shape, shapes};

use super::super::super::super::pathops::text_outline;
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

fn line(s: &mut Session, pts: &[(f64, f64)]) -> NodeId {
    let anchors: Vec<Value> = pts.iter().map(|(x, y)| json!({"x": x, "y": y})).collect();
    id_of(&s.execute("path.create", &json!({"anchors": anchors, "closed": false})).unwrap())
}

fn select(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

/// Runs Weed Box and Lines with `p` and gives the group it made.
fn weed(s: &mut Session, p: Value) -> (Value, Node) {
    let r = s.execute("addon.cutting.weed", &p).unwrap();
    let g = node(s, id_of(&r));
    (r, g)
}

/// The weed box's rectangle in group `g`, if it has one.
fn weed_box(g: &Node) -> Option<Rect> {
    g.children().unwrap().iter().find(|c| c.name.as_deref() == Some("Weed Box")).map(|c| c.geometric_bounds().unwrap())
}

/// The weeding lines in group `g` as their two ends, rounded to 1/1000 pt.
fn lines(g: &Node) -> Vec<((f64, f64), (f64, f64))> {
    let r = |v: f64| (v * 1000.0).round() / 1000.0;
    g.children()
        .unwrap()
        .iter()
        .filter(|c| c.name.is_none())
        .map(|c| match &c.kind {
            NodeKind::Path { path, .. } => {
                let a: Vec<Point> = path.anchors().map(|(_, _, a)| a.p).collect();
                assert_eq!(a.len(), 2, "a weeding line is one straight segment");
                ((r(a[0].x), r(a[0].y)), (r(a[1].x), r(a[1].y)))
            }
            other => panic!("not a path: {other:?}"),
        })
        .collect()
}

#[test]
fn a_weed_box_and_lines_go_around_a_rectangle() {
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 50.0, 40.0);
    let undo = s.doc().unwrap().history.undo.len();
    let (r, g) = weed(&mut s, json!({"margin": 10}));
    assert_eq!(g.name.as_deref(), Some("Weeding"));
    assert_eq!(weed_box(&g), Some(Rect::new(90.0, 90.0, 160.0, 150.0)));
    // One line each way through the middle, left out over the art.
    assert_eq!(
        lines(&g),
        [((90.0, 120.0), (100.0, 120.0)), ((150.0, 120.0), (160.0, 120.0)), ((125.0, 90.0), (125.0, 100.0)), ((125.0, 140.0), (125.0, 150.0))]
    );
    assert_eq!(r["lines"], json!(4));
    assert_eq!(r["box"], json!(weed_box_id(&g)));
    // Stroked in Registration with no fill, selected, as one undo step that takes it all away.
    for c in g.children().unwrap() {
        assert_eq!(c.appearance.fill_paint(), Paint::None, "no fill");
        assert!(c.appearance.stroke_paint().is_registration());
    }
    assert_eq!(s.doc().unwrap().selection.objects, [g.id]);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(g.id).is_none());
}

fn weed_box_id(g: &Node) -> u64 {
    g.children().unwrap().iter().find(|c| c.name.as_deref() == Some("Weed Box")).unwrap().id.0
}

#[test]
fn lines_stop_at_the_outline_and_cross_the_hole_of_a_compound_path() {
    let mut s = session();
    let outer = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let inner = rect(&mut s, 130.0, 130.0, 40.0, 40.0);
    select(&mut s, &[outer, inner]);
    let c = id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap());
    // Both subpaths wind the same way, so the fill rule alone decides whether the inner one is a
    // hole.
    let same_way = |rule: FillRule| {
        move |d: &mut vectorcraft_doc::Document, _: &mut vectorcraft_doc::Selection| -> Result<()> {
            let Some(NodeKind::Compound { children, rule: r }) = d.node_mut(c).map(|n| &mut n.kind) else { panic!("not a compound path") };
            *r = rule;
            let area = |n: &Node| match &n.kind {
                NodeKind::Path { path, .. } => path.subpaths[0].signed_area(),
                _ => panic!("not a path"),
            };
            let outer = area(&children[0]).signum();
            for child in children.iter_mut() {
                if area(child).signum() != outer
                    && let NodeKind::Path { path, .. } = &mut std::sync::Arc::make_mut(child).kind
                {
                    path.reverse();
                }
            }
            Ok(())
        }
    };
    s.edit("even-odd", same_way(FillRule::EvenOdd)).unwrap();
    select(&mut s, &[c]);
    let (_, g) = weed(&mut s, json!({"margin": 10, "direction": "horizontal"}));
    // The waste in the hole is cut too; the ring itself is not.
    assert_eq!(lines(&g), [((90.0, 150.0), (100.0, 150.0)), ((130.0, 150.0), (170.0, 150.0)), ((200.0, 150.0), (210.0, 150.0))]);
    // With the non-zero rule the inner subpath fills, and no line crosses it.
    s.edit("non-zero", same_way(FillRule::NonZero)).unwrap();
    select(&mut s, &[c]);
    let (_, g) = weed(&mut s, json!({"margin": 10, "direction": "horizontal"}));
    assert_eq!(lines(&g), [((90.0, 150.0), (100.0, 150.0)), ((200.0, 150.0), (210.0, 150.0))]);
}

#[test]
fn a_gap_keeps_the_lines_away_from_the_outline() {
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 50.0, 40.0);
    let (_, g) = weed(&mut s, json!({"margin": 20, "gap": 5, "direction": "horizontal"}));
    assert_eq!(weed_box(&g), Some(Rect::new(80.0, 80.0, 170.0, 160.0)));
    assert_eq!(lines(&g), [((80.0, 120.0), (95.0, 120.0)), ((155.0, 120.0), (170.0, 120.0))]);
    // The gap is a distance from the outline: a line passing 5 pt beside a corner, with a gap of
    // 6, stops √11 pt short of the corner's height.
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 50.0, 40.0);
    let b = rect(&mut s, 200.0, 60.0, 10.0, 120.0);
    select(&mut s, &[a, b]);
    let (_, g) = weed(&mut s, json!({"margin": 0, "gap": 6, "direction": "vertical"}));
    assert_eq!(weed_box(&g), Some(Rect::new(100.0, 60.0, 210.0, 180.0)));
    assert_eq!(lines(&g), [((155.0, 60.0), (155.0, 96.683)), ((155.0, 143.317), (155.0, 180.0))]);
}

#[test]
fn a_box_with_no_height_gets_no_lines_across_it() {
    // A horizontal line with no margin: the box is as flat as the line, and no weeding line runs
    // along it.
    let mut s = session();
    let l = line(&mut s, &[(100.0, 150.0), (200.0, 150.0)]);
    let n = s.doc().unwrap().doc.node_count();
    assert!(s.execute("addon.cutting.weed", &json!({"margin": 0, "direction": "horizontal", "box": false})).is_err());
    assert_eq!(s.doc().unwrap().doc.node_count(), n, "nothing to add");
    select(&mut s, &[l]);
    let (_, g) = weed(&mut s, json!({"margin": 0, "direction": "horizontal"}));
    assert_eq!(weed_box(&g), Some(Rect::new(100.0, 150.0, 200.0, 150.0)));
    assert_eq!(lines(&g), []);
}

#[test]
fn an_open_path_stops_a_line_where_it_crosses() {
    let mut s = session();
    line(&mut s, &[(100.0, 100.0), (100.0, 200.0)]);
    let (_, g) = weed(&mut s, json!({"margin": 10, "direction": "horizontal", "gap": 2}));
    assert_eq!(lines(&g), [((90.0, 150.0), (98.0, 150.0)), ((102.0, 150.0), (110.0, 150.0))]);
}

#[test]
fn type_is_measured_by_its_glyph_outlines() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "OHO", "size": 72})).unwrap());
    let glyphs = match &node(&s, t).kind {
        NodeKind::Text(t) => text_outline(t),
        _ => panic!("not type"),
    };
    let ink = glyphs.bounds().unwrap();
    let (_, g) = weed(&mut s, json!({"margin": 10, "count": 3, "direction": "horizontal"}));
    let b = weed_box(&g).unwrap();
    for (got, want) in [(b.x0, ink.x0 - 10.0), (b.y0, ink.y0 - 10.0), (b.x1, ink.x1 + 10.0), (b.y1, ink.y1 + 10.0)] {
        assert!((got - want).abs() < 0.05, "the box is 10 pt around the glyphs: {b:?} vs {ink:?}");
    }
    // No piece of line runs over a glyph: each piece's middle is outside the ink.
    let shape = glyphs.to_bezpath();
    let pieces = lines(&g);
    assert!(pieces.len() > 3, "the letters break the lines: {pieces:?}");
    for ((x0, y0), (x1, y1)) in pieces {
        let mid = Point::new((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        assert_eq!(shape.winding(mid), 0, "{mid:?} is outside the glyphs");
    }
}

#[test]
fn a_clip_group_is_measured_by_what_shows() {
    let mut s = session();
    let big = rect(&mut s, 0.0, 0.0, 400.0, 300.0);
    let clip = rect(&mut s, 100.0, 100.0, 50.0, 40.0);
    select(&mut s, &[big, clip]);
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    let (_, g) = weed(&mut s, json!({"margin": 10}));
    assert_eq!(weed_box(&g), Some(Rect::new(90.0, 90.0, 160.0, 150.0)));
    assert_eq!(lines(&g).len(), 4);
    // Art that shows only partly: the clip path's edge where the art is, and the art's edge
    // inside the clip.
    let mut s = session();
    let art = rect(&mut s, 120.0, 0.0, 100.0, 300.0);
    let clip = rect(&mut s, 100.0, 100.0, 50.0, 40.0);
    select(&mut s, &[art, clip]);
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    let (_, g) = weed(&mut s, json!({"margin": 10, "direction": "horizontal"}));
    assert_eq!(weed_box(&g), Some(Rect::new(110.0, 90.0, 160.0, 150.0)));
    assert_eq!(lines(&g), [((110.0, 120.0), (120.0, 120.0)), ((150.0, 120.0), (160.0, 120.0))]);
}

#[test]
fn a_group_as_the_clipping_path_clips_by_its_members() {
    // Make Clipping Mask takes a path, a compound path or type; an opened file can clip by a
    // group, as this one does.
    let mut s = session();
    let parent = s.doc().unwrap().target_parent().unwrap();
    let id = s
        .edit("clip group", |d, sel| {
            let mut r = |x: f64, w: f64, y: f64, h: f64| {
                Arc::new(Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, y, x + w, y + h)), Appearance::default()))
            };
            let (a, b, art) = (r(100.0, 50.0, 100.0, 40.0), r(250.0, 50.0, 100.0, 40.0), r(0.0, 400.0, 0.0, 300.0));
            let clip = Arc::new(Node::group(d.alloc_id(), vec![a, b]));
            let mut g = Node::group(d.alloc_id(), vec![clip, art]);
            if let NodeKind::Group { clip, .. } = &mut g.kind {
                *clip = true;
            }
            let id = g.id;
            d.insert(parent, usize::MAX, g)?;
            sel.set([id]);
            Ok(id)
        })
        .unwrap();
    select(&mut s, &[id]);
    let (_, g) = weed(&mut s, json!({"margin": 10, "direction": "horizontal"}));
    // The art shows through both rectangles, and the line is cut between them too.
    assert_eq!(weed_box(&g), Some(Rect::new(90.0, 90.0, 310.0, 150.0)));
    assert_eq!(lines(&g), [((90.0, 120.0), (100.0, 120.0)), ((150.0, 120.0), (250.0, 120.0)), ((300.0, 120.0), (310.0, 120.0))]);
}

#[test]
fn lines_near_too_many_outline_segments_are_refused() {
    let mut s = session();
    // A zigzag of 12,000 segments; with a gap as tall as the box, each of 1,000 lines comes near
    // every segment.
    let mut zigzag = BezPath::new();
    zigzag.move_to((100.0, 100.0));
    for i in 1..=12_000 {
        zigzag.line_to((100.0 + f64::from(i) * 0.01, if i % 2 == 0 { 100.0 } else { 101.0 }));
    }
    let parent = s.doc().unwrap().target_parent().unwrap();
    let id = s
        .edit("zigzag", |d, sel| {
            let n = Node::path(d.alloc_id(), PathData::from_bezpath(&zigzag), Appearance::default());
            let id = n.id;
            d.insert(parent, usize::MAX, n)?;
            sel.set([id]);
            Ok(id)
        })
        .unwrap();
    select(&mut s, &[id]);
    let n = s.doc().unwrap().doc.node_count();
    let e = s.execute("addon.cutting.weed", &json!({"count": 1000, "gap": 50, "direction": "horizontal"})).unwrap_err();
    assert!(e.to_string().contains("too many outline segments"), "{e}");
    assert_eq!(s.doc().unwrap().doc.node_count(), n);
    // Fewer lines measure.
    s.execute("addon.cutting.weed", &json!({"count": 10, "gap": 50, "direction": "horizontal"})).unwrap();
}

/// Adds a clip group, clipping path `clip` over `art` (both unpainted), and selects it.
fn clip_group(s: &mut Session, clip: PathData, art: PathData) -> NodeId {
    let parent = s.doc().unwrap().target_parent().unwrap();
    let id = s
        .edit("clip group", |d, sel| {
            let clip = Arc::new(Node::path(d.alloc_id(), clip, Appearance::default()));
            let art = Arc::new(Node::path(d.alloc_id(), art, Appearance::default()));
            let mut g = Node::group(d.alloc_id(), vec![clip, art]);
            if let NodeKind::Group { clip, .. } = &mut g.kind {
                *clip = true;
            }
            let id = g.id;
            d.insert(parent, usize::MAX, g)?;
            sel.set([id]);
            Ok(id)
        })
        .unwrap();
    select(s, &[id]);
    id
}

#[test]
fn an_open_clipping_path_clips_as_if_closed() {
    let mut s = session();
    let mut v = BezPath::new();
    v.move_to((100.0, 100.0));
    v.line_to((150.0, 200.0));
    v.line_to((200.0, 100.0));
    clip_group(&mut s, PathData::from_bezpath(&v), shapes::rectangle(Rect::new(0.0, 0.0, 400.0, 300.0)));
    let (_, g) = weed(&mut s, json!({"margin": 10, "direction": "horizontal"}));
    // The art shows inside the triangle the V closes; the line stops at its sides.
    assert_eq!(weed_box(&g), Some(Rect::new(90.0, 90.0, 210.0, 210.0)));
    assert_eq!(lines(&g), [((90.0, 150.0), (125.0, 150.0)), ((175.0, 150.0), (210.0, 150.0))]);
}

#[test]
fn art_a_nested_clip_hides_adds_no_outline() {
    // A clip group, clipping path A, holding a clip group whose clipping path B shows a small
    // rectangle and hides a line that runs along A's bottom edge.
    let mut s = session();
    let parent = s.doc().unwrap().target_parent().unwrap();
    let id = s
        .edit("nested clips", |d, sel| {
            let mut path = |p: PathData| Arc::new(Node::path(d.alloc_id(), p, Appearance::default()));
            let mut l = BezPath::new();
            l.move_to((0.0, 200.0));
            l.line_to((400.0, 200.0));
            let (b, shown, hidden) = (
                path(shapes::rectangle(Rect::new(100.0, 100.0, 150.0, 140.0))),
                path(shapes::rectangle(Rect::new(110.0, 110.0, 140.0, 130.0))),
                path(PathData::from_bezpath(&l)),
            );
            let a = path(shapes::rectangle(Rect::new(50.0, 50.0, 300.0, 200.0)));
            let clipped = |mut g: Node| {
                if let NodeKind::Group { clip, .. } = &mut g.kind {
                    *clip = true;
                }
                g
            };
            let inner = Arc::new(clipped(Node::group(d.alloc_id(), vec![b, shown, hidden])));
            let outer = clipped(Node::group(d.alloc_id(), vec![a, inner]));
            let id = outer.id;
            d.insert(parent, usize::MAX, outer)?;
            sel.set([id]);
            Ok(id)
        })
        .unwrap();
    select(&mut s, &[id]);
    let (_, g) = weed(&mut s, json!({"margin": 10, "count": 0}));
    assert_eq!(weed_box(&g), Some(Rect::new(100.0, 100.0, 150.0, 140.0)), "only the shown rectangle is measured");
}

#[test]
fn art_a_nested_clip_hides_counts_toward_the_clip_limit() {
    // A clip group, clipping path A of 20,000 segments, holding a clip group whose small clipping
    // path B shows a rectangle and hides a closed zigzag C of 20,000 points across every row of A.
    // The inner group shows four outline pieces, but testing each piece of A for being inside it
    // looks at all of C.
    let mut s = session();
    let parent = s.doc().unwrap().target_parent().unwrap();
    let id = s
        .edit("nested clips", |d, sel| {
            let mut path = |p: BezPath| Arc::new(Node::path(d.alloc_id(), PathData::from_bezpath(&p), Appearance::default()));
            let mut a = BezPath::new();
            a.move_to((0.0, 0.0));
            for i in 1..=20_000 {
                a.line_to((f64::from(i) * 0.02, if i % 2 == 0 { 0.0 } else { 0.5 }));
            }
            a.line_to((400.0, 300.0));
            a.line_to((0.0, 300.0));
            a.close_path();
            let mut c = BezPath::new();
            c.move_to((300.0, 0.0));
            for i in 1..=20_000 {
                c.line_to((300.0 + f64::from(i) * 0.002, if i % 2 == 0 { 0.0 } else { 300.0 }));
            }
            c.close_path();
            let b = path(shapes::rectangle(Rect::new(100.0, 100.0, 150.0, 140.0)).to_bezpath());
            let shown = path(shapes::rectangle(Rect::new(110.0, 110.0, 140.0, 130.0)).to_bezpath());
            let (a, c) = (path(a), path(c));
            let clipped = |mut g: Node| {
                if let NodeKind::Group { clip, .. } = &mut g.kind {
                    *clip = true;
                }
                g
            };
            let inner = Arc::new(clipped(Node::group(d.alloc_id(), vec![b, shown, c])));
            let outer = clipped(Node::group(d.alloc_id(), vec![a, inner]));
            let id = outer.id;
            d.insert(parent, usize::MAX, outer)?;
            sel.set([id]);
            Ok(id)
        })
        .unwrap();
    select(&mut s, &[id]);
    let n = s.doc().unwrap().doc.node_count();
    let e = s.execute("addon.cutting.weed", &json!({"margin": 10, "count": 0})).unwrap_err();
    assert!(e.to_string().contains("too many outline segments"), "{e}");
    assert_eq!(s.doc().unwrap().doc.node_count(), n);
}

#[test]
fn art_selected_inside_a_clip_group_is_measured_by_what_shows() {
    // The rectangle runs far outside the clipping path; selected on its own, it still shows only
    // inside it.
    let mut s = session();
    let clip = shapes::rectangle(Rect::new(100.0, 100.0, 150.0, 140.0));
    let g = clip_group(&mut s, clip, shapes::rectangle(Rect::new(120.0, 0.0, 220.0, 300.0)));
    let art = node(&s, g).children().unwrap()[1].id;
    select(&mut s, &[art]);
    let (_, w) = weed(&mut s, json!({"margin": 10, "direction": "horizontal"}));
    assert_eq!(weed_box(&w), Some(Rect::new(110.0, 90.0, 160.0, 150.0)));
    assert_eq!(lines(&w), [((110.0, 120.0), (120.0, 120.0)), ((150.0, 120.0), (160.0, 120.0))]);
    // The clipping path selected on its own is measured by the art it shows.
    let clip_path = node(&s, g).children().unwrap()[0].id;
    select(&mut s, &[clip_path]);
    let (_, w) = weed(&mut s, json!({"margin": 10, "direction": "horizontal"}));
    assert_eq!(weed_box(&w), Some(Rect::new(110.0, 90.0, 160.0, 150.0)));
    // With the art selected too, or the group, the group is measured once.
    let d = &s.doc().unwrap().doc;
    let outline = |ids: &[NodeId]| super::geometry::Reader::new(d).read_ids(ids).unwrap().unwrap().outline().unwrap().len();
    assert_eq!(outline(&[clip_path, art]), outline(&[g]));
    assert_eq!(outline(&[g, art, g]), outline(&[g]));
}

#[test]
fn art_in_a_hidden_group_is_left_out() {
    let mut s = session();
    let shown = rect(&mut s, 300.0, 300.0, 20.0, 20.0);
    let a = rect(&mut s, 100.0, 100.0, 50.0, 40.0);
    let b = rect(&mut s, 200.0, 100.0, 50.0, 40.0);
    select(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.edit("hide", |d, _| {
        if let Some(n) = d.node_mut(g) {
            n.visible = false;
        }
        Ok(())
    })
    .unwrap();
    select(&mut s, &[a]);
    assert!(s.execute("addon.cutting.weed", &json!({})).is_err(), "nothing selected shows");
    select(&mut s, &[a, shown]);
    let (_, w) = weed(&mut s, json!({"margin": 10}));
    assert_eq!(weed_box(&w), Some(Rect::new(290.0, 290.0, 330.0, 330.0)));
}

#[test]
fn art_in_a_hidden_group_is_not_measured() {
    // Selected art in a hidden group with more outline points than the limit doesn't count toward
    // it.
    let mut s = session();
    let shown = rect(&mut s, 300.0, 300.0, 20.0, 20.0);
    let mut p = BezPath::new();
    p.move_to((100.0, 100.0));
    for i in 1..=super::geometry::MAX_POINTS as u32 {
        p.line_to((100.0 + f64::from(i) * 0.001, if i % 2 == 0 { 100.0 } else { 101.0 }));
    }
    let dense = insert(&mut s, |id| Node::path(id, PathData::from_bezpath(&p), Appearance::default()));
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.edit("hide", |d, _| {
        if let Some(n) = d.node_mut(g) {
            n.visible = false;
        }
        Ok(())
    })
    .unwrap();
    select(&mut s, &[dense, shown]);
    let (_, w) = weed(&mut s, json!({"margin": 10}));
    assert_eq!(weed_box(&w), Some(Rect::new(290.0, 290.0, 330.0, 330.0)));
}

#[test]
fn clip_outlines_crossing_too_often_are_refused() {
    // A clipping path whose 700 edges each run across the art, and open art whose 700 segments
    // each run down across the clipping path: every pair crosses, and each of the 490,000 pieces
    // of art between crossings would be tested against all 700 edges.
    let zigzag = |across: bool| {
        let mut p = BezPath::new();
        for i in 0..=700 {
            let (along, side) = (100.0 + f64::from(i) * 0.25, if i % 2 == 0 { 100.0 } else { 300.0 });
            let pt = if across { (side, along) } else { (along, side) };
            if i == 0 { p.move_to(pt) } else { p.line_to(pt) }
        }
        p
    };
    let mut clip = zigzag(true);
    clip.close_path();
    let mut s = session();
    clip_group(&mut s, PathData::from_bezpath(&clip), PathData::from_bezpath(&zigzag(false)));
    let n = s.doc().unwrap().doc.node_count();
    let e = s.execute("addon.cutting.weed", &json!({})).unwrap_err();
    assert!(e.to_string().contains("too many outline segments"), "{e}");
    assert_eq!(s.doc().unwrap().doc.node_count(), n);
}

/// Each weeding line's stretches inside the art, found from the crossings indexed once for all the
/// lines, match those found by measuring the whole art along that line.
#[test]
fn indexed_crossings_find_the_same_stretches() {
    let mut s = session();
    let mut ids = vec![];
    let outer = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let inner = rect(&mut s, 130.0, 130.0, 40.0, 40.0);
    select(&mut s, &[outer, inner]);
    ids.push(id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap()));
    let big = rect(&mut s, 220.0, 50.0, 200.0, 200.0);
    let window = id_of(&s.execute("shape.ellipse", &json!({"x": 260, "y": 80, "width": 90, "height": 120})).unwrap());
    select(&mut s, &[big, window]);
    ids.push(id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap()));
    ids.push(id_of(&s.execute("text.create", &json!({"x": 100, "y": 320, "text": "WEED 8", "size": 60})).unwrap()));
    ids.push(line(&mut s, &[(90.0, 40.0), (450.0, 380.0)]));
    let a = rect(&mut s, 300.0, 300.0, 30.0, 30.0);
    let b = id_of(&s.execute("shape.ellipse", &json!({"x": 320, "y": 310, "width": 60, "height": 40})).unwrap());
    select(&mut s, &[a, b]);
    ids.push(id_of(&s.execute("object.group", &json!({})).unwrap()));
    let d = &s.doc().unwrap().doc;
    let nodes: Vec<&Node> = ids.iter().filter_map(|id| d.node(*id)).collect();
    let region = super::geometry::Reader::new(d).read(&nodes).unwrap().unwrap();
    let (layout, _) = super::geometry::Layout::of(&region);
    let count = 97;
    for vertical in [false, true] {
        let (v0, v1) = (30.0, 400.0);
        let step = (v1 - v0) / (count as f64 + 1.0);
        let crossed = super::geometry::crossings(&region, vertical, v0, step, count).unwrap();
        assert_eq!(crossed.len(), count);
        let mut hit = 0;
        for (k, line) in crossed.iter().enumerate() {
            let c = v0 + step * (k as f64 + 1.0);
            let want = region.spans(vertical, c);
            hit += usize::from(!want.is_empty());
            assert_eq!(region.spans_on(&layout, line, 0), want, "line {k}, vertical {vertical}");
        }
        assert!(hit > count / 2, "most lines cross the art ({hit})");
    }
}

#[test]
fn art_nested_too_deep_is_refused() {
    // A rectangle inside `levels` groups, each holding the next. A document keeps none this deep,
    // but the art symbol instances and live objects draw nests further.
    let doc = vectorcraft_doc::Document::new(800.0, 600.0);
    let nested = |levels: usize| {
        let mut n = Node::path(NodeId(1), shapes::rectangle(Rect::new(100.0, 100.0, 150.0, 140.0)), Appearance::default());
        for i in 0..levels {
            n = Node::group(NodeId(i as u64 + 2), vec![Arc::new(n)]);
        }
        n
    };
    let read = |n: &Node| super::geometry::Reader::new(&doc).read(&[n]);
    assert!(read(&nested(super::geometry::MAX_DEPTH)).unwrap().is_some());
    let e = read(&nested(super::geometry::MAX_DEPTH + 1)).unwrap_err();
    assert!(e.contains("nested more than"), "{e}");
}

#[test]
fn spacing_divides_the_box_into_strips_no_wider() {
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 10.0, 40.0);
    // The box is 60 pt tall: three strips of 20 pt.
    let (_, g) = weed(&mut s, json!({"margin": 10, "spacing": 25, "direction": "horizontal", "box": false}));
    assert_eq!(weed_box(&g), None);
    let ys: Vec<f64> = lines(&g).iter().map(|l| l.0.1).collect();
    assert_eq!(ys, [110.0, 110.0, 130.0, 130.0]);
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 10.0, 40.0);
    assert!(s.execute("addon.cutting.weed", &json!({"spacing": 0.01})).is_err(), "too many lines");
}

#[test]
fn the_lines_take_a_swatch() {
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Cut", "color": "cmyk 0 100 0 0", "spot": true})).unwrap();
    rect(&mut s, 100.0, 100.0, 50.0, 40.0);
    let (_, g) = weed(&mut s, json!({"swatch": "Cut", "weight": 1}));
    for c in g.children().unwrap() {
        match c.appearance.stroke_paint() {
            Paint::Solid { swatch, .. } => assert_eq!(swatch.as_deref(), Some("Cut")),
            other => panic!("stroke {other:?}"),
        }
        assert_eq!(c.appearance.stroke_width(), 1.0);
    }
}

/// Adds the object `make` makes with a new id to the current layer, selects it and gives its id.
fn insert(s: &mut Session, make: impl FnOnce(NodeId) -> Node) -> NodeId {
    let parent = s.doc().unwrap().target_parent().unwrap();
    let id = s
        .edit("insert", |d, _| {
            let n = make(d.alloc_id());
            let id = n.id;
            d.insert(parent, usize::MAX, n)?;
            Ok(id)
        })
        .unwrap();
    select(s, &[id]);
    id
}

/// A 100 pt square turned 45° about (300, 300): a diamond 141.42 pt across.
fn diamond_xf() -> Affine {
    Affine::translate((300.0, 300.0)) * Affine::rotate(std::f64::consts::FRAC_PI_4) * Affine::translate((-50.0, -50.0))
}

/// The weeding lines of `g` at height `y` (to 1/1000 pt).
fn at(g: &Node, y: f64) -> Vec<(f64, f64)> {
    lines(g).into_iter().filter(|((_, y0), (_, y1))| *y0 == y && *y1 == y).map(|((x0, _), (x1, _))| (x0, x1)).collect()
}

/// The diamond's weeding with three horizontal lines and a margin of 10: the box, and where the
/// first line's left piece ends.
fn diamond_weeding(s: &mut Session) -> (Rect, f64) {
    let (_, g) = weed(s, json!({"margin": 10, "count": 3, "direction": "horizontal"}));
    let b = weed_box(&g).unwrap();
    let y = ((b.y0 + b.height() / 4.0) * 1000.0).round() / 1000.0;
    let first = at(&g, y);
    assert_eq!(first.len(), 2, "{first:?}");
    (b, first[0].1)
}

#[test]
fn an_image_is_measured_by_its_frame_as_turned() {
    let mut s = session();
    let im = vectorcraft_doc::ImageObject {
        key: "image".into(),
        width: 100,
        height: 100,
        xf: diamond_xf(),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    insert(&mut s, |id| Node::new(id, NodeKind::Image(Box::new(im))));
    let (b, end) = diamond_weeding(&mut s);
    let half = 50.0 * std::f64::consts::SQRT_2;
    assert!((b.x0 - (290.0 - half)).abs() < 1e-3 && (b.y1 - (310.0 + half)).abs() < 1e-3, "{b:?}");
    // A quarter of the box down, the line meets the diamond's upper left edge, not its bounds.
    let y = b.y0 + b.height() / 4.0;
    assert!((end - (300.0 - (y - (300.0 - half)))).abs() < 0.01, "{end} at {y}");
}

#[test]
fn a_placed_file_is_measured_by_the_art_it_shows() {
    let dir = crate::tests_links::Folder::new("weed-placed");
    // A file with a 60 × 40 artboard and a 20 pt wide rectangle at each side (hidden when
    // `hidden`), placed centered on (200, 150) and selected.
    let place = |s: &mut Session, name: &str, hidden: bool| {
        let src = dir.file(name);
        let mut d = vectorcraft_doc::Document::new(60.0, 40.0);
        let layer = d.layers[0].id;
        for x in [0.0, 40.0] {
            let id = d.alloc_id();
            let look = Appearance::basic(Paint::solid(vectorcraft_color::Color::BLACK), Paint::None, 0.0);
            let mut n = Node::path(id, shapes::rectangle(Rect::new(x, 0.0, x + 20.0, 40.0)), look);
            n.visible = !hidden;
            d.insert(Some(layer), 0, n).unwrap();
        }
        crate::tests_links::write(&src, &vectorcraft_format::save_file(&d));
        let r = s.execute("file.place", &json!({"path": src, "at": [200, 150]})).unwrap();
        select(s, &[NodeId(r["ids"][0].as_u64().unwrap())]);
    };
    let mut s = session();
    place(&mut s, "pair.vectorcraft", false);
    let (_, g) = weed(&mut s, json!({"margin": 10, "direction": "horizontal"}));
    // The rectangles lie at 170..190 and 210..230; the line runs between them too.
    assert_eq!(at(&g, 150.0), [(160.0, 170.0), (190.0, 210.0), (230.0, 240.0)]);
    // A file read but shown in a box with no size shows nothing, and its box isn't measured.
    let id = NodeId(s.execute("file.place", &json!({"path": dir.file("pair.vectorcraft"), "at": [400, 150]})).unwrap()["ids"][0].as_u64().unwrap());
    s.edit("no size", |d, _| {
        if let Some(NodeKind::PlacedDocument(p)) = d.node_mut(id).map(|n| &mut n.kind) {
            (p.width, p.height) = (0.0, 0.0);
        }
        Ok(())
    })
    .unwrap();
    select(&mut s, &[id]);
    let e = s.execute("addon.cutting.weed", &json!({})).unwrap_err();
    assert!(e.to_string().contains("has no outlines"), "{e}");
    // A file read with nothing showing has no outlines, and its box isn't measured instead.
    let mut s = session();
    place(&mut s, "hidden.vectorcraft", true);
    let e = s.execute("addon.cutting.weed", &json!({})).unwrap_err();
    assert!(e.to_string().contains("has no outlines"), "{e}");
    // A file whose art is an instance of a symbol of its own: the instance's art is measured.
    let src = dir.file("symbol.vectorcraft");
    let mut d = vectorcraft_doc::Document::new(60.0, 40.0);
    let look = Appearance::basic(Paint::solid(vectorcraft_color::Color::BLACK), Paint::None, 0.0);
    let art = Node::path(NodeId(900), shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 40.0)), look);
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Bar".into(), art: Arc::new(art) });
    let (layer, id) = (d.layers[0].id, d.alloc_id());
    d.insert(Some(layer), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "Bar".into(), xf: Affine::IDENTITY })).unwrap();
    crate::tests_links::write(&src, &vectorcraft_format::save_file(&d));
    let mut s = session();
    let r = s.execute("file.place", &json!({"path": src, "at": [200, 150]})).unwrap();
    select(&mut s, &[NodeId(r["ids"][0].as_u64().unwrap())]);
    let (_, g) = weed(&mut s, json!({"margin": 10}));
    let b = weed_box(&g).unwrap();
    assert!(b.x0 >= 160.0 - 1e-6 && b.x1 <= 240.0 + 1e-6 && b.width() < 41.0, "the bar, not the whole box: {b:?}");
}

#[test]
fn a_placed_file_that_cant_be_read_is_measured_by_its_box_as_turned() {
    let mut s = session();
    let p = vectorcraft_doc::PlacedDocument {
        link: vectorcraft_doc::LinkInfo::new("/no/such/file.pdf"),
        key: "missing".into(),
        bounding: false,
        page_box: Default::default(),
        width: 100.0,
        height: 100.0,
        xf: diamond_xf(),
        placement: Default::default(),
    };
    insert(&mut s, |id| Node::new(id, NodeKind::PlacedDocument(Box::new(p))));
    let (b, end) = diamond_weeding(&mut s);
    let half = 50.0 * std::f64::consts::SQRT_2;
    let y = b.y0 + b.height() / 4.0;
    assert!((end - (300.0 - (y - (300.0 - half)))).abs() < 0.01, "{end} at {y}");
}

#[test]
fn a_symbol_instance_is_measured_by_its_art() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 20.0, 40.0);
    let b = rect(&mut s, 140.0, 100.0, 20.0, 40.0);
    select(&mut s, &[a, b]);
    let inst = id_of(&s.execute("symbol.new", &json!({"name": "Pair"})).unwrap());
    select(&mut s, &[inst]);
    let (_, g) = weed(&mut s, json!({"margin": 10, "direction": "horizontal"}));
    // The line runs between the two rectangles too.
    assert_eq!(at(&g, 120.0), [(90.0, 100.0), (120.0, 140.0), (160.0, 170.0)]);
    // An instance of a symbol the document doesn't have draws nothing, and has no outlines.
    s.edit("lose the symbol", |d, _| {
        if let Some(NodeKind::SymbolInstance { symbol, .. }) = d.node_mut(inst).map(|n| &mut n.kind) {
            *symbol = "Missing".into();
        }
        Ok(())
    })
    .unwrap();
    select(&mut s, &[inst]);
    let n = s.doc().unwrap().doc.node_count();
    let e = s.execute("addon.cutting.weed", &json!({})).unwrap_err();
    assert!(e.to_string().contains("has no outlines"), "{e}");
    assert_eq!(s.doc().unwrap().doc.node_count(), n);
    let shown = rect(&mut s, 300.0, 300.0, 20.0, 20.0);
    select(&mut s, &[inst, shown]);
    let (_, g) = weed(&mut s, json!({"margin": 10}));
    assert_eq!(weed_box(&g), Some(Rect::new(290.0, 290.0, 330.0, 330.0)));
}

#[test]
fn a_blend_is_measured_by_the_art_it_draws() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 20.0, 40.0);
    let b = rect(&mut s, 200.0, 100.0, 20.0, 40.0);
    let blend = id_of(&s.execute("object.blend.make", &json!({"ids": [a.0, b.0], "steps": 1})).unwrap());
    select(&mut s, &[blend]);
    let (_, g) = weed(&mut s, json!({"margin": 10, "direction": "horizontal"}));
    // The step halfway between the two rectangles stops the line too.
    assert_eq!(at(&g, 120.0), [(90.0, 100.0), (120.0, 150.0), (170.0, 200.0), (220.0, 230.0)]);
}

#[test]
fn the_group_goes_on_top_of_the_current_layer_and_hidden_layers_are_left_out() {
    let mut s = session();
    let first = s.doc().unwrap().doc.layers[0].id;
    let hidden = rect(&mut s, 100.0, 100.0, 50.0, 40.0);
    let art = id_of(&s.execute("layer.new", &json!({"name": "Art"})).unwrap());
    let shown = rect(&mut s, 300.0, 300.0, 20.0, 20.0);
    let below = rect(&mut s, 400.0, 400.0, 20.0, 20.0);
    s.edit("hide", |d, _| {
        if let Some(n) = d.node_mut(first) {
            n.visible = false;
        }
        Ok(())
    })
    .unwrap();
    select(&mut s, &[hidden]);
    assert!(s.execute("addon.cutting.weed", &json!({})).is_err(), "art in a hidden layer doesn't show");
    select(&mut s, &[hidden, shown]);
    let (_, g) = weed(&mut s, json!({"margin": 10}));
    assert_eq!(weed_box(&g), Some(Rect::new(290.0, 290.0, 330.0, 330.0)));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.parent_of(g.id), Some(art));
    assert_eq!(d.node(art).unwrap().children().unwrap().last().map(|c| c.id), Some(g.id), "on top");
    // A hidden object selected on its own shows nothing.
    s.edit("hide", |d, _| {
        if let Some(n) = d.node_mut(below) {
            n.visible = false;
        }
        Ok(())
    })
    .unwrap();
    select(&mut s, &[below]);
    assert!(s.execute("addon.cutting.weed", &json!({})).is_err(), "a hidden object doesn't show");
}

#[test]
fn the_group_goes_outside_a_clip_group_edited_in_isolation() {
    let mut s = session();
    let layer = s.doc().unwrap().doc.layers[0].id;
    let g = clip_group(&mut s, shapes::rectangle(Rect::new(100.0, 100.0, 150.0, 140.0)), shapes::rectangle(Rect::new(120.0, 0.0, 220.0, 300.0)));
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    let art = node(&s, g).children().unwrap()[1].id;
    select(&mut s, &[art]);
    let (_, w) = weed(&mut s, json!({"margin": 10, "direction": "horizontal"}));
    // The clipping path would hide the lines and the box around the art inside it.
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.parent_of(w.id), Some(layer));
    assert_eq!(weed_box(&w), Some(Rect::new(110.0, 90.0, 160.0, 150.0)));
    // A plain group edited in isolation inside a clip group: the group goes outside the clip group.
    let mut s = session();
    let parent = s.doc().unwrap().target_parent().unwrap();
    let (inner, art) = s
        .edit("clip group around a group", |d, _| {
            let clip = Arc::new(Node::path(d.alloc_id(), shapes::rectangle(Rect::new(100.0, 100.0, 150.0, 140.0)), Appearance::default()));
            let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(120.0, 0.0, 220.0, 300.0)), Appearance::default());
            let art_id = art.id;
            let inner = Node::group(d.alloc_id(), vec![Arc::new(art)]);
            let inner_id = inner.id;
            let mut g = Node::group(d.alloc_id(), vec![clip, Arc::new(inner)]);
            if let NodeKind::Group { clip, .. } = &mut g.kind {
                *clip = true;
            }
            d.insert(parent, usize::MAX, g)?;
            Ok((inner_id, art_id))
        })
        .unwrap();
    s.execute("object.isolate", &json!({"id": inner.0})).unwrap();
    select(&mut s, &[art]);
    let (_, w) = weed(&mut s, json!({"margin": 10, "direction": "horizontal"}));
    assert_eq!(s.doc().unwrap().doc.parent_of(w.id), parent);
    assert_eq!(weed_box(&w), Some(Rect::new(110.0, 90.0, 160.0, 150.0)));
}

/// Adds one path of `count` open vertical subpaths 2 pt apart, from (100, 100) down to y = 200.
fn bars(s: &mut Session, count: u32) -> NodeId {
    let mut p = BezPath::new();
    for i in 0..count {
        let x = 100.0 + f64::from(i) * 2.0;
        p.move_to((x, 100.0));
        p.line_to((x, 200.0));
    }
    insert(s, |id| Node::path(id, PathData::from_bezpath(&p), Appearance::default()))
}

#[test]
fn more_than_the_most_pieces_of_line_is_refused() {
    // 1,000 lines each cut into 100 pieces by 101 bars: as many as one run adds.
    let mut s = session();
    bars(&mut s, 101);
    let r = s.execute("addon.cutting.weed", &json!({"margin": 0, "count": 1000, "direction": "horizontal"})).unwrap();
    assert_eq!(r["lines"], json!(super::MAX_PIECES));
    // One bar more makes 1,000 pieces more.
    let mut s = session();
    bars(&mut s, 102);
    let n = s.doc().unwrap().doc.node_count();
    let e = s.execute("addon.cutting.weed", &json!({"margin": 0, "count": 1000, "direction": "horizontal"})).unwrap_err();
    assert!(e.to_string().contains("more than 100000 pieces"), "{e}");
    assert_eq!(s.doc().unwrap().doc.node_count(), n);
}

#[test]
fn art_with_more_than_the_most_outline_points_is_refused() {
    // An open zigzag of `points` points: one subpath, measured by its points.
    let zigzag = |s: &mut Session, points: u32| {
        let mut p = BezPath::new();
        p.move_to((100.0, 100.0));
        for i in 1..points {
            p.line_to((100.0 + f64::from(i) * 0.001, if i % 2 == 0 { 100.0 } else { 101.0 }));
        }
        insert(s, |id| Node::path(id, PathData::from_bezpath(&p), Appearance::default()));
    };
    let mut s = session();
    zigzag(&mut s, super::geometry::MAX_POINTS as u32);
    s.execute("addon.cutting.weed", &json!({"count": 0})).unwrap();
    let mut s = session();
    zigzag(&mut s, super::geometry::MAX_POINTS as u32 + 1);
    let e = s.execute("addon.cutting.weed", &json!({"count": 0})).unwrap_err();
    assert!(e.to_string().contains("too many outline points"), "{e}");
}

#[test]
fn bad_params_and_an_empty_selection_are_refused() {
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 50.0, 40.0);
    for p in [
        json!({"margin": -1}),
        json!({"gap": "x"}),
        json!({"count": 1.5}),
        json!({"count": 1001}),
        json!({"count": 2, "spacing": 10}),
        json!({"spacing": 0}),
        json!({"direction": "diagonal"}),
        json!({"box": "yes"}),
        json!({"weight": 0}),
        json!({"swatch": "no such swatch"}),
        json!({"swatch": 3}),
        json!({"box": false, "count": 0}),
    ] {
        let n = s.doc().unwrap().doc.node_count();
        assert!(s.execute("addon.cutting.weed", &p).is_err(), "{p}");
        assert_eq!(s.doc().unwrap().doc.node_count(), n, "nothing added for {p}");
    }
    // The same art with good params is weeded; with nothing selected it is refused.
    s.execute("addon.cutting.weed", &json!({"count": 2, "gap": "1 mm"})).unwrap();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    assert!(s.execute("addon.cutting.weed", &json!({})).is_err());
}
