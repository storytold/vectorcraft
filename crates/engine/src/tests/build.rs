//! Tests for Shape Builder, Live Paint and Image Trace commands and tools.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, ImageBlob, ImageObject, LineCap, LineJoin, Node, NodeKind};
use vectorcraft_geom::{Affine, FillRule, PathData, Point, Rect};
use vectorcraft_testkit::pdf::{PdfPage, pdf};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn set_fill(s: &mut Session, id: NodeId, c: Color) {
    s.edit("t", |d, _| {
        d.node_mut(id).unwrap().appearance.set_fill(Paint::solid(c));
        Ok(())
    })
    .unwrap();
}

fn ids(v: &Value) -> Vec<NodeId> {
    v["ids"].as_array().unwrap().iter().map(|x| NodeId(x.as_u64().unwrap())).collect()
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

fn outline(n: &Node) -> (PathData, FillRule) {
    vectorcraft_tools::builder::node_outline(n).expect("path")
}

fn area(s: &Session, id: NodeId) -> f64 {
    let (p, r) = outline(&node(s, id));
    vectorcraft_pathops::area(&p, r)
}

/// Rect A (0..100) and rect B (50..150), both 100 tall, selected. A red, B blue.
fn two(s: &mut Session) -> (NodeId, NodeId) {
    two_at(s, 50.0)
}

/// [`two`] with B at `x`..`x`+100.
fn two_at(s: &mut Session, x: f64) -> (NodeId, NodeId) {
    let a = rect(s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(s, x, 0.0, 100.0, 100.0);
    set_fill(s, a, Color::rgb(1.0, 0.0, 0.0));
    set_fill(s, b, Color::rgb(0.0, 0.0, 1.0));
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    (a, b)
}

fn layer_children(s: &Session) -> usize {
    let d = &s.doc().unwrap().doc;
    d.children(d.default_layer()).map(|c| c.len()).unwrap_or(0)
}

// ---------- Shape Builder ----------

#[test]
fn shape_builder_drag_merges_touched_regions() {
    let mut s = session();
    two(&mut s);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[25, 50], [75, 50]]})).unwrap();
    assert_eq!(r["regions"], 2);
    let out = ids(&r);
    assert_eq!(out.len(), 2, "merged shape + remaining part of B");
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert!((area(&s, merged) - 10000.0).abs() < 1.0, "{}", area(&s, merged));
    let rest = out.iter().copied().find(|i| *i != merged).unwrap();
    assert!((area(&s, rest) - 5000.0).abs() < 1.0);
    // Merged shape takes the fill of the object under the drag start (A, red).
    assert_eq!(node(&s, merged).appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    assert_eq!(layer_children(&s), 2);
}

#[test]
fn shape_builder_drag_merges_a_region_thinner_than_the_sampling_step() {
    let mut s = session();
    two_at(&mut s, 99.9);
    assert_eq!(s.execute("shapeBuilder.regions", &json!({})).unwrap()["regions"].as_array().unwrap().len(), 3);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[50, 50], [150, 50]]})).unwrap();
    assert_eq!(r["regions"], 3, "the 0.1 pt overlap too");
    let out = ids(&r);
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(outline(&node(&s, out[0])).0.subpaths.len(), 1);
    assert!((area(&s, out[0]) - 19990.0).abs() < 1.0, "{}", area(&s, out[0]));
}

#[test]
fn shape_builder_drag_merges_a_narrow_overlap_when_far_art_is_selected() {
    let mut s = session();
    let (a, b) = two_at(&mut s, 99.0);
    // A far square makes the selection 4,010 pt wide.
    let far = rect(&mut s, 4000.0, 4000.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0, far.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[50.3, 50], [150.3, 50]]})).unwrap();
    assert_eq!(r["regions"], 3);
    let out = ids(&r);
    assert_eq!(out.len(), 2, "the merged shape and the far square: {out:?}");
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert_eq!(outline(&node(&s, merged)).0.subpaths.len(), 1);
    assert!((area(&s, merged) - 19900.0).abs() < 1.0, "{}", area(&s, merged));
}

#[test]
fn shape_builder_drag_merges_across_an_overlap_thinner_than_the_precision() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let b = rect(&mut s, 199.999, 100.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[150, 150], [250, 150]]})).unwrap();
    let out = ids(&r);
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(outline(&node(&s, out[0])).0.subpaths.len(), 1, "no hairline gap at the 0.001 pt overlap");
    assert!((area(&s, out[0]) - 19999.9).abs() < 1.0, "{}", area(&s, out[0]));
    // A click on one side leaves the sliver to the other square, which stays whole.
    s.execute("edit.undo", &json!({})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[150, 150]]})).unwrap();
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert!((area(&s, merged) - 9999.9).abs() < 1e-3, "{}", area(&s, merged));
    let out = ids(&r);
    assert_eq!(out.len(), 2, "{out:?}");
    let rest = out.iter().copied().find(|i| *i != merged).unwrap();
    assert!((area(&s, rest) - 10000.0).abs() < 1e-6, "{}", area(&s, rest));
}

#[test]
fn shape_builder_drag_merges_across_an_overlap_thinner_than_the_precision_with_a_line_selected() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let b = rect(&mut s, 199.999, 100.0, 100.0, 100.0);
    let l = line(&mut s, (400.0, 400.0), (450.0, 450.0));
    s.execute("select.set", &json!({"ids": [a.0, b.0, l.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[150, 150], [250, 150]]})).unwrap();
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert_eq!(ids(&r).len(), 2, "the merged shape and the line: {r}");
    assert_eq!(outline(&node(&s, merged)).0.subpaths.len(), 1, "no hairline gap at the 0.001 pt overlap");
    assert!((area(&s, merged) - 19999.9).abs() < 1.0, "{}", area(&s, merged));
}

#[test]
fn shape_builder_click_beside_a_sliver_leaves_it_to_one_shape() {
    let mut s = session();
    // Two lines 0.005 pt apart cut a square; the strip between them is a sliver.
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let l1 = line(&mut s, (150.0, 50.0), (150.0, 250.0));
    let l2 = line(&mut s, (150.005, 50.0), (150.005, 250.0));
    s.execute("select.set", &json!({"ids": [a.0, l1.0, l2.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[120, 150]]})).unwrap();
    let merged = NodeId(r["merged"].as_u64().unwrap());
    let rest: Vec<NodeId> = ids(&r).into_iter().filter(|i| *i != merged && outline(&node(&s, *i)).0.subpaths.iter().all(|sp| sp.closed)).collect();
    assert_eq!(rest.len(), 1, "{r}");
    let total = area(&s, merged) + area(&s, rest[0]);
    assert!((total - 10000.0).abs() < 0.1, "the merged shape and the rest overlap: {} + {}", area(&s, merged), area(&s, rest[0]));
}

#[test]
fn shape_builder_click_beside_a_sliver_of_closed_shapes_leaves_it_to_one_shape() {
    let mut s = session();
    // Two squares over a third leave a strip of it 0.005 pt wide between them.
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let b = rect(&mut s, 100.0, 100.0, 49.9975, 100.0);
    let c = rect(&mut s, 150.0025, 100.0, 49.9975, 100.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0, c.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[125, 150]]})).unwrap();
    let out = ids(&r);
    assert_eq!(out.len(), 3, "the merged shape, the rest of the back square and the right square: {r}");
    let total: f64 = out.iter().map(|i| area(&s, *i)).sum();
    assert!((total - 14999.75).abs() < 0.1, "the merged shape and the rest overlap: {total}");
}

#[test]
fn shape_builder_click_keeps_a_sliver_the_merge_leaves_out_in_the_rest() {
    // Also with a line selected, which merges in the planar map.
    for with_line in [false, true] {
        let mut s = session();
        // B overlaps A and C by 0.005 pt more than C does: the strip of A and B between them is a
        // sliver next to the region of A alone, which a click merges without it.
        let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
        let b = rect(&mut s, 49.995, 0.0, 50.005, 100.0);
        let c = rect(&mut s, 50.0, 0.0, 50.0, 100.0);
        let mut sel = vec![a.0, b.0, c.0];
        if with_line {
            sel.push(line(&mut s, (400.0, 400.0), (450.0, 450.0)).0);
        }
        s.execute("select.set", &json!({ "ids": sel })).unwrap();
        let r = s.execute("shapeBuilder.merge", &json!({"points": [[25, 50]]})).unwrap();
        let merged = NodeId(r["merged"].as_u64().unwrap());
        assert!((area(&s, merged) - 4999.5).abs() < 0.1, "{}", area(&s, merged));
        let out = ids(&r);
        assert_eq!(out.len(), sel.len() + 1, "{r}");
        // What is left of A, B and C: 5,000.5, 5,000.5 and 5,000.
        let total: f64 = out.iter().map(|i| area(&s, *i)).sum();
        assert!((total - 20000.5).abs() < 0.1, "what is left of the back square lost the strip: {total} ({with_line})");
    }
}

#[test]
fn shape_builder_click_keeps_a_sliver_away_from_the_merge_in_the_rest() {
    let mut s = session();
    // Three squares over a fourth leave two strips of it 0.005 pt wide; a click merges the left
    // square's region, next to the first strip only.
    let a = rect(&mut s, 100.0, 100.0, 150.0, 100.0);
    let b = rect(&mut s, 100.0, 100.0, 49.9975, 100.0);
    let c = rect(&mut s, 150.0025, 100.0, 49.995, 100.0);
    let e = rect(&mut s, 200.0025, 100.0, 49.9975, 100.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0, c.0, e.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[125, 150]]})).unwrap();
    let out = ids(&r);
    assert_eq!(out.len(), 4, "{r}");
    let rest = out.iter().copied().find(|i| area(&s, *i) > 9000.0).unwrap();
    assert_eq!(outline(&node(&s, rest)).0.subpaths.len(), 1, "no hairline gap at the second strip");
    let total: f64 = out.iter().map(|i| area(&s, *i)).sum();
    assert!((total - 24999.25).abs() < 0.1, "{total}");
}

#[test]
fn shape_builder_click_keeps_a_sliver_away_from_the_merge_in_the_rest_with_a_compound_path() {
    let mut s = session();
    // The same with the three squares in one compound path, whose three regions have one coverage.
    let a = rect(&mut s, 100.0, 100.0, 150.0, 100.0);
    let b = rect(&mut s, 100.0, 100.0, 49.9975, 100.0);
    let c = rect(&mut s, 150.0025, 100.0, 49.995, 100.0);
    let e = rect(&mut s, 200.0025, 100.0, 49.9975, 100.0);
    s.execute("select.set", &json!({"ids": [b.0, c.0, e.0]})).unwrap();
    let compound = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [a.0, compound.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[125, 150]]})).unwrap();
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert!((area(&s, merged) - 5000.25).abs() < 0.1, "{}", area(&s, merged));
    let out = ids(&r);
    assert_eq!(out.len(), 3, "{r}");
    // What is left of the back square holds the second strip; what is left of the compound path
    // keeps the gap it had there.
    let mut rest: Vec<(usize, f64)> =
        out.iter().copied().filter(|i| *i != merged).map(|i| (outline(&node(&s, i)).0.subpaths.len(), area(&s, i))).collect();
    rest.sort_by_key(|r| r.0);
    assert_eq!(rest.iter().map(|r| r.0).collect::<Vec<_>>(), [1, 2], "{rest:?}");
    assert!((rest[0].1 - 9999.75).abs() < 0.1 && (rest[1].1 - 9999.25).abs() < 0.1, "{rest:?}");
}

#[test]
fn shape_builder_click_splits_out_one_region() {
    let mut s = session();
    two(&mut s);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[75, 50]]})).unwrap();
    let out = ids(&r);
    assert_eq!(out.len(), 3);
    let total: f64 = out.iter().map(|i| area(&s, *i)).sum();
    assert!((total - 15000.0).abs() < 1.0, "{total}");
    let merged = NodeId(r["merged"].as_u64().unwrap());
    // Overlap is covered by B on top → blue.
    assert_eq!(node(&s, merged).appearance.fill_paint(), Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
    let bb = node(&s, merged).geometric_bounds().unwrap();
    assert!((bb.x0 - 50.0).abs() < 1e-6 && (bb.x1 - 100.0).abs() < 1e-6);
}

#[test]
fn shape_builder_erase_deletes_regions() {
    let mut s = session();
    two(&mut s);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[75, 50]], "erase": true})).unwrap();
    assert!(r["merged"].is_null());
    let out = ids(&r);
    assert_eq!(out.len(), 2);
    for i in out {
        assert!((area(&s, i) - 5000.0).abs() < 1.0);
    }
}

#[test]
fn shape_builder_fill_option_and_untouched_shapes_kept() {
    let mut s = session();
    let (a, b) = two(&mut s);
    let c = rect(&mut s, 300.0, 300.0, 50.0, 50.0);
    let r = s.execute("shapeBuilder.merge", &json!({"ids": [a.0, b.0, c.0], "points": [[25, 50], [125, 50]], "fill": "#00ff00"})).unwrap();
    let out = ids(&r);
    assert_eq!(out.len(), 2, "one merged + the untouched square");
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert!((area(&s, merged) - 15000.0).abs() < 1.0);
    assert_eq!(node(&s, merged).appearance.fill_paint(), Paint::solid(Color::from_hex("#00ff00").unwrap()));
    let sq = out.iter().copied().find(|i| *i != merged).unwrap();
    assert_eq!(node(&s, sq).geometric_bounds().unwrap(), Rect::new(300.0, 300.0, 350.0, 350.0));
    assert_eq!(outline(&node(&s, sq)).0.anchor_count(), 4);
}

#[test]
fn shape_builder_miss_is_an_error_and_undo_restores() {
    let mut s = session();
    let (a, b) = two(&mut s);
    let undo_before = s.doc().unwrap().history.undo.len();
    assert!(s.execute("shapeBuilder.merge", &json!({"points": [[500, 500]]})).is_err());
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_before);
    s.execute("shapeBuilder.merge", &json!({"points": [[25, 50], [75, 50]]})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(a).is_some() && s.doc().unwrap().doc.node(b).is_some());
    assert!(s.execute("shapeBuilder.merge", &json!({"points": []})).is_err());
    assert!(s.execute("shapeBuilder.merge", &json!({})).is_err());
}

/// A 3×3 grid of touching 100 pt squares from (0, 0) to (300, 300), each in its own gray, selected.
fn grid(s: &mut Session) -> Vec<NodeId> {
    let ids: Vec<NodeId> = (0..9)
        .map(|k| {
            let id = rect(s, (k % 3) as f64 * 100.0, (k / 3) as f64 * 100.0, 100.0, 100.0);
            let gray = k as f32 / 10.0;
            set_fill(s, id, Color::rgb(gray, gray, gray));
            id
        })
        .collect();
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    ids
}

/// `rect`: the regions a marquee reaches merge into one shape, which takes the look of the square
/// where the marquee started, else of the square where it ended.
#[test]
fn shape_builder_rect_merges_every_region_the_marquee_reaches() {
    let mut s = session();
    let squares = grid(&mut s);
    let r = s.execute("shapeBuilder.merge", &json!({"rect": [250, 250, -200, -200]})).unwrap();
    assert_eq!(r["regions"], 9);
    let out = ids(&r);
    assert_eq!(out.len(), 1, "{r}");
    assert!((area(&s, out[0]) - 90000.0).abs() < 1.0, "{}", area(&s, out[0]));
    assert_eq!(outline(&node(&s, out[0])).0.subpaths.len(), 1, "one path, no seams");
    assert_eq!(node(&s, out[0]).appearance.fill_paint(), Paint::solid(Color::rgb(0.8, 0.8, 0.8)), "the bottom-right square's look");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": squares.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"rect": [50, 50, 200, 200]})).unwrap();
    assert_eq!(node(&s, ids(&r)[0]).appearance.fill_paint(), Paint::solid(Color::rgb(0.0, 0.0, 0.0)), "the top-left square's look");
    s.execute("edit.undo", &json!({})).unwrap();
    // Started outside every square: the look of the middle square, where it ended.
    s.execute("select.set", &json!({"ids": squares.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"rect": [-50, -50, 200, 200]})).unwrap();
    assert_eq!(r["regions"], 4);
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert_eq!(node(&s, merged).appearance.fill_paint(), Paint::solid(Color::rgb(0.4, 0.4, 0.4)), "the middle square's look");
    s.execute("edit.undo", &json!({})).unwrap();
    // Points as well add the regions along them; the marquee touches the left column only.
    s.execute("select.set", &json!({"ids": squares.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"rect": [40, 50, 20, 200], "points": [[150, 50], [250, 50]]})).unwrap();
    assert_eq!(r["regions"], 5);
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert!((area(&s, merged) - 50000.0).abs() < 1.0, "{}", area(&s, merged));
}

/// `rect` with `erase`: the regions the marquee reaches are deleted.
#[test]
fn shape_builder_rect_with_erase_deletes_the_regions_the_marquee_reaches() {
    let mut s = session();
    grid(&mut s);
    let r = s.execute("shapeBuilder.merge", &json!({"rect": [140, 50, 20, 200], "erase": true})).unwrap();
    assert_eq!((&r["regions"], &r["merged"]), (&json!(3), &Value::Null));
    let out = ids(&r);
    assert_eq!(out.len(), 6, "{r}");
    assert!(out.iter().all(|i| (area(&s, *i) - 10000.0).abs() < 1.0));
    let outside_the_middle = |i: &NodeId| node(&s, *i).geometric_bounds().is_some_and(|b| b.x1 <= 100.0 + 1e-6 || b.x0 >= 200.0 - 1e-6);
    assert!(out.iter().all(outside_the_middle), "the middle column is gone");
}

#[test]
fn shape_builder_rect_must_be_four_finite_numbers() {
    let mut s = session();
    grid(&mut s);
    let undo_before = s.doc().unwrap().history.undo.len();
    for rect in [json!([0, 0, 10]), json!("big"), json!([0, 0, 10, f64::MAX]), json!([0, 0, "10", 10])] {
        let e = s.execute("shapeBuilder.merge", &json!({"rect": rect})).unwrap_err().to_string();
        assert!(e.contains("rect must be [x, y, width, height]"), "{rect}: {e}");
    }
    // Outside every region: nothing to merge.
    assert!(s.execute("shapeBuilder.merge", &json!({"rect": [400, 400, 50, 50]})).is_err());
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_before);
    // A null rect is left out, and points are needed again.
    assert!(s.execute("shapeBuilder.merge", &json!({"rect": null})).is_err());
}

/// With `rect`, `splitStroke` splits nothing, even with one point on a stroke: the marquee's
/// regions merge.
#[test]
fn shape_builder_rect_merges_and_does_not_split_a_stroke() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"rect": [40, -5, 20, 20], "points": [[50, -1]], "splitStroke": true})).unwrap();
    assert_eq!((r.get("split"), r["regions"].as_u64(), r["edges"].as_u64()), (None, Some(1), Some(0)), "{r}");
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert_eq!(shapes_of(&s, &[merged]), vec![(true, 4, 400.0)]);
}

/// Past the segment limit only the filled areas count, so a stroke-only line along y = 150 cuts no
/// square: a marquee above the line merges the three whole squares of the middle row, as a drag
/// above it does. Under the limit the line cuts the middle row, and both merge only the halves
/// above it.
#[test]
fn shape_builder_rect_past_the_segment_limit_merges_whole_regions_as_a_drag_does() {
    let merged_area = |segments: usize, params: Value| {
        let mut s = session();
        let mut ids = grid(&mut s);
        let pts: Vec<Point> = (0..=segments).map(|k| Point::new(-10.0 + 320.0 * k as f64 / segments as f64, 150.0)).collect();
        let look = vectorcraft_doc::Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
        let l = s
            .edit("t", |d, _| {
                let id = d.alloc_id();
                let top = d.children(d.default_layer()).map_or(0, |c| c.len());
                d.insert(d.default_layer(), top, Node::path(id, PathData::single(vectorcraft_geom::SubPath::polyline(&pts, false)), look))?;
                Ok(id)
            })
            .unwrap();
        ids.push(l);
        s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
        let r = s.execute("shapeBuilder.merge", &params).unwrap();
        area(&s, NodeId(r["merged"].as_u64().unwrap()))
    };
    let past = vectorcraft_pathops::SHAPE_BUILDER_MAX_SEGMENTS;
    let marquee = json!({"rect": [20, 120, 260, 20]});
    let drag = json!({"points": [[50, 130], [250, 130]]});
    for (segments, want) in [(2, 15000.0), (past, 30000.0)] {
        for params in [&marquee, &drag] {
            let got = merged_area(segments, params.clone());
            assert!((got - want).abs() < 1.0, "{segments} segments, {params}: {got}");
        }
    }
}

#[test]
fn shape_builder_regions_query() {
    let mut s = session();
    two(&mut s);
    let r = s.execute("shapeBuilder.regions", &json!({})).unwrap();
    let regs = r["regions"].as_array().unwrap();
    assert_eq!(regs.len(), 3);
    let total: f64 = regs.iter().map(|x| x["area"].as_f64().unwrap()).sum();
    assert!((total - 15000.0).abs() < 1.0);
    assert!(regs.iter().any(|x| x["sources"].as_array().unwrap().len() == 2));
}

/// #1691: four circles in a 2×2 grid, each touching its neighbours, enclose a diamond nothing
/// fills. It is a region, and a drag from a circle into it merges the two.
#[test]
fn shape_builder_merges_the_gap_touching_circles_enclose() {
    let mut s = session();
    let mut circles = vec![];
    for (x, y) in [(0.0, 0.0), (100.0, 0.0), (0.0, 100.0), (100.0, 100.0)] {
        let r = s.execute("shape.ellipse", &json!({"x": x, "y": y, "width": 100, "height": 100})).unwrap();
        circles.push(NodeId(r["id"].as_u64().unwrap()));
    }
    set_fill(&mut s, circles[0], Color::rgb(1.0, 0.0, 0.0));
    s.execute("select.set", &json!({"ids": circles.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    let r = s.execute("shapeBuilder.regions", &json!({})).unwrap();
    let regs = r["regions"].as_array().unwrap();
    assert_eq!(regs.len(), 5, "{regs:?}");
    assert_eq!(regs.iter().filter(|x| x["sources"].as_array().unwrap().is_empty()).count(), 1, "the diamond");
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[50, 50], [100, 100]]})).unwrap();
    assert_eq!(r["regions"], 2);
    let out = ids(&r);
    assert_eq!(out.len(), 4, "the merged shape and the three other circles");
    let merged = NodeId(r["merged"].as_u64().unwrap());
    // A circle (its Bézier outline: 7,854.9) and the diamond between the four (2,145.1).
    assert!((area(&s, merged) - 10000.0).abs() < 10.0, "{}", area(&s, merged));
    assert_eq!(node(&s, merged).appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), "the look of the circle the drag began on");
    assert_eq!(layer_children(&s), 4);
    // A click in the diamond alone makes it a shape of its own.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": circles.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[100, 100]]})).unwrap();
    assert_eq!(ids(&r).len(), 5);
    assert!((area(&s, NodeId(r["merged"].as_u64().unwrap())) - 2146.0).abs() < 10.0);
}

#[test]
fn shape_builder_tool_drives_the_command() {
    let mut s = session();
    two(&mut s);
    let v = ViewInfo::default();
    s.select_tool("shapeBuilder", v).unwrap();
    assert_eq!(s.tool_id(), "shapeBuilder");
    s.pointer(&PointerEvent::new(PointerKind::Move, 75.0, 50.0), v).unwrap();
    assert!(!s.overlays(v).is_empty(), "hover highlight");
    s.pointer(&PointerEvent::new(PointerKind::Down, 25.0, 50.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 75.0, 50.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 75.0, 50.0), v).unwrap();
    assert_eq!(layer_children(&s), 2);
    assert_eq!(s.journal.last().unwrap().0, "shapeBuilder.merge");
    // Alt-click deletes a region.
    let alt = Mods { alt: true, ..Default::default() };
    s.pointer(&PointerEvent::new(PointerKind::Down, 125.0, 50.0).with_mods(alt), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 125.0, 50.0).with_mods(alt), v).unwrap();
    assert_eq!(layer_children(&s), 1);
}

/// A click of the Shape Builder merges over the map the tool built for the hover, as does the
/// command over the same inputs with the same gap; another gap needs a map of its own.
#[test]
fn shape_builder_merge_uses_the_map_the_tool_has() {
    use vectorcraft_tools::builder::maps_built;
    let mut s = session();
    two(&mut s);
    let v = ViewInfo::default();
    s.select_tool("shapeBuilder", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Move, 75.0, 50.0), v).unwrap();
    let built = maps_built();
    s.pointer(&PointerEvent::new(PointerKind::Down, 75.0, 50.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 75.0, 50.0), v).unwrap();
    assert_eq!(s.journal.last().unwrap().0, "shapeBuilder.merge");
    assert_eq!(layer_children(&s), 3, "the overlap and what is left of each square");
    assert_eq!(maps_built(), built, "the click builds no map");
    // Undone, the squares are the inputs the tool's map was built from.
    s.execute("edit.undo", &json!({})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[75, 50]]})).unwrap();
    assert!((area(&s, NodeId(r["merged"].as_u64().unwrap())) - 5000.0).abs() < 1e-6);
    assert_eq!(maps_built(), built, "the command over the same inputs builds no map");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("shapeBuilder.merge", &json!({"points": [[75, 50]], "gap": 3})).unwrap();
    assert_eq!(maps_built(), built + 1, "another gap");
}

#[test]
fn shape_builder_tool_merges_the_narrow_shape_it_highlights_in_a_wide_selection() {
    let mut s = session();
    let strip = rect(&mut s, 199.5, 100.0, 1.0, 100.0);
    // A far square makes the selection 9,800.5 pt wide.
    let far = rect(&mut s, 9900.0, 100.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [strip.0, far.0]})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("shapeBuilder", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 190.0, 150.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 210.0, 150.0), v).unwrap();
    assert_eq!(s.overlays(v).len(), 3, "the strip's hatch and outline, and the drag path");
    s.pointer(&PointerEvent::new(PointerKind::Up, 210.0, 150.0), v).unwrap();
    assert_eq!(s.journal.last().unwrap().0, "shapeBuilder.merge");
    assert_eq!(layer_children(&s), 2);
}

#[test]
fn shape_builder_tool_highlights_and_merges_a_narrow_overlap() {
    let mut s = session();
    two_at(&mut s, 99.95);
    let v = ViewInfo::default();
    s.select_tool("shapeBuilder", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 50.3, 50.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 150.3, 50.0), v).unwrap();
    assert_eq!(s.overlays(v).len(), 7, "the three regions' hatches and outlines, and the drag path");
    s.pointer(&PointerEvent::new(PointerKind::Up, 150.3, 50.0), v).unwrap();
    assert_eq!(layer_children(&s), 1, "one shape, no leftover strips");
    let d = &s.doc().unwrap().doc;
    let merged = d.children(d.default_layer()).unwrap()[0].id;
    assert_eq!(outline(&node(&s, merged)).0.subpaths.len(), 1);
}

#[test]
fn shape_builder_tool_zoomed_out_picks_a_narrow_shape() {
    let mut s = session();
    let strip = rect(&mut s, 199.5, 100.0, 1.0, 100.0);
    let near = rect(&mut s, 300.0, 100.0, 3.0, 100.0);
    s.execute("select.set", &json!({"ids": [strip.0, near.0]})).unwrap();
    let v = ViewInfo { zoom: 0.05, ..Default::default() };
    s.select_tool("shapeBuilder", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 190.0, 150.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 260.0, 150.0), v).unwrap();
    assert_eq!(s.overlays(v).len(), 3, "the strip's hatch and outline, and the drag path");
    s.pointer(&PointerEvent::new(PointerKind::Up, 260.0, 150.0), v).unwrap();
    assert_eq!(s.journal.last().unwrap().0, "shapeBuilder.merge");
}

/// The first and last stop colours of a gradient paint.
fn gradient_ends(p: &Paint) -> Option<([f32; 3], [f32; 3])> {
    let Paint::Gradient(g) = p else { return None };
    Some((g.gradient.stops.first()?.color.to_rgb(), g.gradient.stops.last()?.color.to_rgb()))
}

#[test]
fn shape_builder_cuts_gradient_shapes_from_a_pdf_along_their_outlines() {
    // Two overlapping diamonds (x 100–300 and 200–400, y 40–160), each written as gradient-filled
    // shapes are in a PDF or a PDF-compatible .ai: the diamond as a clip, then a shading (#1304).
    let shading = |x0: u32, c0: &str, c1: &str| {
        format!(
            "<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [{x0} 0 {} 0] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [{c0}] /C1 [{c1}] /N 1 >> >>",
            x0 + 200
        )
    };
    let diamond = |x0: u32| format!("q {x0} 100 m {} 160 l {} 100 l {} 40 l h W n", x0 + 100, x0 + 200, x0 + 100);
    let page = PdfPage {
        resources: format!("/Shading << /Sh0 {} /Sh1 {} >>", shading(100, "1 0 0", "1 1 0"), shading(200, "0 0 1", "0 1 1")),
        ..PdfPage::new(500.0, 200.0, &format!("{} /Sh0 sh Q {} /Sh1 sh Q", diamond(100), diamond(200)))
    };
    let bytes = pdf(&[page], None);
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "diamonds.pdf", "dataBase64": vectorcraft_format::base64_encode(&bytes)})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[250, 100]]})).unwrap();
    let mut pieces: Vec<(f64, Rect, Paint)> =
        ids(&r).into_iter().map(|i| (area(&s, i), node(&s, i).geometric_bounds().unwrap(), node(&s, i).appearance.fill_paint())).collect();
    pieces.sort_by(|a, b| a.1.x0.total_cmp(&b.1.x0));
    let near = |a: Rect, b: Rect| [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|v| v.abs() < 0.01);
    let [(left, lb, lp), (mid, mb, mp), (right, rb, rp)] = pieces.as_slice() else { panic!("three pieces: {pieces:?}") };
    // What is left of each diamond, and the diamond where they overlap: not their bounding boxes.
    assert!((left - 9000.0).abs() < 1.0 && (mid - 3000.0).abs() < 1.0 && (right - 9000.0).abs() < 1.0, "{pieces:?}");
    assert!(near(*lb, Rect::new(100.0, 40.0, 250.0, 160.0)), "{lb:?}");
    assert!(near(*mb, Rect::new(200.0, 70.0, 300.0, 130.0)), "{mb:?}");
    assert!(near(*rb, Rect::new(250.0, 40.0, 400.0, 160.0)), "{rb:?}");
    // Each keeps its gradient; the overlap takes the front diamond's.
    let (first, second) = (Some(([1.0, 0.0, 0.0], [1.0, 1.0, 0.0])), Some(([0.0, 0.0, 1.0], [0.0, 1.0, 1.0])));
    assert_eq!((gradient_ends(lp), gradient_ends(mp), gradient_ends(rp)), (first, second, second));
}

/// A red 100 × 100 square crossed by a stroke-only vertical line at x = 50, both selected.
fn square_and_line(s: &mut Session) -> (NodeId, NodeId) {
    let a = rect(s, 0.0, 0.0, 100.0, 100.0);
    set_fill(s, a, Color::rgb(1.0, 0.0, 0.0));
    let l = line(s, (50.0, -20.0), (50.0, 120.0));
    s.execute("select.set", &json!({"ids": [a.0, l.0]})).unwrap();
    (a, l)
}

/// The open paths among `ids`, as (anchors, bounds).
fn open_paths(s: &Session, ids: &[NodeId]) -> Vec<(usize, Rect)> {
    ids.iter()
        .map(|i| node(s, *i))
        .filter(|n| !outline(n).0.is_closed())
        .map(|n| (outline(&n).0.anchor_count(), n.geometric_bounds().unwrap()))
        .collect()
}

#[test]
fn shape_builder_line_splits_a_shape_into_regions() {
    let mut s = session();
    let (a, l) = square_and_line(&mut s);
    let r = s.execute("shapeBuilder.regions", &json!({})).unwrap();
    let regs = r["regions"].as_array().unwrap();
    assert_eq!(regs.len(), 2, "{r}");
    for x in regs {
        assert!((x["area"].as_f64().unwrap() - 5000.0).abs() < 1e-6);
        assert_eq!(x["sources"], json!([a.0]));
    }
    let lines = r["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 3, "the line cut at both sides of the square");
    assert!(lines.iter().all(|x| x["source"] == l.0));
}

#[test]
fn shape_builder_click_on_a_side_of_a_line_splits_the_shape() {
    let mut s = session();
    square_and_line(&mut s);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[25, 50]]})).unwrap();
    assert_eq!(r["regions"], 1);
    let out = ids(&r);
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert_eq!(node(&s, merged).geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 50.0, 100.0));
    assert_eq!(node(&s, merged).appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    let closed: Vec<f64> = out.iter().filter(|i| outline(&node(&s, **i)).0.is_closed()).map(|i| area(&s, *i)).collect();
    assert_eq!(closed.len(), 2, "both halves");
    assert!(closed.iter().all(|a| (a - 5000.0).abs() < 1.0), "{closed:?}");
    // The line only bounds the region: it stays whole.
    assert_eq!(open_paths(&s, &out), vec![(2, Rect::new(50.0, -20.0, 50.0, 120.0))]);
}

/// A line that ends inside a shape has the same region on both sides there: a click or a drag
/// across it, or deleting the region, leaves it whole.
#[test]
fn shape_builder_merge_keeps_a_line_that_ends_inside_the_shape() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    set_fill(&mut s, a, Color::rgb(1.0, 0.0, 0.0));
    let l = line(&mut s, (50.0, -20.0), (50.0, 99.0));
    s.execute("select.set", &json!({"ids": [a.0, l.0]})).unwrap();
    let mut lines = vec![];
    for params in [json!({"points": [[25, 50]]}), json!({"points": [[25, 50], [75, 50]]}), json!({"points": [[25, 50]], "erase": true})] {
        let r = s.execute("shapeBuilder.merge", &params).unwrap();
        assert_eq!(r["regions"], 1, "{params}");
        let merged = r["merged"].as_u64().map(|m| (area(&s, NodeId(m)) - 10000.0).abs() < 1.0);
        assert_eq!(merged, params["erase"].is_null().then_some(true), "{params}");
        lines.push(open_paths(&s, &ids(&r)));
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert_eq!(lines, vec![vec![(2, Rect::new(50.0, -20.0, 50.0, 99.0))]; 3]);
}

#[test]
fn shape_builder_merge_across_a_line_removes_the_piece_between() {
    let mut s = session();
    square_and_line(&mut s);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[25, 50], [75, 50]]})).unwrap();
    assert_eq!(r["regions"], 2);
    let out = ids(&r);
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert!((area(&s, merged) - 10000.0).abs() < 1.0);
    assert_eq!(out.len(), 3, "merged square + the line's two loose ends");
    let mut ends = open_paths(&s, &out);
    ends.sort_by(|a, b| a.1.y0.total_cmp(&b.1.y0));
    assert_eq!(ends, vec![(2, Rect::new(50.0, -20.0, 50.0, 0.0)), (2, Rect::new(50.0, 100.0, 50.0, 120.0))]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(layer_children(&s), 2);
}

#[test]
fn shape_builder_alt_deletes_a_region_or_a_line_piece() {
    let mut s = session();
    square_and_line(&mut s);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[75, 50]], "erase": true})).unwrap();
    assert!(r["merged"].is_null());
    let out = ids(&r);
    let left: Vec<NodeId> = out.iter().copied().filter(|i| outline(&node(&s, *i)).0.is_closed()).collect();
    assert_eq!(left.len(), 1);
    assert_eq!(node(&s, left[0]).geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 50.0, 100.0));
    assert_eq!(open_paths(&s, &out).len(), 1, "the line stays whole");

    // Alt on the line inside the square deletes that piece only.
    let mut s = session();
    square_and_line(&mut s);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[51, 50]], "erase": true, "tolerance": 2})).unwrap();
    assert_eq!((r["regions"].as_u64(), r["lines"].as_u64()), (Some(0), Some(1)));
    let out = ids(&r);
    assert_eq!(out.len(), 3);
    let sq: Vec<NodeId> = out.iter().copied().filter(|i| outline(&node(&s, *i)).0.is_closed()).collect();
    assert_eq!(sq.len(), 1);
    assert!((area(&s, sq[0]) - 10000.0).abs() < 1.0, "the square is untouched");
    assert_eq!(open_paths(&s, &out).len(), 2);
}

#[test]
fn shape_builder_fills_an_area_lines_enclose() {
    let mut s = session();
    let l1 = line(&mut s, (0.0, 0.0), (100.0, 100.0));
    let l2 = line(&mut s, (100.0, 0.0), (0.0, 100.0));
    let l3 = line(&mut s, (-10.0, 80.0), (110.0, 80.0));
    s.execute("select.set", &json!({"ids": [l1.0, l2.0, l3.0]})).unwrap();
    let r = s.execute("shapeBuilder.regions", &json!({})).unwrap();
    assert_eq!(r["regions"].as_array().unwrap().len(), 1);
    assert_eq!(r["regions"][0]["sources"], json!([]));
    let top_line = node(&s, l3).appearance.clone();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[50, 70]]})).unwrap();
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert!((area(&s, merged) - 900.0).abs() < 1e-6, "{}", area(&s, merged));
    assert_eq!(node(&s, merged).appearance, top_line, "no object at the click: the look of the front-most selected line");
    assert_eq!(ids(&r).len(), 4, "the triangle + the three lines, which only bound it");
}

/// Two diagonals and a crossbar that stops about 3 pt short of each: the triangle they bound.
fn lines_with_gaps(s: &mut Session) {
    let l1 = line(s, (0.0, 0.0), (100.0, 100.0));
    let l2 = line(s, (100.0, 0.0), (0.0, 100.0));
    let l3 = line(s, (24.0, 80.0), (76.0, 80.0));
    s.execute("select.set", &json!({"ids": [l1.0, l2.0, l3.0]})).unwrap();
}

#[test]
fn shape_builder_gap_detection_closes_small_gaps_between_lines() {
    let mut s = session();
    lines_with_gaps(&mut s);
    let regions = |s: &mut Session, p: Value| s.execute("shapeBuilder.regions", &p).unwrap()["regions"].as_array().unwrap().len();
    assert_eq!(regions(&mut s, json!({})), 0, "off by default");
    assert_eq!(regions(&mut s, json!({"gap": 2})), 0, "the gap is longer");
    assert_eq!(regions(&mut s, json!({"gap": 3})), 1);
    assert!(s.execute("shapeBuilder.merge", &json!({"points": [[50, 70]]})).is_err(), "no gap, no region");
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[50, 70]], "gap": 3})).unwrap();
    let merged = NodeId(r["merged"].as_u64().unwrap());
    let a = area(&s, merged);
    assert!(a > 850.0 && a < 900.0, "the triangle and its two bridges: {a}");
    assert_eq!(ids(&r).len(), 4, "the bridges are no art: the merged shape and the three lines");
    assert_eq!(open_paths(&s, &ids(&r)).len(), 3);
}

#[test]
fn shape_builder_tool_gap_detection_option() {
    let mut s = session();
    lines_with_gaps(&mut s);
    let v = ViewInfo::default();
    s.select_tool("shapeBuilder", v).unwrap();
    assert_eq!((s.tool_options()["gapDetection"].clone(), s.tool_options()["gapLength"].clone()), (json!(false), json!(3.0)));
    let click = |s: &mut Session| {
        s.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 70.0), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, 50.0, 70.0), v).unwrap();
    };
    click(&mut s);
    assert_eq!(layer_children(&s), 3, "Gap Detection is off");
    s.set_tool_option_cmd(&json!({"values": {"gapDetection": true, "gapLength": 6.0}})).unwrap();
    click(&mut s);
    assert_eq!(layer_children(&s), 4, "the triangle");
    // The options are kept when another tool is chosen.
    s.select_tool("selection", v).unwrap();
    assert_eq!(
        (s.tool_options_of("shapeBuilder")["gapDetection"].clone(), s.tool_options_of("shapeBuilder")["gapLength"].clone()),
        (json!(true), json!(6.0))
    );
}

#[test]
fn shape_builder_closes_filled_open_paths() {
    let mut s = session();
    let r = s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}]})).unwrap();
    let p = NodeId(r["id"].as_u64().unwrap());
    set_fill(&mut s, p, Color::rgb(0.0, 1.0, 0.0));
    s.execute("select.set", &json!({"ids": [p.0]})).unwrap();
    let r = s.execute("shapeBuilder.regions", &json!({})).unwrap();
    assert_eq!(r["regions"].as_array().unwrap().len(), 1, "the fill's area, closed by an invisible edge");
    assert!((r["regions"][0]["area"].as_f64().unwrap() - 5000.0).abs() < 1e-6);
    assert_eq!(r["lines"], json!([]));
    // Unfilled, it is only an edge.
    s.edit("t", |d, _| {
        d.node_mut(p).unwrap().appearance.set_fill(Paint::None);
        Ok(())
    })
    .unwrap();
    let r = s.execute("shapeBuilder.regions", &json!({})).unwrap();
    assert_eq!(r["regions"], json!([]));
    assert_eq!(r["lines"].as_array().unwrap().len(), 1);
}

/// Consider Open Filled Paths as Closed off: a filled open path is only an edge, as an unfilled
/// one is, so it forms no region and a click inside it merges nothing.
#[test]
fn shape_builder_open_filled_paths_cut_without_open_filled_as_closed() {
    let mut s = session();
    let r = s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}]})).unwrap();
    let p = NodeId(r["id"].as_u64().unwrap());
    set_fill(&mut s, p, Color::rgb(0.0, 1.0, 0.0));
    s.execute("select.set", &json!({"ids": [p.0]})).unwrap();
    let r = s.execute("shapeBuilder.regions", &json!({"openFilledAsClosed": false})).unwrap();
    assert_eq!(r["regions"], json!([]));
    assert_eq!(r["lines"].as_array().unwrap().len(), 1);
    assert!(s.execute("shapeBuilder.merge", &json!({"points": [[75, 25]], "openFilledAsClosed": false})).is_err());
    // On (the default), the click makes the area the fill closes a shape.
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[75, 25]]})).unwrap();
    assert!((area(&s, NodeId(r["merged"].as_u64().unwrap())) - 5000.0).abs() < 1e-6);
}

/// Pick Color From Color Swatches: the merged shape takes the current fill, not the fill of the
/// object under the first point; setting the current fill without `ids` leaves the selection's.
#[test]
fn shape_builder_pick_color_from_swatches_fills_with_the_current_fill() {
    let mut s = session();
    let (a, _) = two(&mut s);
    s.execute("paint.setFill", &json!({"color": "#00ff00", "ids": []})).unwrap();
    assert_eq!(node(&s, a).appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), "A keeps its fill");
    let green = s.paint.fill.clone();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[25, 50], [75, 50]], "pickColorFrom": "swatches"})).unwrap();
    assert_eq!(node(&s, NodeId(r["merged"].as_u64().unwrap())).appearance.fill_paint(), green);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[25, 50], [75, 50]], "pickColorFrom": "artwork"})).unwrap();
    assert_eq!(node(&s, NodeId(r["merged"].as_u64().unwrap())).appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    assert!(s.execute("shapeBuilder.merge", &json!({"points": [[25, 50]], "pickColorFrom": "layers"})).is_err());
}

/// Four #888888 bars round a hole (20..80 on both axes), and in front of them a red square
/// (90..140 × 30..70) with a 5 pt stroke that overlaps the right bar. Returns the bars, back to
/// front, and the square.
fn frame_and_red_square(s: &mut Session) -> (Vec<NodeId>, NodeId) {
    let gray = Color::from_hex("#888888").unwrap();
    let mut bars = vec![];
    for (x, y, w, h) in [(0.0, 0.0, 20.0, 100.0), (80.0, 0.0, 20.0, 100.0), (0.0, 0.0, 100.0, 20.0), (0.0, 80.0, 100.0, 20.0)] {
        let id = rect(s, x, y, w, h);
        set_fill(s, id, gray);
        bars.push(id);
    }
    let red = rect(s, 90.0, 30.0, 50.0, 40.0);
    s.edit("t", |d, _| {
        let a = &mut d.node_mut(red).unwrap().appearance;
        a.set_fill(Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
        a.set_stroke(Paint::solid(Color::rgb(0.0, 0.0, 0.0)));
        a.stroke_mut().unwrap().width = 5.0;
        Ok(())
    })
    .unwrap();
    (bars, red)
}

fn select_ids(s: &mut Session, list: &[NodeId]) {
    s.execute("select.set", &json!({"ids": list.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn merged_look(s: &Session, r: &Value) -> vectorcraft_doc::Shared<vectorcraft_doc::Appearance> {
    node(s, NodeId(r["merged"].as_u64().unwrap())).appearance
}

/// Pick Color From Artwork: the merged shape takes the look of the object where the drag starts,
/// else of the object where it ends, else of the front-most selected object, as the combine page
/// states. A drag that starts in an object keeps that object's look.
#[test]
fn shape_builder_artwork_takes_the_look_where_the_drag_starts_then_ends_then_the_top_object() {
    let mut s = session();
    let (bars, red) = frame_and_red_square(&mut s);
    s.execute("paint.setFill", &json!({"color": "#00ff00", "ids": []})).unwrap();
    let gray = node(&s, bars[3]).appearance.clone();
    let red_look = node(&s, red).appearance.clone();
    let all: Vec<NodeId> = bars.iter().copied().chain([red]).collect();
    let merge = |s: &mut Session, list: &[NodeId], points: Value| {
        select_ids(s, list);
        let r = s.execute("shapeBuilder.merge", &json!({ "points": points })).unwrap();
        let look = merged_look(s, &r);
        s.execute("edit.undo", &json!({})).unwrap();
        (r["regions"].as_u64().unwrap(), look)
    };
    // A click in the hole, where no object is: the front-most selected object's look, not the
    // current fill.
    assert_eq!(merge(&mut s, &bars, json!([[50, 50]])), (1, gray.clone()), "the front bar's look");
    assert_eq!(merge(&mut s, &all, json!([[50, 50]])), (1, red_look.clone()), "the square is in front");
    // A drag from the hole across the right bar into the square: the look where it ends, not
    // the look of the first object it crosses.
    assert_eq!(merge(&mut s, &all, json!([[50, 50], [120, 50]])), (4, red_look.clone()));
    // Ending on the right bar, the drag takes the bar's look, though the square is in front.
    let right_bar = node(&s, bars[1]).appearance.clone();
    assert_eq!(merge(&mut s, &all, json!([[50, 50], [85, 50]])), (2, right_bar.clone()));
    // A drag from outside every region: the look where it ends.
    assert_eq!(merge(&mut s, &all, json!([[-10, 50], [120, 50]])).1, red_look);
    // A drag that starts on the right bar: the bar's look.
    assert_eq!(merge(&mut s, &all, json!([[85, 50], [120, 50]])).1, right_bar);
    // From the square back into the hole: the square's look.
    assert_eq!(merge(&mut s, &all, json!([[120, 50], [50, 50]])).1, red_look);

    // A drag from blue into red takes blue, where it starts.
    let mut s = session();
    let (_, b) = two(&mut s);
    let blue = node(&s, b).appearance.clone();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[125, 50], [25, 50]]})).unwrap();
    assert_eq!(merged_look(&s, &r), blue);
}

/// With Pick Color From Color Swatches, the merged shape takes the current fill, and the rest of
/// its look from the front-most object over the first region the drag touches that an object
/// covers.
#[test]
fn shape_builder_swatches_keep_the_current_fill_and_the_first_covered_objects_look() {
    let mut s = session();
    let (bars, red) = frame_and_red_square(&mut s);
    s.execute("paint.setFill", &json!({"color": "#00ff00", "ids": []})).unwrap();
    let green = s.paint.fill.clone();
    let bar = node(&s, bars[1]).appearance.clone();
    select_ids(&mut s, &bars.iter().copied().chain([red]).collect::<Vec<_>>());
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[50, 50], [120, 50]], "pickColorFrom": "swatches"})).unwrap();
    let look = merged_look(&s, &r);
    assert_eq!(look.fill_paint(), green);
    assert_eq!((look.stroke_paint(), look.stroke().map(|k| k.width)), (bar.stroke_paint(), bar.stroke().map(|k| k.width)), "the right bar's stroke");
}

/// The Shape Builder tool sends the point where the drag started first and the point where it
/// ended last, so a drag from the hole into the square takes the square's look.
#[test]
fn shape_builder_tool_drag_from_the_hole_takes_the_look_where_it_ends() {
    let mut s = session();
    let (bars, red) = frame_and_red_square(&mut s);
    let red_look = node(&s, red).appearance.clone();
    select_ids(&mut s, &bars.iter().copied().chain([red]).collect::<Vec<_>>());
    let v = ViewInfo::default();
    s.select_tool("shapeBuilder", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 50.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 85.0, 50.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 120.0, 50.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 120.0, 50.0), v).unwrap();
    assert_eq!(s.journal.last().unwrap().0, "shapeBuilder.merge");
    let d = &s.doc().unwrap().doc;
    let kids = d.children(d.default_layer()).unwrap();
    assert_eq!(kids.iter().filter(|n| n.appearance == red_look).count(), 1, "the merged shape has the square's look");
}

/// The Shape Builder Tool Options reach what the tool does: Straight Line sends the drag's first
/// and last points, and the other options go with them to `shapeBuilder.merge`.
#[test]
fn shape_builder_tool_options_reach_the_command() {
    let mut s = session();
    two(&mut s);
    let v = ViewInfo::default();
    s.select_tool("shapeBuilder", v).unwrap();
    let values = json!({"openFilledAsClosed": false, "pickColorFrom": "swatches", "selection": "straightLine"});
    s.set_tool_option_cmd(&json!({"tool": "shapeBuilder", "values": values})).unwrap();
    // A drag that bends up through (60, 20) on its way.
    for (kind, x, y) in
        [(PointerKind::Down, 25.0, 50.0), (PointerKind::Drag, 60.0, 20.0), (PointerKind::Drag, 75.0, 50.0), (PointerKind::Up, 75.0, 50.0)]
    {
        s.pointer(&PointerEvent::new(kind, x, y), v).unwrap();
    }
    let (cmd, p) = s.journal.last().unwrap().clone();
    assert_eq!(cmd, "shapeBuilder.merge");
    assert_eq!(
        (&p["points"], &p["openFilledAsClosed"], &p["pickColorFrom"]),
        (&json!([[25.0, 50.0], [75.0, 50.0]]), &json!(false), &json!("swatches"))
    );
}

/// A Shift-drag with the tool across the grid merges all nine squares into one shape; a drag along
/// the middle row without Shift merges the three squares it passes over.
#[test]
fn shape_builder_tool_shift_drag_merges_the_grid_into_one_shape() {
    let v = ViewInfo::default();
    let drag = |s: &mut Session, mods: Mods, pts: [(f64, f64); 3]| {
        s.select_tool("shapeBuilder", v).unwrap();
        for (kind, (x, y)) in [PointerKind::Down, PointerKind::Drag, PointerKind::Up].into_iter().zip(pts) {
            s.pointer(&PointerEvent::new(kind, x, y).with_mods(mods), v).unwrap();
        }
        s.journal.last().unwrap().clone()
    };
    let mut s = session();
    grid(&mut s);
    let (cmd, p) = drag(&mut s, Mods { shift: true, ..Default::default() }, [(50.0, 50.0), (150.0, 150.0), (250.0, 250.0)]);
    assert_eq!((cmd.as_str(), &p["rect"]), ("shapeBuilder.merge", &json!([50.0, 50.0, 200.0, 200.0])));
    assert_eq!(layer_children(&s), 1);
    let only = {
        let d = &s.doc().unwrap().doc;
        d.children(d.default_layer()).unwrap()[0].id
    };
    assert!((area(&s, only) - 90000.0).abs() < 1.0, "{}", area(&s, only));
    let mut s = session();
    grid(&mut s);
    let (_, p) = drag(&mut s, Mods::default(), [(50.0, 150.0), (150.0, 150.0), (250.0, 150.0)]);
    assert_eq!((p.get("rect"), &p["points"]), (None, &json!([[50.0, 150.0], [150.0, 150.0], [250.0, 150.0]])));
    assert_eq!(layer_children(&s), 7, "the middle row merged, six squares left");
}

/// Highlight Fill off: hovering a region in merge mode highlights nothing; erase mode still shows
/// what a click would delete.
#[test]
fn shape_builder_highlight_fill_off_leaves_regions_unshaded() {
    let mut s = session();
    two(&mut s);
    let v = ViewInfo::default();
    s.select_tool("shapeBuilder", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Move, 75.0, 50.0), v).unwrap();
    assert!(!s.overlays(v).is_empty());
    s.set_tool_option_cmd(&json!({"key": "highlightFill", "value": false})).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Move, 76.0, 50.0), v).unwrap();
    assert!(s.overlays(v).is_empty());
    let alt = Mods { alt: true, ..Default::default() };
    s.pointer(&PointerEvent::new(PointerKind::Move, 75.0, 50.0).with_mods(alt), v).unwrap();
    assert!(!s.overlays(v).is_empty());
}

/// Pick Color From Color Swatches: Left and Right step the current fill through the document's
/// color swatches (the selection keeps its fills), and Cursor Swatch Preview shows the current fill
/// between the swatches before and after it.
#[test]
fn shape_builder_arrow_keys_step_the_current_fill_through_the_swatches() {
    let mut s = session();
    let (a, _) = two(&mut s);
    let v = ViewInfo::default();
    s.select_tool("shapeBuilder", v).unwrap();
    let chips = |s: &mut Session| s.overlays(v).iter().filter(|o| matches!(o, vectorcraft_tools::Overlay::Swatch { .. })).count();
    assert!(!s.tool_claims_key(ToolKey::Right, v), "Artwork leaves the arrows to nudging");
    let values = json!({"pickColorFrom": "swatches", "cursorSwatchPreview": true});
    s.set_tool_option_cmd(&json!({"tool": "shapeBuilder", "values": values})).unwrap();
    assert!(s.tool_claims_key(ToolKey::Right, v));
    let (first, swatches): (String, Vec<Paint>) = {
        let list = vectorcraft_tools::builder::color_swatches(&s.doc().unwrap().doc);
        (list[0].name.clone(), list.iter().take(3).map(|sw| sw.paint.clone()).collect())
    };
    assert_eq!(swatches.len(), 3);
    s.execute("paint.setFill", &json!({"swatch": first, "ids": []})).unwrap();
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    assert_eq!(s.paint.fill, swatches[2]);
    s.tool_key(ToolKey::Left, Mods::default(), v).unwrap();
    assert_eq!(s.paint.fill, swatches[1]);
    assert_eq!(node(&s, a).appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), "A keeps its fill");
    s.pointer(&PointerEvent::new(PointerKind::Move, 75.0, 50.0), v).unwrap();
    assert_eq!(chips(&mut s), 3);
    s.set_tool_option_cmd(&json!({"key": "cursorSwatchPreview", "value": false})).unwrap();
    assert_eq!(chips(&mut s), 0);
}

/// Two swatches of the same color next to each other: the arrow keys step through both and on to
/// the swatch after them.
#[test]
fn shape_builder_arrow_keys_pass_swatches_of_the_same_color() {
    let mut s = session();
    two(&mut s);
    for name in ["Twin A", "Twin B"] {
        s.execute("swatch.new", &json!({"name": name, "color": "#123456"})).unwrap();
    }
    let v = ViewInfo::default();
    s.select_tool("shapeBuilder", v).unwrap();
    s.set_tool_option_cmd(&json!({"tool": "shapeBuilder", "values": {"pickColorFrom": "swatches"}})).unwrap();
    let names: Vec<String> = vectorcraft_tools::builder::color_swatches(&s.doc().unwrap().doc).iter().map(|sw| sw.name.clone()).collect();
    let a = names.iter().position(|n| n == "Twin A").unwrap();
    assert_eq!(names[a + 1], "Twin B");
    s.execute("paint.setFill", &json!({"swatch": "Twin A", "ids": []})).unwrap();
    let twin = s.paint.fill.clone();
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    assert_eq!(s.paint.fill, twin, "Twin B");
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    let after = vectorcraft_tools::builder::color_swatches(&s.doc().unwrap().doc)[(a + 2) % names.len()].paint.clone();
    assert_eq!(s.paint.fill, after, "the swatch after Twin B");
}

#[test]
fn shape_builder_tool_alt_deletes_line_pieces() {
    let mut s = session();
    square_and_line(&mut s);
    let v = ViewInfo::default();
    s.select_tool("shapeBuilder", v).unwrap();
    let alt = Mods { alt: true, ..Default::default() };
    s.pointer(&PointerEvent::new(PointerKind::Move, 51.0, 50.0).with_mods(alt), v).unwrap();
    assert_eq!(s.overlays(v).len(), 1, "the line piece is highlighted");
    s.pointer(&PointerEvent::new(PointerKind::Down, 51.0, 50.0).with_mods(alt), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 51.0, 50.0).with_mods(alt), v).unwrap();
    assert_eq!(s.journal.last().unwrap().0, "shapeBuilder.merge");
    assert_eq!(layer_children(&s), 3, "square + the line's two loose ends");
}

/// Red square A (0..100) and blue square B (50..150 both ways), overlapping at a corner, selected.
fn corner(s: &mut Session) -> (NodeId, NodeId) {
    let a = rect(s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(s, 50.0, 50.0, 100.0, 100.0);
    set_fill(s, a, Color::rgb(1.0, 0.0, 0.0));
    set_fill(s, b, Color::rgb(0.0, 0.0, 1.0));
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    (a, b)
}

/// (closed, anchors, length) of each of `ids`.
fn shapes_of(s: &Session, ids: &[NodeId]) -> Vec<(bool, usize, f64)> {
    ids.iter()
        .map(|i| {
            let (p, _) = outline(&node(s, *i));
            (p.is_closed(), p.anchor_count(), (p.length() * 1e6).round() / 1e6)
        })
        .collect()
}

#[test]
fn shape_builder_erase_click_on_an_edge_opens_its_path() {
    let mut s = session();
    corner(&mut s);
    let r = s.execute("shapeBuilder.regions", &json!({})).unwrap();
    assert_eq!(r["edges"].as_array().unwrap().len(), 4, "each outline cut in two where the other crosses it: {r}");
    // A's side inside B (1 pt off it).
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[101, 75]], "erase": true})).unwrap();
    assert_eq!((r["regions"].as_u64(), r["lines"].as_u64(), r["edges"].as_u64()), (Some(0), Some(0), Some(1)));
    let out = ids(&r);
    // A opens where B's outline crosses it and keeps its look; B is untouched.
    assert_eq!(shapes_of(&s, &out), vec![(false, 5, 300.0), (true, 4, 400.0)]);
    assert_eq!(node(&s, out[0]).appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    assert_eq!(s.journal.last().unwrap().0, "shapeBuilder.merge");
    s.execute("edit.undo", &json!({})).unwrap();
    let back: Vec<NodeId> = s.doc().unwrap().doc.children(s.doc().unwrap().doc.default_layer()).unwrap().iter().map(|n| n.id).collect();
    assert_eq!(shapes_of(&s, &back), vec![(true, 4, 400.0); 2], "one undo step");
}

/// An even-odd compound path opened by an erase click on one of its edges keeps its fill rule.
#[test]
fn shape_builder_erase_click_on_an_edge_keeps_the_fill_rule() {
    let mut s = session();
    let c = even_odd_ring(&mut s);
    let b = rect(&mut s, 50.0, 50.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [c.0, b.0]})).unwrap();
    // A's side inside B (1 pt off it).
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[101, 75]], "erase": true})).unwrap();
    assert_eq!((r["regions"].as_u64(), r["edges"].as_u64()), (Some(0), Some(1)));
    let out = ids(&r);
    assert_eq!(shapes_of(&s, &out[..1]), vec![(false, 9, 500.0)], "A open where B crosses it, with H");
    assert_eq!(outline(&node(&s, out[0])).1, FillRule::EvenOdd);
}

#[test]
fn shape_builder_erase_drag_along_edges_deletes_each() {
    let mut s = session();
    corner(&mut s);
    // Down A's side inside B, round the corner and along B's side inside A.
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[100, 75], [100, 50], [75, 50]], "erase": true})).unwrap();
    assert_eq!((r["regions"].as_u64(), r["edges"].as_u64()), (Some(0), Some(2)));
    assert_eq!(shapes_of(&s, &ids(&r)), vec![(false, 5, 300.0); 2]);
}

#[test]
fn shape_builder_erase_drag_into_regions_deletes_them_not_the_edges_it_crosses() {
    let mut s = session();
    corner(&mut s);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[25, 25], [75, 75]], "erase": true})).unwrap();
    assert_eq!((r["regions"].as_u64(), r["edges"].as_u64()), (Some(2), Some(0)));
    // A is gone; what is left of B stays closed and blue.
    let out = ids(&r);
    assert_eq!(out.len(), 1);
    assert!((area(&s, out[0]) - 7500.0).abs() < 1e-6);
    assert!(outline(&node(&s, out[0])).0.is_closed());
    assert_eq!(node(&s, out[0]).appearance.fill_paint(), Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
}

#[test]
fn shape_builder_erase_drag_deletes_an_overlap_narrower_than_twice_the_tolerance() {
    let mut s = session();
    // A 1.5 pt overlap: every point of it is within the 2 pt tolerance of its edges.
    two_at(&mut s, 98.5);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[50, 50], [150, 50]], "erase": true})).unwrap();
    assert_eq!((r["regions"].as_u64(), r["edges"].as_u64()), (Some(3), Some(0)), "{r}");
    assert_eq!(layer_children(&s), 0, "no strip is left");
}

#[test]
fn shape_builder_erase_click_or_drag_from_inside_a_narrow_overlap_deletes_it() {
    for points in [json!([[99.2, 50]]), json!([[99.2, 50], [150, 50]]), json!([[95, 10], [103.5, 60]])] {
        let mut s = session();
        two_at(&mut s, 98.5);
        let r = s.execute("shapeBuilder.merge", &json!({"points": points, "erase": true})).unwrap();
        assert_eq!(r["edges"].as_u64(), Some(0), "{points}: {r}");
        let d = &s.doc().unwrap().doc;
        let left: Vec<f64> = d.children(d.default_layer()).unwrap().iter().filter_map(|c| c.geometric_bounds()).map(|b| b.width()).collect();
        assert!(left.iter().all(|w| *w > 2.0), "{points}: no strip is left: {left:?}");
    }
}

/// In Merge Mode, Clicking Stroke Splits the Path: a click on a lone rectangle's top side, 1 pt
/// outside it, splits the rectangle into that side and the rest. Without the option, the same click
/// is refused as outside every region and leaves the rectangle as it was.
#[test]
fn shape_builder_click_on_a_stroke_splits_its_path() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    set_fill(&mut s, a, Color::rgb(1.0, 0.0, 0.0));
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let undo_before = s.doc().unwrap().history.undo.len();
    assert!(s.execute("shapeBuilder.merge", &json!({"points": [[50, -1]]})).is_err());
    assert!(s.execute("shapeBuilder.merge", &json!({"points": [[50, -1]], "splitStroke": false})).is_err());
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_before);
    assert_eq!(shapes_of(&s, &[a]), vec![(true, 4, 400.0)]);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[50, -1]], "splitStroke": true})).unwrap();
    let out = ids(&r);
    assert_eq!((r["merged"].clone(), r["regions"].as_u64(), r["edges"].as_u64()), (Value::Null, Some(0), Some(1)));
    // The rest of the outline (open, the other three sides) comes first, then the top side in
    // front of it; both are red.
    assert_eq!(shapes_of(&s, &out), vec![(false, 4, 300.0), (false, 2, 100.0)]);
    let split = NodeId(r["split"].as_u64().unwrap());
    assert_eq!(split, out[1]);
    assert_eq!(node(&s, split).geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 100.0, 0.0));
    assert!(out.iter().all(|i| node(&s, *i).appearance.fill_paint() == Paint::solid(Color::rgb(1.0, 0.0, 0.0))));
    assert_eq!(s.doc().unwrap().selection.objects, out);
    // One undo step puts the rectangle back.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(layer_children(&s), 1);
    assert_eq!(shapes_of(&s, &[a]), vec![(true, 4, 400.0)]);
}

/// A click on a stroke that is the whole of its path (a lone line, or a filled open path of one
/// curve) has nothing to split off: with splitStroke it is refused as outside every region, as it
/// is without the option, and the path keeps its geometry and its stroke.
#[test]
fn shape_builder_click_on_a_whole_open_path_does_not_split_it() {
    let mut s = session();
    let l = line(&mut s, (0.0, 0.0), (100.0, 0.0));
    let r = s.execute("path.create", &json!({"anchors": [{"x": 200, "y": 0, "out": [200, -100]}, {"x": 300, "y": 0, "in": [300, -100]}]})).unwrap();
    let arch = NodeId(r["id"].as_u64().unwrap());
    set_fill(&mut s, arch, Color::rgb(1.0, 0.0, 0.0));
    let strokes = |s: &Session| [l, arch].map(|i| node(s, i).appearance.stroke_paint());
    let before = (shapes_of(&s, &[l, arch]), strokes(&s));
    let undo_before = s.doc().unwrap().history.undo.len();
    // The line 0.5 pt off it, then the top of the arch 1 pt above it.
    for (id, at) in [(l, [50.0, 0.5]), (arch, [250.0, -76.0])] {
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        let r = s.execute("shapeBuilder.merge", &json!({"points": [at], "splitStroke": true, "fill": "#00ff00"}));
        let err = r.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(err.contains("no region under the pointer"), "{id:?}: {err}");
    }
    assert_eq!(layer_children(&s), 2);
    assert_eq!((shapes_of(&s, &[l, arch]), strokes(&s)), before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_before);
}

/// A click on a stroke whose run would leave no more of the path than the cut tolerance (0.05 pt
/// for a 100 pt path) does not split it: here the line's last 0.01 pt is all that would be left.
/// The command refuses the click as outside every region, and the tool shows no scissors and no
/// highlight there and runs no command on a click.
#[test]
fn shape_builder_click_on_a_stroke_does_not_split_off_all_but_a_speck() {
    let mut s = session();
    let r = s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100.01, "y": 0}]})).unwrap();
    let p = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [p.0]})).unwrap();
    let undo_before = s.doc().unwrap().history.undo.len();
    let err = s.execute("shapeBuilder.merge", &json!({"points": [[50, 1]], "splitStroke": true})).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("no region under the pointer"), "{err}");
    assert_eq!((layer_children(&s), shapes_of(&s, &[p])), (1, vec![(false, 3, 100.01)]));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_before);
    let v = ViewInfo::default();
    s.select_tool("shapeBuilder", v).unwrap();
    s.set_tool_option_cmd(&json!({"values": {"clickStrokeSplitsPath": true}})).unwrap();
    let journal = s.journal.len();
    s.pointer(&PointerEvent::new(PointerKind::Move, 50.0, 1.0), v).unwrap();
    assert!(s.overlays(v).is_empty(), "{:?}", s.overlays(v));
    assert_eq!(s.cursor(vectorcraft_geom::Point::new(50.0, 1.0), Mods::default(), v), vectorcraft_tools::Cursor::ShapeBuilder);
    s.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 1.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 50.0, 1.0), v).unwrap();
    assert_eq!((s.journal.len(), layer_children(&s)), (journal, 1), "the click runs no command");
}

/// The stroke a click splits ends at the points where other paths cross it as well as at the
/// path's anchors: A's right side inside B, then a line piece inside a square. A click away from
/// every stroke merges and an erase click deletes, as they do without the option.
#[test]
fn shape_builder_stroke_split_stops_at_crossings_and_anchors() {
    let mut s = session();
    let (a, b) = corner(&mut s);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[101, 75]], "splitStroke": true})).unwrap();
    let out = ids(&r);
    assert_eq!(shapes_of(&s, &out), vec![(false, 5, 350.0), (false, 2, 50.0), (true, 4, 400.0)]);
    assert_eq!(node(&s, out[1]).geometric_bounds().unwrap(), Rect::new(100.0, 50.0, 100.0, 100.0));
    assert_eq!(node(&s, out[2]).appearance.fill_paint(), Paint::solid(Color::rgb(0.0, 0.0, 1.0)), "B as it was");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    // A click inside a region away from the strokes merges it; erase mode deletes the edge.
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[75, 75]], "splitStroke": true})).unwrap();
    assert_eq!((r["regions"].as_u64(), r.get("split")), (Some(1), None));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[101, 75]], "erase": true, "splitStroke": true})).unwrap();
    assert_eq!(shapes_of(&s, &ids(&r)), vec![(false, 5, 300.0), (true, 4, 400.0)]);

    // A click on the line inside the square splits off the piece between the square's sides and
    // leaves the square whole.
    let mut s = session();
    square_and_line(&mut s);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[51, 50]], "splitStroke": true})).unwrap();
    let out = ids(&r);
    assert_eq!(r["lines"].as_u64(), Some(1));
    let split = NodeId(r["split"].as_u64().unwrap());
    assert_eq!(node(&s, split).geometric_bounds().unwrap(), Rect::new(50.0, 0.0, 50.0, 100.0));
    let lengths: Vec<f64> = out.iter().map(|i| (outline(&node(&s, *i)).0.length() * 1e6).round() / 1e6).collect();
    assert_eq!(lengths, vec![400.0, 40.0, 100.0], "the square, the line's two loose ends, the piece");
}

/// The run a click splits off goes the way its path goes, so arrowheads and the other settings that
/// follow a stroke's direction stay at the ends they were at: a line and a rectangle, each as drawn
/// and reversed.
#[test]
fn shape_builder_stroke_split_keeps_the_path_direction() {
    let ends = |s: &Session, id: NodeId| {
        let (p, _) = outline(&node(s, id));
        let sp = &p.subpaths[0];
        (sp.anchors.first().unwrap().p, sp.anchors.last().unwrap().p)
    };
    let pt = vectorcraft_geom::Point::new;
    for reverse in [false, true] {
        let mut s = session();
        let (a, l) = square_and_line(&mut s);
        if reverse {
            s.execute("path.reverse", &json!({"ids": [l.0]})).unwrap();
            s.execute("select.set", &json!({"ids": [a.0, l.0]})).unwrap();
        }
        let r = s.execute("shapeBuilder.merge", &json!({"points": [[51, 50]], "splitStroke": true})).unwrap();
        let split = NodeId(r["split"].as_u64().unwrap());
        let want = if reverse { (pt(50.0, 100.0), pt(50.0, 0.0)) } else { (pt(50.0, 0.0), pt(50.0, 100.0)) };
        assert_eq!(ends(&s, split), want, "the line, reversed: {reverse}");

        let mut s = session();
        let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
        if reverse {
            s.execute("path.reverse", &json!({"ids": [a.0]})).unwrap();
        }
        s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
        let (p, _) = outline(&node(&s, a));
        let sp = &p.subpaths[0];
        let n = sp.anchors.len();
        // The rectangle's top side, from the anchor the path reaches it at.
        let want = (0..n).map(|k| (sp.anchors[k].p, sp.anchors[(k + 1) % n].p)).find(|(x, y)| x.y == 0.0 && y.y == 0.0).unwrap();
        let r = s.execute("shapeBuilder.merge", &json!({"points": [[50, -1]], "splitStroke": true})).unwrap();
        let split = NodeId(r["split"].as_u64().unwrap());
        assert_eq!(ends(&s, split), want, "the rectangle, reversed: {reverse}");
    }
}

/// A split replaces only the path it splits: the two paths take its place in the stacking order and
/// in its group, and the other selected objects stay as they are. A path in a Live Paint group does
/// not split.
#[test]
fn shape_builder_stroke_split_leaves_the_other_objects_in_place() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 300.0, 0.0, 100.0, 100.0);
    let c = rect(&mut s, 0.0, 200.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0, c.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[50, -1]], "splitStroke": true})).unwrap();
    let split = NodeId(r["split"].as_u64().unwrap());
    let order = |s: &Session| -> Vec<NodeId> {
        let d = &s.doc().unwrap().doc;
        d.children(d.default_layer()).unwrap().iter().map(|n| n.id).collect()
    };
    let o = order(&s);
    assert_eq!((o.len(), o[1], o[2], o[3]), (4, split, b, c), "the rest and the run where A was, under B and C");
    assert_eq!(ids(&r), vec![o[0], split, c]);
    assert_eq!(s.doc().unwrap().selection.objects, ids(&r));
    // In a group, the two paths stay in the group, and the group stays selected.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, c.0]})).unwrap();
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[50, -1]], "splitStroke": true})).unwrap();
    assert_eq!(ids(&r), vec![g]);
    assert_eq!(node(&s, g).children().unwrap().len(), 3);
    assert_eq!(order(&s), vec![b, g]);
    // A Live Paint group's edge: the click merges as it does without the option.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let lp = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[50, 1]], "splitStroke": true})).unwrap();
    assert_eq!((r.get("split"), r["regions"].as_u64()), (None, Some(1)));
    assert!(s.doc().unwrap().doc.node(lp).is_none(), "merged as before");
}

/// A split and an erase click cut a small path with a tolerance from its own size, however far
/// away the other selected objects lie.
#[test]
fn shape_builder_split_and_erase_cut_a_small_path_beside_a_far_object() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 5.0, 5.0, 10.0, 10.0);
    let far = rect(&mut s, 1_000_000.0, 0.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [a.0, far.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[5, -0.5]], "splitStroke": true})).unwrap();
    let split = NodeId(r["split"].as_u64().unwrap_or_else(|| panic!("no split: {r}")));
    assert_eq!(node(&s, split).geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 10.0, 0.0));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0, far.0]})).unwrap();
    // A's side inside B (0.5 pt off it).
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[10.5, 7.5]], "erase": true})).unwrap();
    assert_eq!(shapes_of(&s, &ids(&r)[..1]), vec![(false, 5, 30.0)], "A open where B crosses it");
}

/// Pick Color From Color Swatches: the stroke split off takes the current fill as its stroke.
#[test]
fn shape_builder_stroke_split_with_swatches_paints_the_stroke() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let before = node(&s, a).appearance.stroke_paint();
    s.execute("paint.setFill", &json!({"color": "#00ff00", "ids": []})).unwrap();
    let green = s.paint.fill.clone();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[50, -1]], "splitStroke": true, "pickColorFrom": "swatches"})).unwrap();
    let out = ids(&r);
    assert_eq!(node(&s, NodeId(r["split"].as_u64().unwrap())).appearance.stroke_paint(), green);
    assert_eq!(node(&s, out[0]).appearance.stroke_paint(), before, "the rest keeps its stroke");
}

/// Gives `id` a red-to-blue linear gradient, fitted to its bounds, as its fill and its stroke, and
/// returns that fill and stroke fixed where they fit `id` now.
fn paint_with_gradients(s: &mut Session, id: NodeId) -> (Paint, Paint) {
    use vectorcraft_color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    s.edit("t", |d, _| {
        let a = &mut d.node_mut(id).unwrap().appearance;
        a.set_fill(gradient.clone());
        a.set_stroke(gradient);
        Ok(())
    })
    .unwrap();
    let mut n = node(s, id);
    n.pin_gradients();
    (n.appearance.fill_paint(), n.appearance.stroke_paint())
}

/// The two paths a split leaves keep the gradients of the path they come from where they were on
/// it: a gradient fitted to the rectangle's bounds is not fitted again to the side split off or to
/// what is left.
#[test]
fn shape_builder_stroke_split_keeps_the_gradients_where_they_were() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let want = paint_with_gradients(&mut s, a);
    // The rectangle's left side.
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[-1, 50]], "splitStroke": true})).unwrap();
    let out = ids(&r);
    assert_eq!(node(&s, out[1]).geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 0.0, 100.0));
    for i in out {
        let n = node(&s, i);
        assert_eq!((n.appearance.fill_paint(), n.appearance.stroke_paint()), want, "{i:?}");
    }
}

/// A red even-odd compound path, square A (0..100) with the square H (25..75) inside it, selected.
fn even_odd_ring(s: &mut Session) -> NodeId {
    let a = rect(s, 0.0, 0.0, 100.0, 100.0);
    let h = rect(s, 25.0, 25.0, 50.0, 50.0);
    set_fill(s, a, Color::rgb(1.0, 0.0, 0.0));
    s.execute("select.set", &json!({"ids": [a.0, h.0]})).unwrap();
    let c = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("path.setFillRule", &json!({"rule": "evenOdd"})).unwrap();
    assert_eq!(outline(&node(s, c)).1, FillRule::EvenOdd);
    c
}

/// A path split off an even-odd compound path, and the rest of it, keep its fill rule.
#[test]
fn shape_builder_stroke_split_keeps_the_fill_rule() {
    let mut s = session();
    even_odd_ring(&mut s);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[50, -1]], "splitStroke": true})).unwrap();
    let out = ids(&r);
    assert_eq!(shapes_of(&s, &out), vec![(false, 8, 500.0), (false, 2, 100.0)], "A's other three sides with H, then A's top side");
    assert_eq!(out.iter().map(|i| outline(&node(&s, *i)).1).collect::<Vec<_>>(), vec![FillRule::EvenOdd; 2]);
}

/// A split, and an Alt-click that opens a path on one of its edges, keep the path's name and opacity
/// mask on what is left of it.
#[test]
fn shape_builder_stroke_split_and_erase_keep_the_name_and_opacity_mask() {
    let mut s = session();
    let (a, b) = corner(&mut s);
    let m = rect(&mut s, 0.0, 0.0, 50.0, 50.0);
    s.execute("transparency.makeOpacityMask", &json!({"ids": [a.0, m.0]})).unwrap();
    s.edit("t", |d, _| {
        d.node_mut(a).unwrap().name = Some("A".into());
        Ok(())
    })
    .unwrap();
    let mask = node(&s, a).extras().mask.clone();
    assert!(mask.is_some());
    let kept = |s: &Session, ids: &[NodeId]| {
        ids.iter().all(|i| {
            let n = node(s, *i);
            n.name.as_deref() == Some("A") && n.extras().mask == mask
        })
    };
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[50, -1]], "splitStroke": true})).unwrap();
    let out = ids(&r);
    assert_eq!(out.len(), 2);
    assert!(kept(&s, &out), "the rest and the run");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[101, 75]], "erase": true})).unwrap();
    let out = ids(&r);
    assert_eq!(shapes_of(&s, &out[..1]), vec![(false, 5, 300.0)]);
    assert!(kept(&s, &out[..1]), "A opened where B crosses it");
}

/// What is left of a path after an Alt-click or a merge, and the pieces of an open path that an
/// Alt-click leaves, keep the path's gradients where they were on it: A without its region outside
/// B, B without the regions merged into A, and the line without its piece inside the square.
#[test]
fn shape_builder_erase_and_merge_keep_the_gradients_where_they_were() {
    let mut s = session();
    let (a, b) = two(&mut s);
    let want = paint_with_gradients(&mut s, a);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[25, 50]], "erase": true})).unwrap();
    let out = ids(&r);
    assert_eq!(node(&s, out[0]).geometric_bounds().unwrap(), Rect::new(50.0, 0.0, 100.0, 100.0), "A inside B");
    assert_eq!((node(&s, out[0]).appearance.fill_paint(), node(&s, out[0]).appearance.stroke_paint()), want);

    let mut s = session();
    let (a, b) = two(&mut s);
    let want = paint_with_gradients(&mut s, b);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[25, 50], [75, 50]]})).unwrap();
    let merged = NodeId(r["merged"].as_u64().unwrap());
    let rest = ids(&r).into_iter().find(|i| *i != merged).unwrap();
    assert_eq!(node(&s, rest).geometric_bounds().unwrap(), Rect::new(100.0, 0.0, 150.0, 100.0), "B outside A");
    assert_eq!((node(&s, rest).appearance.fill_paint(), node(&s, rest).appearance.stroke_paint()), want);

    let mut s = session();
    let (a, l) = square_and_line(&mut s);
    let want = paint_with_gradients(&mut s, l);
    s.execute("select.set", &json!({"ids": [a.0, l.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[51, 50]], "erase": true})).unwrap();
    let ends: Vec<NodeId> = ids(&r).into_iter().filter(|i| !outline(&node(&s, *i)).0.is_closed()).collect();
    assert_eq!(ends.len(), 2, "the line's two loose ends");
    for i in ends {
        let n = node(&s, i);
        assert_eq!((n.appearance.fill_paint(), n.appearance.stroke_paint()), want, "{i:?}");
    }
}

/// The tool with In Merge Mode, Clicking Stroke Splits the Path: over a stroke, the pointer shows
/// scissors and the run a click splits is drawn in the highlight color (none with Highlight Stroke
/// When Editable off), and a click splits it. Off, the same click leaves the rectangle as it was.
#[test]
fn shape_builder_tool_click_on_a_stroke_splits_with_the_option() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("shapeBuilder", v).unwrap();
    let o = s.tool_options();
    assert_eq!((&o["clickStrokeSplitsPath"], &o["highlightStroke"], &o["highlightColor"]), (&json!(false), &json!(true), &json!("#ff4f4f")));
    let click = |s: &mut Session| {
        s.pointer(&PointerEvent::new(PointerKind::Down, 50.0, -1.0), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, 50.0, -1.0), v).unwrap();
    };
    let journal = s.journal.len();
    s.pointer(&PointerEvent::new(PointerKind::Move, 50.0, -1.0), v).unwrap();
    assert!(s.overlays(v).is_empty());
    click(&mut s);
    assert_eq!((s.journal.len(), layer_children(&s)), (journal, 1), "off, the click runs no command");
    s.set_tool_option_cmd(&json!({"values": {"clickStrokeSplitsPath": true, "highlightColor": "Light Blue"}})).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Move, 50.0, -1.0), v).unwrap();
    let ov = s.overlays(v);
    let [vectorcraft_tools::Overlay::Path { path, color: [0x4f, 0x80, 0xff], .. }] = ov.as_slice() else { panic!("{ov:?}") };
    assert_eq!(vectorcraft_geom::Shape::bounding_box(path), Rect::new(0.0, 0.0, 100.0, 0.0));
    assert_eq!(s.cursor(vectorcraft_geom::Point::new(50.0, -1.0), Mods::default(), v), vectorcraft_tools::Cursor::Scissors);
    s.set_tool_option_cmd(&json!({"values": {"highlightStroke": false}})).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Move, 50.0, -1.0), v).unwrap();
    assert!(s.overlays(v).is_empty());
    click(&mut s);
    let (cmd, p) = s.journal.last().unwrap().clone();
    assert_eq!((cmd.as_str(), &p["splitStroke"], &p["points"]), ("shapeBuilder.merge", &json!(true), &json!([[50.0, -1.0]])));
    assert_eq!(layer_children(&s), 2);
    // The options are kept when another tool is chosen.
    s.select_tool("selection", v).unwrap();
    let o = s.tool_options_of("shapeBuilder");
    assert_eq!((&o["clickStrokeSplitsPath"], &o["highlightStroke"], &o["highlightColor"]), (&json!(true), &json!(false), &json!("#4f80ff")));
}

/// The tool with In Merge Mode, Clicking Stroke Splits the Path over a lone line, which a click
/// cannot split: the pointer does not show scissors, the line is not highlighted, and a click runs no
/// command.
#[test]
fn shape_builder_tool_does_not_offer_to_split_a_lone_line() {
    let mut s = session();
    let l = line(&mut s, (0.0, 0.0), (100.0, 0.0));
    s.execute("select.set", &json!({"ids": [l.0]})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("shapeBuilder", v).unwrap();
    s.set_tool_option_cmd(&json!({"values": {"clickStrokeSplitsPath": true}})).unwrap();
    let journal = s.journal.len();
    s.pointer(&PointerEvent::new(PointerKind::Move, 50.0, 1.0), v).unwrap();
    assert!(s.overlays(v).is_empty(), "{:?}", s.overlays(v));
    assert_eq!(s.cursor(vectorcraft_geom::Point::new(50.0, 1.0), Mods::default(), v), vectorcraft_tools::Cursor::ShapeBuilder);
    s.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 1.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 50.0, 1.0), v).unwrap();
    assert_eq!((s.journal.len(), layer_children(&s)), (journal, 1), "the click runs no command");
}

/// The tool over a side shorter than the cut tolerance (0.05 pt for a 100 pt path), which
/// `shapeBuilder.merge` cannot cut out: the pointer does not show scissors, the side is not
/// highlighted, and a click outside the path runs no command.
#[test]
fn shape_builder_tool_does_not_offer_to_split_a_side_too_short_to_cut() {
    let mut s = session();
    let anchors = [(0.0, 0.0), (100.0, 0.0), (100.0, 0.03), (100.0, 100.0), (0.0, 100.0)].map(|(x, y)| json!({"x": x, "y": y}));
    let r = s.execute("path.create", &json!({"anchors": anchors, "closed": true})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("shapeBuilder", v).unwrap();
    s.set_tool_option_cmd(&json!({"values": {"clickStrokeSplitsPath": true}})).unwrap();
    let journal = s.journal.len();
    let at = (101.0, 0.015);
    s.pointer(&PointerEvent::new(PointerKind::Move, at.0, at.1), v).unwrap();
    assert!(s.overlays(v).is_empty(), "{:?}", s.overlays(v));
    assert_eq!(s.cursor(vectorcraft_geom::Point::new(at.0, at.1), Mods::default(), v), vectorcraft_tools::Cursor::ShapeBuilder);
    s.pointer(&PointerEvent::new(PointerKind::Down, at.0, at.1), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, at.0, at.1), v).unwrap();
    assert_eq!((s.journal.len(), layer_children(&s)), (journal, 1), "the click runs no command");
}

// ---------- Live Paint ----------

fn live(s: &mut Session) -> NodeId {
    two(s);
    let r = s.execute("livePaint.make", &json!({})).unwrap();
    assert_eq!(r["faces"], 3);
    assert!(r["edges"].as_u64().unwrap() >= 4);
    NodeId(r["id"].as_u64().unwrap())
}

#[test]
fn live_paint_make_builds_group_with_hidden_sources() {
    let mut s = session();
    let g = live(&mut s);
    let n = node(&s, g);
    assert!(vectorcraft_tools::builder::is_live_paint(&n));
    let src = vectorcraft_tools::builder::sources(&n).unwrap();
    assert!(!src.visible);
    assert_eq!(src.children().unwrap().len(), 2);
    assert_eq!(layer_children(&s), 1);
    assert_eq!(s.doc().unwrap().selection.objects, vec![g]);
    // Faces start with the fill of the top-most covering object.
    let info = s.execute("livePaint.info", &json!({"group": g.0})).unwrap();
    assert_eq!(info["faces"].as_array().unwrap().len(), 3);
    assert_eq!(info["sources"], 2);
}

#[test]
fn live_paint_fill_assigns_the_face_under_the_point() {
    let mut s = session();
    let g = live(&mut s);
    let r = s.execute("livePaint.fill", &json!({"group": g.0, "point": [75, 50], "color": "#00ff00"})).unwrap();
    let face = node(&s, NodeId(r["face"].as_u64().unwrap()));
    let bb = face.geometric_bounds().unwrap();
    assert!((bb.x0 - 50.0).abs() < 1e-6 && (bb.x1 - 100.0).abs() < 1e-6, "{bb:?}");
    assert_eq!(face.appearance.fill_paint(), Paint::solid(Color::from_hex("#00ff00").unwrap()));
    // The other faces are untouched.
    let gn = node(&s, g);
    let green = vectorcraft_tools::builder::faces(&gn).iter().filter(|f| f.appearance.fill_paint() == face.appearance.fill_paint()).count();
    assert_eq!(green, 1);
    // Default paint = current fill; a miss is an error.
    s.paint.fill = Paint::solid(Color::rgb(1.0, 1.0, 0.0));
    let r = s.execute("livePaint.fill", &json!({"group": g.0, "point": [10, 10]})).unwrap();
    assert_eq!(node(&s, NodeId(r["face"].as_u64().unwrap())).appearance.fill_paint(), s.paint.fill);
    assert!(s.execute("livePaint.fill", &json!({"group": g.0, "point": [400, 400]})).is_err());
}

#[test]
fn live_paint_fill_on_plain_paths_makes_the_group() {
    let mut s = session();
    let (a, b) = two(&mut s);
    let r = s.execute("livePaint.fill", &json!({"ids": [a.0, b.0], "point": [125, 50], "color": "#123456"})).unwrap();
    let g = NodeId(r["group"].as_u64().unwrap());
    assert!(vectorcraft_tools::builder::is_live_paint(&node(&s, g)));
    assert_eq!(layer_children(&s), 1);
    // One undo step removes both the make and the fill.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(layer_children(&s), 2);
}

#[test]
fn live_paint_stroke_edge() {
    let mut s = session();
    let g = live(&mut s);
    let r = s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [100, 50], "color": "#ff0000", "width": 3})).unwrap();
    let e = node(&s, NodeId(r["edge"].as_u64().unwrap()));
    assert_eq!(e.appearance.stroke_width(), 3.0);
    assert!(e.appearance.fill_paint().is_none());
    let bb = e.geometric_bounds().unwrap();
    assert!((bb.x0 - 100.0).abs() < 1e-6 && (bb.x1 - 100.0).abs() < 1e-6, "the vertical edge at x=100: {bb:?}");
    assert!(s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [300, 300]})).is_err());
}

/// Shape Builder refuses an on/off option that is not true or false and a tolerance or gap that is
/// not a number, given as text or as another value of the wrong kind, naming the key; the document
/// and the undo history stay as they were. True, false and null still work.
#[test]
fn shape_builder_refuses_a_number_or_boolean_given_as_text() {
    use crate::tests::{not_a_number, not_a_switch, not_refused};
    let mut s = session();
    two(&mut s);
    let merge = |extra: Value| {
        let mut p = json!({"points": [[75, 50]]});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        p
    };
    let calls = [
        not_a_switch("shapeBuilder.merge", merge(json!({"erase": "true"})), "erase"),
        not_a_switch("shapeBuilder.merge", merge(json!({"openFilledAsClosed": "false"})), "openFilledAsClosed"),
        not_a_switch("shapeBuilder.merge", merge(json!({"splitStroke": "true"})), "splitStroke"),
        not_a_number("shapeBuilder.merge", merge(json!({"erase": true, "tolerance": "2"})), "tolerance"),
        not_a_number("shapeBuilder.merge", merge(json!({"gap": "5"})), "gap"),
        not_a_switch("shapeBuilder.regions", json!({"openFilledAsClosed": 1}), "openFilledAsClosed"),
        not_a_number("shapeBuilder.regions", json!({"gap": [5]}), "gap"),
    ];
    let wrong = not_refused(&mut s, &calls);
    assert!(wrong.is_empty(), "not refused: {wrong:#?}");
    s.execute("shapeBuilder.merge", &merge(json!({"erase": false, "openFilledAsClosed": null}))).unwrap();
}

/// Live Paint and Image Trace refuse an on/off option that is not true or false and a width or
/// tolerance that is not a number, given as text or as another value of the wrong kind, naming the
/// key; the document and the undo history stay as they were. Numbers and null still work.
#[test]
fn live_paint_and_image_trace_refuse_a_number_or_boolean_given_as_text() {
    use crate::tests::{not_a_number, not_a_switch, not_refused};
    let mut s = session();
    let g = live(&mut s);
    let calls = [
        not_a_switch("livePaint.gapOptions", json!({"closeGaps": "true"}), "closeGaps"),
        not_a_number("livePaint.strokeEdge", json!({"group": g.0, "point": [100, 50], "width": "3"}), "width"),
        not_a_number("livePaint.strokeEdge", json!({"group": g.0, "point": [100, 50], "tolerance": [4]}), "tolerance"),
        not_a_switch("imageTrace.make", json!({"enhanced": "true"}), "enhanced"),
    ];
    let wrong = not_refused(&mut s, &calls);
    assert!(wrong.is_empty(), "not refused: {wrong:#?}");
    let r = s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [100, 50], "width": 3, "tolerance": null})).unwrap();
    assert_eq!(node(&s, NodeId(r["edge"].as_u64().unwrap())).appearance.stroke_width(), 3.0);
}

#[test]
fn live_paint_stroke_edge_skips_hidden_edges() {
    let mut s = session();
    let g = live(&mut s);
    // The vertical edge at x=100 is hidden on its own; the group stays visible.
    let vertical_at = |s: &Session, x: f64| -> Vec<u64> {
        vectorcraft_tools::builder::edges(&node(s, g))
            .iter()
            .filter(|e| e.geometric_bounds().is_some_and(|bb| (bb.x0 - x).abs() < 1e-6 && (bb.x1 - x).abs() < 1e-6))
            .map(|e| e.id.0)
            .collect()
    };
    let hidden = vertical_at(&s, 100.0);
    assert!(!hidden.is_empty());
    s.execute("object.setProps", &json!({"ids": hidden, "visible": false})).unwrap();
    // 1 pt from the hidden edge, the nearest visible edge within the tolerance is the one at x=50.
    let r = s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [99, 50], "tolerance": 60})).unwrap();
    assert!(vertical_at(&s, 50.0).contains(&r["edge"].as_u64().unwrap()), "{r}");
    // With only the hidden edge within the tolerance, no edge is stroked.
    assert!(s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [99, 50]})).is_err());
}

#[test]
fn live_paint_fill_skips_hidden_faces() {
    let mut s = session();
    let g = live(&mut s);
    // The face from x=50 to x=100 is hidden on its own; the group stays visible.
    let hidden = vectorcraft_tools::builder::face_at(&node(&s, g), vectorcraft_geom::Point::new(75.0, 50.0)).unwrap().id;
    s.execute("object.setProps", &json!({"ids": [hidden.0], "visible": false})).unwrap();
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let children = node(&s, g).children().unwrap().len();
    // livePaint.fill at a point on the hidden face returns an error and fills no face, with `group`
    // and with the group in `ids`, and it does not rebuild the group.
    for p in [json!({"group": g.0, "point": [75, 50], "color": "#00ff00"}), json!({"ids": [g.0], "point": [75, 50], "color": "#00ff00"})] {
        assert!(s.execute("livePaint.fill", &p).is_err(), "{p}");
        let face = node(&s, hidden);
        assert!(!face.visible && face.appearance.fill_paint() != green, "{p}");
        assert_eq!(node(&s, g).children().unwrap().len(), children, "{p}");
    }
    // Given another Live Paint group with a face at that point as well, that face is filled.
    let other = rect(&mut s, 60.0, 40.0, 20.0, 20.0);
    let other = NodeId(s.execute("livePaint.make", &json!({"ids": [other.0]})).unwrap()["id"].as_u64().unwrap());
    let r = s.execute("livePaint.fill", &json!({"ids": [g.0, other.0], "point": [75, 50], "color": "#00ff00"})).unwrap();
    assert_eq!(r["group"], other.0);
    assert_eq!(node(&s, NodeId(r["face"].as_u64().unwrap())).appearance.fill_paint(), green);
    assert!(!node(&s, hidden).visible);
}

#[test]
fn live_paint_release_and_expand() {
    let mut s = session();
    let g = live(&mut s);
    s.execute("livePaint.fill", &json!({"group": g.0, "point": [75, 50], "none": true})).unwrap();
    let r = s.execute("livePaint.expand", &json!({})).unwrap();
    assert_eq!(ids(&r), vec![g]);
    let n = node(&s, g);
    assert!(!vectorcraft_tools::builder::is_live_paint(&n));
    // Unpainted face and the hidden sources are gone: 2 faces + stroked edges.
    let ch = n.children().unwrap();
    assert!(ch.iter().all(|c| c.visible));
    assert_eq!(ch.iter().filter(|c| c.name.as_deref() == Some("Face")).count(), 2);

    let mut s = session();
    let (a, b) = two(&mut s);
    live_group_only(&mut s);
    let r = s.execute("livePaint.release", &json!({})).unwrap();
    assert_eq!(ids(&r), vec![a, b]);
    assert!((area(&s, a) - 10000.0).abs() < 1.0);
}

fn live_group_only(s: &mut Session) -> NodeId {
    let r = s.execute("livePaint.make", &json!({})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

#[test]
fn live_paint_merge_keeps_paint_and_adds_faces() {
    let mut s = session();
    let g = live(&mut s);
    s.execute("livePaint.fill", &json!({"group": g.0, "point": [25, 50], "color": "#00ff00"})).unwrap();
    let c = rect(&mut s, 120.0, 20.0, 100.0, 20.0);
    s.execute("select.set", &json!({"ids": [g.0, c.0]})).unwrap();
    let r = s.execute("livePaint.merge", &json!({})).unwrap();
    assert_eq!(NodeId(r["id"].as_u64().unwrap()), g);
    assert!(r["faces"].as_u64().unwrap() > 3);
    let gn = node(&s, g);
    let f = vectorcraft_tools::builder::face_at(&gn, vectorcraft_geom::Point::new(25.0, 50.0)).unwrap();
    assert_eq!(f.appearance.fill_paint(), Paint::solid(Color::from_hex("#00ff00").unwrap()));
    let src = vectorcraft_tools::builder::sources(&gn).unwrap();
    assert_eq!(src.children().unwrap().len(), 3, "the rectangle joined the sources");
    assert_eq!(s.doc().unwrap().doc.parent_of(c), Some(src.id));
    assert_eq!(layer_children(&s), 1);
}

/// Live Paint builds its faces and edges from path outlines alone, so Merge, Make and the Live
/// Paint Bucket refuse type, an image, or a group holding type: before, they went into the
/// group's hidden sources and vanished from the canvas. Nothing changes, not even the undo
/// history. Paths, and groups of paths, still merge.
#[test]
fn live_paint_refuses_type_and_images_and_changes_nothing() {
    let mut s = session();
    let g = live(&mut s);
    let text = NodeId(s.execute("text.create", &json!({"x": 200, "y": 300, "text": "Hi"})).unwrap()["id"].as_u64().unwrap());
    let image = add_image(&mut s, &vectorcraft_trace::Raster::from_fn(4, 4, |_, _| [0, 0, 0, 255]), Affine::translate((300.0, 300.0)));
    let path = rect(&mut s, 400.0, 20.0, 40.0, 40.0);
    s.execute("select.set", &json!({"ids": [text.0, path.0]})).unwrap();
    let text_group = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    let state = |s: &Session| {
        let d = s.doc().unwrap();
        (d.doc.clone(), d.selection.clone(), d.history.undo.len())
    };
    let want = "Live Paint takes paths and compound paths: convert type, images and other objects to paths first";
    for other in [text, image, text_group] {
        let calls = [
            ("livePaint.merge", json!({"ids": [g.0, other.0]})),
            ("livePaint.make", json!({"ids": [g.0, other.0]})),
            ("livePaint.make", json!({"ids": [other.0]})),
            ("livePaint.fill", json!({"ids": [other.0], "point": [200, 300], "color": "#00ff00"})),
        ];
        for (c, p) in calls {
            s.execute("select.set", &json!({"ids": [g.0, other.0]})).unwrap();
            let before = state(&s);
            let r = s.execute(c, &p);
            assert!(matches!(&r, Err(EngineError::Other(m)) if m == want), "{c} {p}: {r:?}");
            assert!(state(&s) == before, "{c} {p} changed the document");
        }
    }
    // A group of paths and a path still merge.
    let (a, b) = (rect(&mut s, 120.0, 20.0, 100.0, 20.0), rect(&mut s, 120.0, 60.0, 100.0, 20.0));
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let paths = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    let c = rect(&mut s, 0.0, 120.0, 100.0, 20.0);
    let r = s.execute("livePaint.merge", &json!({"ids": [g.0, paths.0, c.0]})).unwrap();
    assert_eq!(NodeId(r["id"].as_u64().unwrap()), g);
    assert!(vectorcraft_tools::builder::live_paint_takes(&node(&s, g)));
    assert!(!vectorcraft_tools::builder::live_paint_takes(&node(&s, text_group)));
}

/// Give Live Paint group `g` a lock, hidden state, appearance, knockout, opacity mask and
/// graphic style link, and return it as it then is.
fn dress_live_group(s: &mut Session, g: NodeId) -> Node {
    use vectorcraft_doc::{Knockout, OpacityMask};
    let mask = Node::path(NodeId(0), vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Default::default());
    s.edit("t", |d, _| {
        let n = d.node_mut(g).unwrap();
        n.locked = true;
        n.visible = false;
        n.isolate = true;
        n.knockout = Knockout::On;
        n.appearance.set_stroke(Paint::solid(Color::rgb(0.0, 0.5, 0.0)));
        let mut e = n.extras_mut();
        e.mask = Some(Box::new(OpacityMask::new(mask, true)));
        e.graphic_style = Some(7);
        Ok(())
    })
    .unwrap();
    node(s, g)
}

/// Asserts that `after` is `before` with other children and Gap Options.
fn same_group_but_children(before: &Node, after: &Node) {
    assert_eq!((after.locked, after.visible, after.isolate), (true, false, true));
    assert_eq!(after.appearance, before.appearance, "the group's own appearance");
    assert_eq!(after.extras().mask, before.extras().mask, "the opacity mask");
    assert_eq!(after.extras().graphic_style, Some(7), "the graphic style link");
    let mut want = before.clone();
    *want.children_mut().unwrap() = after.children().unwrap().clone();
    want.extras_mut().live_paint_gaps = after.extras().live_paint_gaps.clone();
    assert_eq!(after, &want);
}

#[test]
fn live_paint_merge_keeps_the_groups_lock_visibility_mask_and_graphic_style() {
    let mut s = session();
    let g = live(&mut s);
    let before = dress_live_group(&mut s, g);
    let c = rect(&mut s, 120.0, 20.0, 100.0, 20.0);
    let r = s.execute("livePaint.merge", &json!({"ids": [g.0, c.0]})).unwrap();
    assert_eq!(NodeId(r["id"].as_u64().unwrap()), g);
    let after = node(&s, g);
    same_group_but_children(&before, &after);
    assert_eq!(vectorcraft_tools::builder::sources(&after).unwrap().children().unwrap().len(), 3, "the rectangle joined the sources");
    // One undo step brings back the group as it was, and the rectangle beside it.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, g), before);
    assert_eq!(layer_children(&s), 2);
}

#[test]
fn live_paint_make_and_fill_with_a_live_paint_group_selected_add_the_paths_to_it() {
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let fill_at = |s: &Session, g: NodeId, x: f64, y: f64| {
        vectorcraft_tools::builder::face_at(&node(s, g), vectorcraft_geom::Point::new(x, y)).map(|f| f.appearance.fill_paint())
    };
    // Make, and the Live Paint Bucket on a path beside the group, each keep the group, its
    // properties and the paint of its faces.
    for (cmd, params) in [("livePaint.make", json!({})), ("livePaint.fill", json!({"point": [170, 30], "color": "#0000ff"}))] {
        let mut s = session();
        let g = live(&mut s);
        s.execute("livePaint.fill", &json!({"group": g.0, "point": [25, 50], "color": "#00ff00"})).unwrap();
        let before = dress_live_group(&mut s, g);
        let c = rect(&mut s, 120.0, 20.0, 100.0, 20.0);
        let mut p = params;
        p["ids"] = json!([g.0, c.0]);
        let r = s.execute(cmd, &p).unwrap();
        let id = r.get("id").or_else(|| r.get("group")).and_then(Value::as_u64);
        assert_eq!(id, Some(g.0), "{cmd}");
        same_group_but_children(&before, &node(&s, g));
        assert_eq!(fill_at(&s, g, 25.0, 50.0), Some(green.clone()), "{cmd}");
        assert_eq!(layer_children(&s), 1, "{cmd}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(node(&s, g), before, "{cmd}");
    }
}

/// A Live Paint group of one open path round the square 0..100, its end 2 pt short of its start.
fn live_square_with_a_gap(s: &mut Session) -> NodeId {
    let anchors = [(0, 0), (100, 0), (100, 100), (0, 100), (0, 2)].map(|(x, y)| json!({"x": x, "y": y}));
    let p = s.execute("path.create", &json!({ "anchors": anchors })).unwrap();
    s.execute("select.set", &json!({"ids": [p["id"]]})).unwrap();
    live_group_only(s)
}

fn gap_options(s: &Session, g: NodeId) -> vectorcraft_doc::GapOptions {
    node(s, g).gap_options()
}

/// The gaps of a `livePaint.*` reply, rounded to 1e-6.
fn gaps_of(r: &Value) -> Vec<Vec<f64>> {
    r["gaps"].as_array().unwrap().iter().map(|g| g.as_array().unwrap().iter().map(|v| (v.as_f64().unwrap() * 1e6).round() / 1e6).collect()).collect()
}

#[test]
fn live_paint_gap_detection_stops_paint_at_a_gap() {
    let mut s = session();
    let g = live_square_with_a_gap(&mut s);
    // Paint leaks through the gap: there is no face to fill.
    assert!(vectorcraft_tools::builder::faces(&node(&s, g)).is_empty());
    assert!(s.execute("livePaint.fill", &json!({"group": g.0, "point": [50, 50]})).is_err());
    let info = s.execute("livePaint.info", &json!({"group": g.0})).unwrap();
    assert_eq!(info["gapOptions"], json!({"detect": false, "size": "small", "points": 3.0, "color": "#ff4f4f"}));
    assert_eq!(gaps_of(&info), [[0.0, 0.0, 0.0, 2.0]], "Small Gaps (3 pt) finds the gap");
    // Gap Detection with Small Gaps: the square is one face, filled in one undo step.
    let depth = s.doc().unwrap().history.undo.len();
    let r = s.execute("livePaint.gapOptions", &json!({"detect": true, "size": "small"})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    assert_eq!((ids(&r), &r["faces"], &r["closed"]), (vec![g], &json!(1), &json!(0)));
    assert_eq!(gaps_of(&r), [[0.0, 0.0, 0.0, 2.0]]);
    let f = s.execute("livePaint.fill", &json!({"group": g.0, "point": [50, 50], "color": "#00ff00"})).unwrap();
    let face = node(&s, NodeId(f["face"].as_u64().unwrap()));
    assert!((vectorcraft_pathops::area(face.path_data().unwrap(), FillRule::NonZero).abs() - 10000.0).abs() < 1e-6);
    assert_eq!(vectorcraft_tools::builder::edges(&node(&s, g)).len(), 1, "the gap is no edge");
    // A color change keeps the face and its paint.
    s.execute("livePaint.gapOptions", &json!({"id": g.0, "color": "Green"})).unwrap();
    let opts = gap_options(&s, g);
    assert_eq!((opts.detect, opts.size, opts.color), (true, vectorcraft_doc::GapSize::Small, [0x4f, 0xff, 0x4f]));
    let gn = node(&s, g);
    let kept = vectorcraft_tools::builder::face_at(&gn, vectorcraft_geom::Point::new(50.0, 50.0)).unwrap();
    assert_eq!(kept.appearance.fill_paint(), Paint::solid(Color::from_hex("#00ff00").unwrap()));
    // A custom size narrower than the gap lets paint leak again, and so does Gap Detection off.
    let r = s.execute("livePaint.gapOptions", &json!({"size": 1.5})).unwrap();
    assert_eq!((&r["faces"], gaps_of(&r).len()), (&json!(0), 0));
    s.execute("livePaint.gapOptions", &json!({"size": 12})).unwrap();
    assert_eq!(vectorcraft_tools::builder::faces(&node(&s, g)).len(), 1);
    let r = s.execute("livePaint.gapOptions", &json!({"detect": false})).unwrap();
    assert_eq!((&r["faces"], gaps_of(&r).len()), (&json!(0), 1));
    // Bad values change nothing.
    for bad in [json!({"size": 0}), json!({"size": 73}), json!({"size": "huge"}), json!({"detect": "yes"}), json!({"color": "#12"})] {
        assert!(s.execute("livePaint.gapOptions", &bad).is_err(), "{bad}");
    }
    assert_eq!(gap_options(&s, g).size, vectorcraft_doc::GapSize::Custom(12.0));
    // Plain paths are no Live Paint group.
    let r = rect(&mut s, 200.0, 0.0, 10.0, 10.0);
    assert!(s.execute("livePaint.gapOptions", &json!({"id": r.0, "detect": true})).is_err());
    // Expand leaves a plain group, without the options.
    s.execute("livePaint.expand", &json!({"id": g.0})).unwrap();
    assert!(node(&s, g).extras().live_paint_gaps.is_none());
}

#[test]
fn medium_and_large_gaps_stop_paint_at_wider_gaps() {
    // Squares whose ends stop 5 pt and 9 pt short of their starts: each size closes the gaps up
    // to its width (Small 3 pt, Medium 6 pt, Large 12 pt).
    for (gap, closed_by) in [(5, ["medium", "large"]), (9, ["large", "large"])] {
        let mut s = session();
        let anchors = [(0, 0), (100, 0), (100, 100), (0, 100), (0, gap)].map(|(x, y)| json!({"x": x, "y": y}));
        let p = s.execute("path.create", &json!({ "anchors": anchors })).unwrap();
        s.execute("select.set", &json!({"ids": [p["id"]]})).unwrap();
        let g = live_group_only(&mut s);
        for size in ["small", "medium", "large"] {
            let r = s.execute("livePaint.gapOptions", &json!({"detect": true, "size": size})).unwrap();
            let faces = if closed_by.contains(&size) { 1 } else { 0 };
            assert_eq!(r["faces"], json!(faces), "a {gap} pt gap with {size}");
        }
        let info = s.execute("livePaint.info", &json!({"group": g.0})).unwrap();
        assert_eq!(info["gapOptions"], json!({"detect": true, "size": "large", "points": 12.0, "color": "#ff4f4f"}));
        s.execute("livePaint.gapOptions", &json!({"size": "Medium"})).unwrap();
        assert_eq!(gap_options(&s, g).size, vectorcraft_doc::GapSize::Medium);
    }
}

#[test]
fn gap_options_keep_the_paint_of_split_faces_and_of_unpainted_faces() {
    let mut s = session();
    // A red square with a divider that stops 2 pt short of its top edge.
    let r = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    set_fill(&mut s, r, Color::rgb(1.0, 0.0, 0.0));
    let l = line(&mut s, (50.0, 2.0), (50.0, 100.0));
    s.execute("select.set", &json!({"ids": [r.0, l.0]})).unwrap();
    let g = live_group_only(&mut s);
    assert_eq!(vectorcraft_tools::builder::faces(&node(&s, g)).len(), 1);
    s.execute("livePaint.fill", &json!({"group": g.0, "point": [25, 50], "color": "#00ff00"})).unwrap();
    let fill_at = |s: &Session, x: f64| {
        let gn = node(s, g);
        vectorcraft_tools::builder::face_at(&gn, vectorcraft_geom::Point::new(x, 50.0)).map(|f| f.appearance.fill_paint())
    };
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    // Closing the gap splits the face in two, and both halves keep its paint.
    let r = s.execute("livePaint.gapOptions", &json!({"detect": true})).unwrap();
    assert_eq!(r["faces"], 2, "{r}");
    assert_eq!((fill_at(&s, 25.0), fill_at(&s, 75.0)), (Some(green.clone()), Some(green.clone())));
    // A face painted with none stays unpainted when the options change, and a face keeps its
    // transparency and the rest of its look when the faces are made again.
    s.execute("livePaint.fill", &json!({"group": g.0, "point": [75, 50], "none": true})).unwrap();
    s.execute("livePaint.gapOptions", &json!({"color": "Blue"})).unwrap();
    assert_eq!((fill_at(&s, 25.0), fill_at(&s, 75.0)), (Some(green.clone()), Some(Paint::None)));
    let left = vectorcraft_tools::builder::face_at(&node(&s, g), vectorcraft_geom::Point::new(25.0, 50.0)).unwrap().id;
    s.edit("t", |d, _| {
        d.node_mut(left).unwrap().opacity = 0.5;
        Ok(())
    })
    .unwrap();
    s.execute("livePaint.gapOptions", &json!({"size": 12})).unwrap();
    let gn = node(&s, g);
    let f = vectorcraft_tools::builder::face_at(&gn, vectorcraft_geom::Point::new(25.0, 50.0)).unwrap();
    assert_eq!((f.opacity, f.appearance.fill_paint()), (0.5, green));
    assert_eq!(fill_at(&s, 75.0), Some(Paint::None));
}

#[test]
fn a_gradient_face_split_by_a_closed_gap_keeps_one_gradient_across_both_halves() {
    use vectorcraft_color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    s.edit("t", |d, _| {
        d.node_mut(r).unwrap().appearance.set_fill(gradient);
        Ok(())
    })
    .unwrap();
    let l = line(&mut s, (50.0, 2.0), (50.0, 100.0));
    s.execute("select.set", &json!({"ids": [r.0, l.0]})).unwrap();
    let g = live_group_only(&mut s);
    s.execute("livePaint.gapOptions", &json!({"detect": true})).unwrap();
    let gn = node(&s, g);
    let geoms: Vec<_> = vectorcraft_tools::builder::faces(&gn)
        .iter()
        .map(|f| match f.appearance.fill_paint() {
            Paint::Gradient(gp) => gp.geom,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(geoms.len(), 2);
    assert!(geoms[0].is_some() && geoms[0] == geoms[1], "both halves keep the square's fit: {geoms:?}");
}

#[test]
fn close_gaps_with_paths_adds_an_unpainted_path_across_each_gap() {
    let mut s = session();
    let g = live_square_with_a_gap(&mut s);
    let r = s.execute("livePaint.gapOptions", &json!({"closeGaps": true})).unwrap();
    assert_eq!((&r["faces"], &r["closed"], gaps_of(&r).len()), (&json!(1), &json!(1), 0), "{r}");
    assert!(!gap_options(&s, g).detect, "Gap Detection stays off: the path closes the gap");
    let gn = node(&s, g);
    let src = vectorcraft_tools::builder::sources(&gn).unwrap();
    let added = src.children().unwrap().last().unwrap();
    assert!(added.appearance.fill_paint().is_none() && added.appearance.stroke_paint().is_none(), "unpainted");
    let bb = added.geometric_bounds().unwrap();
    assert!(bb.x0.abs() < 1e-9 && bb.x1.abs() < 1e-9 && bb.y0.abs() < 1e-9 && (bb.y1 - 2.0).abs() < 1e-9, "{bb:?}");
    assert!(s.execute("livePaint.fill", &json!({"group": g.0, "point": [50, 50]})).is_ok());
    // Release gives the closing path back with the others.
    assert_eq!(ids(&s.execute("livePaint.release", &json!({"id": g.0})).unwrap()).len(), 2);
}

#[test]
fn gap_options_survive_save_copy_and_undo() {
    let mut s = session();
    let g = live_square_with_a_gap(&mut s);
    s.execute("livePaint.gapOptions", &json!({"detect": true, "size": 4.5, "color": "#123456"})).unwrap();
    let want = vectorcraft_doc::GapOptions { detect: true, size: vectorcraft_doc::GapSize::Custom(4.5), color: [0x12, 0x34, 0x56] };
    assert_eq!(gap_options(&s, g), want);
    let doc = s.doc().unwrap().doc.clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    assert_eq!(back.node(g).unwrap().gap_options(), want);
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    let pasted = s.doc().unwrap().selection.objects[0];
    assert_ne!(pasted, g);
    assert_eq!(gap_options(&s, pasted), want);
    assert_eq!(vectorcraft_tools::builder::faces(&node(&s, pasted)).len(), 1);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(gap_options(&s, g), vectorcraft_doc::GapOptions::default());
    assert!(vectorcraft_tools::builder::faces(&node(&s, g)).is_empty());
}

#[test]
fn merged_live_paint_groups_keep_the_bottom_groups_gap_options() {
    use vectorcraft_doc::{GapOptions, GapSize};
    // Two groups side by side, the left one at the bottom, merged with each Gap Detection setting.
    let merged = |bottom: Value, top: Value| {
        let mut s = session();
        let groups: Vec<NodeId> = [0.0, 200.0]
            .into_iter()
            .zip([bottom, top])
            .map(|(x, opts)| {
                let r = rect(&mut s, x, 0.0, 100.0, 100.0);
                s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
                let g = live_group_only(&mut s);
                s.execute("livePaint.gapOptions", &opts).unwrap();
                g
            })
            .collect();
        s.execute("select.set", &json!({"ids": [groups[0].0, groups[1].0]})).unwrap();
        let r = s.execute("livePaint.merge", &json!({})).unwrap();
        assert_eq!(NodeId(r["id"].as_u64().unwrap()), groups[0]);
        gap_options(&s, groups[0])
    };
    let six = json!({"detect": true, "size": 6, "color": "Blue"});
    let twelve = json!({"detect": true, "size": 12});
    let off = json!({"detect": false, "size": 12, "color": "Blue"});
    let blue = [0x4f, 0x4f, 0xff];
    // The bottom group's options, with its Gap Detection on.
    assert_eq!(merged(six.clone(), twelve), GapOptions { detect: true, size: GapSize::Custom(6.0), color: blue });
    // Detection on with Small Gaps when only a group above has it on.
    assert_eq!(merged(off.clone(), six), GapOptions { detect: true, size: GapSize::Small, color: blue });
    // With detection off in every group, the bottom group's options.
    assert_eq!(merged(off, json!({"detect": false})), GapOptions { detect: false, size: GapSize::Custom(12.0), color: blue });
}

#[test]
fn live_paint_bucket_tool_fills_face() {
    let mut s = session();
    let g = live(&mut s);
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.fill = Paint::solid(Color::rgb(0.0, 1.0, 1.0));
    s.pointer(&PointerEvent::new(PointerKind::Move, 125.0, 50.0), v).unwrap();
    assert!(!s.overlays(v).is_empty(), "red face highlight");
    s.pointer(&PointerEvent::new(PointerKind::Down, 125.0, 50.0), v).unwrap();
    let gn = node(&s, g);
    let f = vectorcraft_tools::builder::face_at(&gn, vectorcraft_geom::Point::new(125.0, 50.0)).unwrap();
    assert_eq!(f.appearance.fill_paint(), s.paint.fill);
    // Live Paint Selection selects the face.
    s.select_tool("livePaintSelection", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 125.0, 50.0), v).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, vec![f.id]);
}

fn line(s: &mut Session, a: (f64, f64), b: (f64, f64)) -> NodeId {
    let r = s.execute("shape.line", &json!({"x1": a.0, "y1": a.1, "x2": b.0, "y2": b.1})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

/// Crossing lines, as in the bug report: the Live Paint Bucket fills the area they enclose.
#[test]
fn live_paint_bucket_fills_an_area_enclosed_by_lines() {
    let mut s = session();
    let ids = [line(&mut s, (0.0, 0.0), (100.0, 100.0)), line(&mut s, (100.0, 0.0), (0.0, 100.0)), line(&mut s, (-10.0, 80.0), (110.0, 80.0))];
    s.execute("select.set", &json!({"ids": ids.map(|i| i.0)})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.fill = Paint::solid(Color::rgb(1.0, 0.0, 1.0));
    // Inside the triangle (20,80) (50,50) (80,80): highlighted, then filled.
    s.pointer(&PointerEvent::new(PointerKind::Move, 50.0, 70.0), v).unwrap();
    assert!(!s.overlays(v).is_empty(), "red face highlight");
    s.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 70.0), v).unwrap();
    let g = s.doc().unwrap().selection.objects[0];
    let gn = node(&s, g);
    assert!(vectorcraft_tools::builder::is_live_paint(&gn));
    assert_eq!(vectorcraft_tools::builder::faces(&gn).len(), 1);
    let f = vectorcraft_tools::builder::face_at(&gn, vectorcraft_geom::Point::new(50.0, 70.0)).unwrap();
    assert_eq!(f.appearance.fill_paint(), s.paint.fill);
    assert!((vectorcraft_pathops::area(f.path_data().unwrap(), FillRule::NonZero).abs() - 900.0).abs() < 1e-6);
    // The lines are edges, cut where they cross, and keep their stroke.
    let edges = vectorcraft_tools::builder::edges(&gn);
    assert_eq!(edges.len(), 9);
    assert!(edges.iter().all(|e| !e.appearance.stroke_paint().is_none()));
    // Outside the triangle there's nothing to fill.
    assert!(s.execute("livePaint.fill", &json!({"group": g.0, "point": [50, 20]})).is_err());
}

#[test]
fn a_line_across_a_live_paint_shape_splits_its_face() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 50.0);
    set_fill(&mut s, r, Color::rgb(1.0, 0.0, 0.0));
    let l = line(&mut s, (40.0, -20.0), (60.0, 70.0));
    s.execute("select.set", &json!({"ids": [r.0, l.0]})).unwrap();
    let made = s.execute("livePaint.make", &json!({})).unwrap();
    assert_eq!(made["faces"], 2, "{made}");
    let g = NodeId(made["id"].as_u64().unwrap());
    s.execute("livePaint.fill", &json!({"group": g.0, "point": [80, 25], "color": "#0000ff"})).unwrap();
    let gn = node(&s, g);
    let face = |x: f64| vectorcraft_tools::builder::face_at(&gn, vectorcraft_geom::Point::new(x, 25.0)).unwrap().appearance.fill_paint();
    assert_eq!(face(10.0), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), "the left half keeps the rectangle's fill");
    assert_eq!(face(80.0), Paint::solid(Color::from_hex("#0000ff").unwrap()));
}

/// The total length of the stroked paths among `out` that have no fill.
fn stroked_length(s: &Session, out: &[NodeId]) -> f64 {
    out.iter()
        .map(|i| node(s, *i))
        .filter(|n| n.appearance.fill_paint().is_none() && !n.appearance.stroke_paint().is_none())
        .map(|n| outline(&n).0.length())
        .sum()
}

/// The Shape Builder takes a selected Live Paint group's filled faces as regions and its stroked
/// edges as lines; its unfilled faces are not regions. The merge replaces the group as it does a
/// path, and the edges keep their strokes, except those between the merged faces.
#[test]
fn shape_builder_uses_the_filled_faces_of_a_live_paint_group() {
    let mut s = session();
    let g = live(&mut s);
    s.execute("livePaint.fill", &json!({"group": g.0, "point": [125, 50], "none": true})).unwrap();
    let edges = vectorcraft_tools::builder::edges(&node(&s, g)).len();
    let c = rect(&mut s, 160.0, 20.0, 100.0, 20.0);
    set_fill(&mut s, c, Color::rgb(0.0, 1.0, 0.0));
    s.execute("select.set", &json!({"ids": [g.0, c.0]})).unwrap();
    let map = s.execute("shapeBuilder.regions", &json!({})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[25, 50], [75, 50]]})).unwrap();
    let out = ids(&r);
    let unfilled_faces =
        out.iter().map(|i| node(&s, *i)).filter(|n| n.appearance.fill_paint().is_none() && n.appearance.stroke_paint().is_none()).count();
    assert_eq!(
        (map["regions"].as_array().unwrap().len(), map["lines"].as_array().unwrap().len() >= edges, unfilled_faces),
        (4, true, 0),
        "the two faces, the rectangle and the area the edges enclose; the edges as lines; no unfilled face comes back"
    );
    let enclosed: Vec<&Value> = map["regions"].as_array().unwrap().iter().filter(|r| r["sources"].as_array().unwrap().is_empty()).collect();
    assert_eq!(enclosed.len(), 1, "{map}");
    let bounds: Vec<f64> = enclosed[0]["bounds"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    assert!(bounds.iter().zip([100.0, 0.0, 150.0, 100.0]).all(|(a, b)| (a - b).abs() < 0.5), "{bounds:?}");
    assert_eq!(r["regions"], 2);
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert!((area(&s, merged) - 10000.0).abs() < 1.0, "{}", area(&s, merged));
    assert!(s.doc().unwrap().doc.node(g).is_none());
    // The outline of 0..150 × 0..100 and the edge at x = 100 keep their strokes; the edge at x = 50
    // between the merged faces is removed.
    assert!((stroked_length(&s, &out) - 600.0).abs() < 1.0, "{}", stroked_length(&s, &out));
}

/// An edge that closes on itself is a line around its face, not a second shape over it.
#[test]
fn shape_builder_takes_a_closed_live_paint_edge_as_a_line() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    set_fill(&mut s, a, Color::rgb(1.0, 0.0, 0.0));
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let group = node(&s, g);
    let edges = vectorcraft_tools::builder::edges(&group);
    assert_eq!(edges.len(), 1);
    assert!(outline(edges[0]).0.subpaths.iter().any(|sp| sp.closed), "the edge closes on itself");
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    // A click on the face keeps its fill on the merged shape and the edge's stroke around it.
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[50, 50]]})).unwrap();
    let out = ids(&r);
    let merged = node(&s, NodeId(r["merged"].as_u64().unwrap()));
    assert_eq!(merged.appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    assert!((stroked_length(&s, &out) - 400.0).abs() < 1.0, "{}", stroked_length(&s, &out));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    let map = s.execute("shapeBuilder.regions", &json!({})).unwrap();
    let regions = map["regions"].as_array().unwrap();
    assert_eq!((regions.len(), map["lines"].as_array().unwrap().len()), (1, 1), "{map}");
    assert_eq!(regions[0]["sources"].as_array().unwrap().len(), 1, "{map}");
    // Unfilled, the face is not a region, and the edge still encloses all of it, closing segment
    // included.
    s.execute("livePaint.fill", &json!({"group": g.0, "point": [50, 50], "none": true})).unwrap();
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    let map = s.execute("shapeBuilder.regions", &json!({})).unwrap();
    let regions = map["regions"].as_array().unwrap();
    assert_eq!(regions.len(), 1, "{map}");
    assert!(regions[0]["sources"].as_array().unwrap().is_empty(), "{map}");
    assert!((regions[0]["area"].as_f64().unwrap() - 10000.0).abs() < 1.0, "{map}");
}

/// A Live Paint group with no filled face keeps its stroked edges when another selected object is
/// merged.
#[test]
fn shape_builder_keeps_the_edges_of_an_unfilled_live_paint_group() {
    let mut s = session();
    let g = live(&mut s);
    for x in [25, 75, 125] {
        s.execute("livePaint.fill", &json!({"group": g.0, "point": [x, 50], "none": true})).unwrap();
    }
    let c = rect(&mut s, 160.0, 20.0, 100.0, 20.0);
    set_fill(&mut s, c, Color::rgb(0.0, 1.0, 0.0));
    s.execute("select.set", &json!({"ids": [g.0, c.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[200, 30]]})).unwrap();
    let out = ids(&r);
    assert!(s.doc().unwrap().doc.node(g).is_none());
    // Every edge of the group: the outline of 0..150 × 0..100 and the edges at x = 50 and 100.
    assert!((stroked_length(&s, &out) - 700.0).abs() < 1.0, "{}", stroked_length(&s, &out));
}

/// A plain group whose name starts with "Live Paint" is not a Live Paint group: the Shape Builder
/// takes its paths, and a merge elsewhere keeps them.
#[test]
fn shape_builder_takes_the_paths_of_a_plain_group_named_live_paint() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    set_fill(&mut s, a, Color::rgb(1.0, 0.0, 0.0));
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.edit("t", |d, _| {
        d.node_mut(g).unwrap().name = Some("Live Painting".into());
        Ok(())
    })
    .unwrap();
    let c = rect(&mut s, 160.0, 20.0, 100.0, 20.0);
    set_fill(&mut s, c, Color::rgb(0.0, 1.0, 0.0));
    s.execute("select.set", &json!({"ids": [g.0, c.0]})).unwrap();
    let map = s.execute("shapeBuilder.regions", &json!({})).unwrap();
    assert_eq!(map["regions"].as_array().unwrap().len(), 2, "{map}");
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[200, 30]]})).unwrap();
    let out = ids(&r);
    assert!(out.iter().any(|i| (area(&s, *i) - 10000.0).abs() < 1.0), "the group's rectangle is kept: {r}");
    // A stroke of the group's rectangle splits as a plain path's does.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[-1, 50]], "splitStroke": true})).unwrap();
    assert_eq!(node(&s, g).children().unwrap().len(), 2, "{r}");
    // Make Live Paint takes the group as a source rather than as a Live Paint group to merge into.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    let r = s.execute("livePaint.make", &json!({})).unwrap();
    assert_eq!(r["faces"], 1, "{r}");
    let made = node(&s, NodeId(r["id"].as_u64().unwrap()));
    assert!(vectorcraft_tools::builder::sources(&made).unwrap().children().unwrap().iter().any(|c| c.id == g));
}

// ---------- Image Trace ----------

/// A 100×100 image with a black disc (r = 30 px), placed at (50, 50) scaled ×2.
fn image_doc(s: &mut Session) -> NodeId {
    let r = vectorcraft_trace::Raster::from_fn(100, 100, |x, y| {
        let (dx, dy) = (x as f64 + 0.5 - 50.0, y as f64 + 0.5 - 50.0);
        if dx * dx + dy * dy <= 900.0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }
    });
    add_image(s, &r, Affine::translate((50.0, 50.0)) * Affine::scale(2.0))
}

fn add_image(s: &mut Session, r: &vectorcraft_trace::Raster, xf: Affine) -> NodeId {
    let png = r.encode_png();
    let (w, h) = (r.width, r.height);
    s.edit("img", |d, sel| {
        d.images.insert("img1".into(), ImageBlob::new("image/png", png));
        let id = d.alloc_id();
        let l = d.default_layer();
        d.insert(
            l,
            0,
            Node::new(
                id,
                NodeKind::Image(Box::new(ImageObject {
                    key: "img1".into(),
                    width: w,
                    height: h,
                    xf,
                    link: None,
                    placement: Default::default(),
                    pixels: None,
                })),
            ),
        )?;
        sel.set([id]);
        Ok(id)
    })
    .unwrap()
}

fn traced_paths(n: &Node) -> Vec<Arc<Node>> {
    n.children().unwrap().iter().filter(|c| !matches!(c.kind, NodeKind::Image(_))).cloned().collect()
}

#[test]
fn a_large_image_traces_with_a_warning() {
    // The public documentation: images over 1500 px on a side are traced, only more slowly.
    let mut s = session();
    let art = vectorcraft_trace::Raster::from_fn(
        1600,
        40,
        |x, y| if (100..1500).contains(&x) && (10..30).contains(&y) { [0, 0, 0, 255] } else { [255; 4] },
    );
    add_image(&mut s, &art, Affine::IDENTITY);
    let r = s.execute("imageTrace.make", &json!({"params": {"ignoreWhite": true}})).unwrap();
    assert_eq!(r["paths"], 1);
    let w = r["warnings"].as_array().unwrap();
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].as_str().unwrap().contains("1600 × 40") && w[0].as_str().unwrap().contains("1500"), "{w:?}");
    // Make and Expand says so too.
    let r = s.execute("imageTrace.makeAndExpand", &json!({"id": r["id"], "params": {"ignoreWhite": true}})).unwrap();
    assert_eq!(r["warnings"].as_array().map(Vec::len), Some(1));
    // At 1500 px and under, no warning.
    let mut s = session();
    image_doc(&mut s);
    let r = s.execute("imageTrace.make", &json!({})).unwrap();
    assert_eq!(r["warnings"], json!([]));
    let edge = vectorcraft_trace::Raster::from_fn(1500, 8, |x, _| if x < 700 { [0, 0, 0, 255] } else { [255; 4] });
    add_image(&mut s, &edge, Affine::IDENTITY);
    assert_eq!(s.execute("imageTrace.make", &json!({})).unwrap()["warnings"], json!([]));
}

#[test]
fn flat_logo_says_when_it_traces_an_image_averaged_down() {
    // Flat Logo works at most 1024 px on a side: a 1025 px image is averaged down to 513 px.
    let params = json!({"params": {"mode": "logo", "logoColors": ["#000000", "#F0F0DC"]}});
    let warnings = |n: u32| {
        let art = vectorcraft_trace::Raster::from_fn(n, n, |x, y| {
            if !(50..n - 50).contains(&x) || !(50..n - 50).contains(&y) {
                [0, 0, 0, 0]
            } else if (i64::from(x) - i64::from(y) / 8).rem_euclid(40) < 2 {
                [0, 0, 0, 255]
            } else {
                [240, 240, 220, 255]
            }
        });
        let mut s = session();
        add_image(&mut s, &art, Affine::IDENTITY);
        let r = s.execute("imageTrace.makeAndExpand", &params).unwrap();
        assert!(r["paths"].as_u64().unwrap() > 0, "{r}");
        r["warnings"].clone()
    };
    let w = warnings(1025);
    let w = w.as_array().unwrap();
    assert_eq!(w.len(), 1, "{w:?}");
    let text = w[0].as_str().unwrap();
    assert!(text.contains("513 × 513") && text.contains("1025 × 1025"), "{text}");
    assert_eq!(warnings(1024), json!([]));
    // Over 1500 px, Flat Logo still traces at its reduced size: no slow-trace warning.
    assert_eq!(warnings(1600), json!(["Flat Logo traced the image at 800 × 800 pixels, averaged down from 1600 × 1600."]));
}

#[test]
fn image_trace_make_places_paths_over_the_image() {
    let mut s = session();
    let img = image_doc(&mut s);
    let r = s.execute("imageTrace.make", &json!({"params": {"ignoreWhite": true}})).unwrap();
    assert_eq!(r["paths"], 1);
    let g = NodeId(r["id"].as_u64().unwrap());
    let n = node(&s, g);
    assert_eq!(n.name.as_deref(), Some("Image Trace"));
    let first = &n.children().unwrap()[0];
    assert!(matches!(first.kind, NodeKind::Image(_)) && !first.visible && first.id == img);
    let paths = traced_paths(&n);
    assert_eq!(paths.len(), 1);
    let (p, rule) = outline(&paths[0]);
    let want = std::f64::consts::PI * 60.0 * 60.0; // r = 30 px × 2
    let got = vectorcraft_pathops::area(&p, rule);
    assert!((got - want).abs() / want < 0.03, "{got} vs {want}");
    let bb = p.bounds().unwrap();
    assert!((bb.center().x - 150.0).abs() < 1.5 && (bb.center().y - 150.0).abs() < 1.5, "{bb:?}");
    assert_eq!(paths[0].appearance.fill_paint(), Paint::solid(Color::rgb8(0, 0, 0)));
}

/// Shapes: the disc of the test image comes out as a live ellipse where the image lies, the same
/// circle as its traced path; without the option it is a plain path.
#[test]
fn image_trace_shapes_makes_live_shapes() {
    let mut s = session();
    image_doc(&mut s);
    let r = s.execute("imageTrace.make", &json!({"params": {"ignoreWhite": true, "shapes": true}})).unwrap();
    let n = node(&s, NodeId(r["id"].as_u64().unwrap()));
    let paths = traced_paths(&n);
    assert_eq!(paths.len(), 1);
    let NodeKind::Path { path, live: Some(live), .. } = &paths[0].kind else { panic!("not a live shape: {:?}", paths[0].kind) };
    let vectorcraft_doc::LiveShape::Ellipse { w, h, .. } = **live else { panic!("{live:?}") };
    // A circle of r = 30 px, drawn twice the size.
    assert!(w == h && (w - 60.0).abs() < 0.75, "{w} × {h}");
    let bb = path.bounds().unwrap();
    assert!((bb.center().x - 150.0).abs() < 1.5 && (bb.center().y - 150.0).abs() < 1.5 && (bb.width() - 120.0).abs() < 1.5, "{bb:?}");
    assert_eq!(*path, live.to_path());
    assert_eq!(paths[0].appearance.fill_paint(), Paint::solid(Color::rgb8(0, 0, 0)));
    // Off, as documents saved before have it: a path.
    let mut s = session();
    image_doc(&mut s);
    let r = s.execute("imageTrace.make", &json!({"params": {"ignoreWhite": true}})).unwrap();
    let n = node(&s, NodeId(r["id"].as_u64().unwrap()));
    assert!(matches!(traced_paths(&n)[0].kind, NodeKind::Path { live: None, .. }));
}

/// Auto Grouping: a ring and the dot in its hole are a group (the ring first), the square beside
/// them is not in it; the picture is the same as without, and the group traces again, expands
/// and releases as any other.
#[test]
fn image_trace_auto_grouping_groups_what_is_inside() {
    let art = vectorcraft_trace::Raster::from_fn(120, 80, |x, y| {
        let d = (x as f64 + 0.5 - 40.0).hypot(y as f64 + 0.5 - 40.0);
        if d <= 6.0 || (90..110).contains(&x) && (30..50).contains(&y) {
            [200, 0, 0, 255]
        } else if (16.0..=30.0).contains(&d) {
            [0, 0, 160, 255]
        } else {
            [255, 255, 255, 255]
        }
    });
    let make = |grouping: bool| {
        let mut s = session();
        add_image(&mut s, &art, Affine::IDENTITY);
        let r = s.execute("imageTrace.make", &json!({"preset": "6 Colors", "params": {"ignoreWhite": true, "autoGrouping": grouping}})).unwrap();
        assert_eq!(r["paths"], 3);
        let g = NodeId(r["id"].as_u64().unwrap());
        (s, g)
    };
    let (mut s, g) = make(true);
    let items = traced_paths(&node(&s, g));
    assert_eq!(items.len(), 2);
    let group = items.iter().find(|n| matches!(n.kind, NodeKind::Group { .. })).expect("a group");
    let inside = group.children().unwrap();
    assert_eq!(inside.len(), 2);
    assert_eq!(inside[0].appearance.fill_paint(), Paint::solid(Color::rgb8(0, 0, 160)));
    assert_eq!(inside[1].appearance.fill_paint(), Paint::solid(Color::rgb8(200, 0, 0)));
    assert!(matches!(items.iter().find(|n| n.id != group.id).unwrap().kind, NodeKind::Path { .. }));
    // The same picture as without.
    let (flat, fg) = make(false);
    assert_eq!(traced_paths(&node(&flat, fg)).len(), 3);
    let shot = |s: &Session| {
        let img = render_doc(s, true);
        (0..120).flat_map(|x| (0..80).map(move |y| (x, y))).map(|(x, y)| img.pixel(x, y)).collect::<Vec<_>>()
    };
    assert_eq!(shot(&s), shot(&flat));
    // In black and white too, where the white inside the ring is a path traced before the ring:
    // it stays under the ring in their group (over it, the ring's inner edge would show a seam).
    let bw = |grouping: bool| {
        let mut s = session();
        add_image(&mut s, &art, Affine::IDENTITY);
        let r = s.execute("imageTrace.make", &json!({"params": {"autoGrouping": grouping}})).unwrap();
        let groups = traced_paths(&node(&s, NodeId(r["id"].as_u64().unwrap()))).iter().filter(|n| matches!(n.kind, NodeKind::Group { .. })).count();
        (s, groups)
    };
    let (grouped, flat) = (bw(true), bw(false));
    assert!(grouped.1 > 0 && flat.1 == 0);
    assert_eq!(shot(&grouped.0), shot(&flat.0));
    // Traced again without the option, it is flat again; expanded, the groups stay.
    let r = s.execute("imageTrace.make", &json!({"preset": "6 Colors", "params": {"ignoreWhite": true}})).unwrap();
    assert_eq!(traced_paths(&node(&s, NodeId(r["id"].as_u64().unwrap()))).len(), 3);
    s.execute("imageTrace.make", &json!({"preset": "6 Colors", "params": {"ignoreWhite": true, "autoGrouping": true}})).unwrap();
    let r = s.execute("imageTrace.expand", &json!({})).unwrap();
    let expanded = node(&s, NodeId(r["ids"][0].as_u64().unwrap()));
    assert_eq!(expanded.children().unwrap().len(), 2);
}

/// Gradients: an image that is one linear gradient is one path filled with it, the gradient's line
/// where the image lies; without the option it is bands of flat colour, as saved documents have it.
#[test]
fn image_trace_gradients_fills_with_a_gradient() {
    let art = vectorcraft_trace::Raster::from_fn(100, 60, |x, _| {
        let t = (f64::from(x) + 0.5) / 100.0;
        [(20.0 + 200.0 * t) as u8, 40, (220.0 - 180.0 * t) as u8, 255]
    });
    let make = |gradients: bool| {
        let mut s = session();
        add_image(&mut s, &art, Affine::translate((50.0, 50.0)) * Affine::scale(2.0));
        let r = s.execute("imageTrace.make", &json!({"preset": "6 Colors", "params": {"gradients": gradients}})).unwrap();
        traced_paths(&node(&s, NodeId(r["id"].as_u64().unwrap())))
    };
    let paths = make(true);
    assert_eq!(paths.len(), 1);
    let Paint::Gradient(g) = paths[0].appearance.fill_paint() else { panic!("{:?}", paths[0].appearance.fill_paint()) };
    assert_eq!(g.gradient.kind, vectorcraft_color::GradientKind::Linear);
    assert_eq!(g.gradient.stops.len(), 2);
    // Level, from the image's left edge (x = 50) to its right one (x = 250), at any height.
    let geom = g.geom.as_ref().unwrap();
    let (a, b) = if geom.start.x < geom.end.x { (geom.start, geom.end) } else { (geom.end, geom.start) };
    assert!((a.x - 50.0).abs() < 3.0 && (b.x - 250.0).abs() < 3.0 && (a.y - b.y).abs() < 2.0, "{geom:?}");
    let flat = make(false);
    assert!(flat.len() > 1 && flat.iter().all(|n| matches!(n.appearance.fill_paint(), Paint::Solid { .. })));
}

/// A key inside `params` that isn't a setting is refused with the nearest setting, the image is
/// left as it was, and the error names the command that was run.
#[test]
fn image_trace_refuses_an_unknown_setting() {
    for id in ["imageTrace.make", "imageTrace.makeAndExpand"] {
        let mut s = session();
        let img = image_doc(&mut s);
        let e = s.execute(id, &json!({"preset": "6 Colors", "params": {"mode": "color", "colours": 3}})).unwrap_err().to_string();
        assert!(e.contains(&format!("`{id}`")) && e.contains("unknown parameter `colours`; did you mean `colors`?"), "{e}");
        assert!(matches!(node(&s, img).kind, NodeKind::Image(_)));
        let e = s.execute(id, &json!({"params": {"zz": 1}})).unwrap_err().to_string();
        assert!(e.contains("unknown parameter `zz`; the command takes: ") && e.contains("colorDetail"), "{e}");
        let e = s.execute(id, &json!({"enhanced": "yes"})).unwrap_err().to_string();
        assert!(e.contains(&format!("`{id}`")), "{e}");
        let library = json!({"preset": "6 Colors", "params": {"palette": "documentLibrary", "library": "No Such Library"}});
        let e = s.execute(id, &library).unwrap_err().to_string();
        assert!(e.contains(&format!("`{id}`")) && e.contains("no swatch library"), "{e}");
    }
}

/// Every preset's settings, as imageTrace.presets lists them, trace when passed back whole.
#[test]
fn image_trace_takes_every_preset_back_as_params() {
    let list = session().execute("imageTrace.presets", &json!({})).unwrap();
    for preset in list["presets"].as_array().unwrap() {
        for params in [&preset["params"], &preset["enhanced"]].into_iter().filter(|p| !p.is_null()) {
            let mut s = session();
            image_doc(&mut s);
            let r = s.execute("imageTrace.make", &json!({"preset": preset["name"], "params": params}));
            assert!(r.is_ok(), "{}: {r:?}", preset["name"]);
        }
    }
}

/// A preset's enhanced version: the preset with Auto Grouping and Shapes, under the preset's name.
#[test]
fn image_trace_enhanced_presets() {
    let mut s = session();
    image_doc(&mut s);
    let r = s.execute("imageTrace.make", &json!({"preset": "6 Colors", "enhanced": true})).unwrap();
    let n = node(&s, NodeId(r["id"].as_u64().unwrap()));
    let stored = n.extras().trace.clone().unwrap();
    assert_eq!(stored["preset"], "6 Colors");
    assert_eq!((stored["params"]["shapes"].as_bool(), stored["params"]["autoGrouping"].as_bool()), (Some(true), Some(true)));
    // The disc of the test image is a live ellipse, in a group with nothing (it has nothing inside).
    let shapes: Vec<_> = traced_paths(&n).into_iter().filter(|c| matches!(&c.kind, NodeKind::Path { live: Some(_), .. })).collect();
    assert_eq!(shapes.len(), 1);
    // Listed with each preset; the legacy preset is as it was.
    let list = s.execute("imageTrace.presets", &json!({})).unwrap();
    let six = list["presets"].as_array().unwrap().iter().find(|p| p["name"] == "6 Colors").unwrap().clone();
    assert_eq!((six["params"]["shapes"].as_bool(), six["enhanced"]["shapes"].as_bool()), (Some(false), Some(true)));
    assert_eq!(list["presets"].as_array().unwrap().iter().find(|p| p["name"] == "High Fidelity Photo").unwrap()["enhanced"]["shapes"], false);
    assert!(s.execute("imageTrace.make", &json!({"preset": "No Such", "enhanced": true})).is_err());
    // Flat Logo, a preset of our own, has no enhanced version.
    assert!(list["presets"].as_array().unwrap().iter().find(|p| p["name"] == "Flat Logo").unwrap()["enhanced"].is_null());
}

#[test]
fn image_trace_release_and_expand() {
    let mut s = session();
    let img = image_doc(&mut s);
    s.execute("imageTrace.make", &json!({"preset": "Black and White Logo"})).unwrap();
    let r = s.execute("imageTrace.release", &json!({})).unwrap();
    assert_eq!(ids(&r), vec![img]);
    assert!(node(&s, img).visible);
    assert_eq!(layer_children(&s), 1);

    let r = s.execute("imageTrace.make", &json!({"id": img.0})).unwrap();
    let g = NodeId(r["id"].as_u64().unwrap());
    // Re-trace an existing Image Trace group with other parameters.
    let r2 = s.execute("imageTrace.make", &json!({"id": g.0, "params": {"ignoreWhite": true}})).unwrap();
    assert_eq!(r2["paths"], 1);
    let g = NodeId(r2["id"].as_u64().unwrap());
    s.execute("imageTrace.expand", &json!({})).unwrap();
    let n = node(&s, g);
    assert!(n.name.is_none());
    assert!(n.children().unwrap().iter().all(|c| !matches!(c.kind, NodeKind::Image(_))));
    assert!(s.execute("imageTrace.release", &json!({})).is_err());
}

#[test]
fn image_trace_make_and_expand_and_color_presets() {
    let mut s = session();
    let r = vectorcraft_trace::Raster::from_fn(60, 60, |x, y| match (x < 30, y < 30) {
        (true, true) => [230, 20, 20, 255],
        (false, true) => [20, 200, 30, 255],
        (true, false) => [20, 30, 220, 255],
        (false, false) => [250, 230, 10, 255],
    });
    add_image(&mut s, &r, Affine::IDENTITY);
    let res = s.execute("imageTrace.makeAndExpand", &json!({"preset": "16 Colors", "params": {"colors": 4}})).unwrap();
    assert_eq!(res["colors"], 4);
    assert_eq!(res["paths"], 4);
    let g = node(&s, NodeId(res["id"].as_u64().unwrap()));
    assert!(g.name.is_none());
    assert_eq!(g.children().unwrap().len(), 4);
    assert!(s.execute("imageTrace.make", &json!({"preset": "No Such Preset"})).is_err());
    assert!(s.execute("imageTrace.make", &json!({"params": {"mode": "rainbow"}})).is_err());
}

#[test]
fn image_trace_palettes_and_swatch_libraries() {
    let mut s = session();
    let quads = vectorcraft_trace::Raster::from_fn(60, 60, |x, y| match (x < 30, y < 30) {
        (true, true) => [230, 20, 20, 255],
        (false, true) => [20, 200, 30, 255],
        (true, false) => [20, 30, 220, 255],
        (false, false) => [250, 230, 10, 255],
    });
    add_image(&mut s, &quads, Affine::IDENTITY);
    let fills = |s: &Session, id: &Value| -> Vec<Color> {
        traced_paths(&node(s, NodeId(id.as_u64().unwrap()))).iter().filter_map(|n| n.appearance.fill_paint().color()).collect()
    };
    // Document Library: every fill is one of the document's swatch colours, as it is.
    let r = s.execute("imageTrace.make", &json!({"preset": "6 Colors", "params": {"palette": "documentLibrary"}})).unwrap();
    let doc = crate::cmd::swatchlib::library_colors(&s, "document").unwrap();
    let got = fills(&s, &r["id"]);
    assert!(!got.is_empty() && got.iter().all(|c| doc.contains(c)), "{got:?}");
    let t = node(&s, NodeId(r["id"].as_u64().unwrap())).extras().trace.clone().unwrap();
    assert_eq!((t["params"]["palette"].as_str(), t["params"]["library"].as_str()), (Some("documentLibrary"), Some("document")));
    assert!(t["params"].get("swatches").is_none(), "looked up again at every trace");
    // A swatch library by id: the web-safe colours.
    let r = s
        .execute(
            "imageTrace.make",
            &json!({"id": r["id"], "preset": "6 Colors", "params": {"palette": "documentLibrary", "library": "web-safe-216", "colors": 3}}),
        )
        .unwrap();
    let got = fills(&s, &r["id"]);
    assert!((1..=3).contains(&got.len()), "{got:?}");
    assert!(got.iter().all(|c| c.to_rgba8(1.0)[..3].iter().all(|v| v % 51 == 0)), "{got:?}");
    // Full Tone and Automatic keep the four flat colours.
    let mut id = r["id"].clone();
    for palette in ["fullTone", "automatic"] {
        let r = s.execute("imageTrace.make", &json!({"id": id, "preset": "6 Colors", "params": {"palette": palette, "colorDetail": 50}})).unwrap();
        assert_eq!(r["colors"], 4, "{palette}");
        id = r["id"].clone();
    }
    let mut bad = |p: Value| s.execute("imageTrace.make", &json!({"preset": "6 Colors", "params": p})).is_err();
    assert!(bad(json!({"palette": "documentLibrary", "library": "No Such Library"})));
    assert!(bad(json!({"palette": "rainbow"})));
}

#[test]
fn image_trace_document_library_uses_a_color_group() {
    let mut s = session();
    let r = vectorcraft_trace::Raster::from_fn(60, 60, |x, y| match (x < 30, y < 30) {
        (true, _) => [200, 40, 40, 255],
        (false, true) => [40, 160, 60, 255],
        (false, false) => [50, 60, 200, 255],
    });
    add_image(&mut s, &r, Affine::IDENTITY);
    // Ungrouped swatches of the image's own colors, and a group of colors a little off them.
    s.execute("swatch.new", &json!({"colors": ["#c82828", "#28a03c", "#323cc8"]})).unwrap();
    s.execute("swatch.newGroup", &json!({"name": "Brand", "colors": ["#b43c3c", "#3c8c50", "#4650b4"]})).unwrap();
    s.execute("swatch.newGroup", &json!({"name": "Empty"})).unwrap();
    s.execute("swatch.newGroup", &json!({"name": "Été", "colors": ["#b43c3c", "#3c8c50", "#4650b4"]})).unwrap();
    let hex = |c: &Color| {
        let [r, g, b, _] = c.to_rgba8(1.0);
        format!("#{r:02x}{g:02x}{b:02x}")
    };
    let trace = |s: &mut Session, extra: Value| {
        let mut params = json!({"mode": "color", "palette": "documentLibrary", "library": "document", "colors": 3});
        params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        s.execute("imageTrace.makeAndExpand", &json!({"params": params}))
    };
    let fills = |s: &Session, id: &Value| -> Vec<String> {
        let g = node(s, NodeId(id.as_u64().unwrap()));
        let mut v: Vec<String> = g.children().unwrap().iter().filter_map(|n| n.appearance.fill_paint().color()).map(|c| hex(&c)).collect();
        v.sort();
        v.dedup();
        v
    };
    // Every swatch: the image's own colors win.
    let r = trace(&mut s, json!({})).unwrap();
    assert_eq!(fills(&s, &r["id"]), ["#28a03c", "#323cc8", "#c82828"]);
    s.execute("edit.undo", &json!({})).unwrap();
    // The group: its colors only, and its name matches whatever its case.
    for name in ["Brand", "brand", "Été", "été", "ÉTÉ"] {
        let r = trace(&mut s, json!({"colorGroup": name})).unwrap();
        assert_eq!(fills(&s, &r["id"]), ["#3c8c50", "#4650b4", "#b43c3c"], "{name}");
        s.execute("edit.undo", &json!({})).unwrap();
    }
    // An empty name is every swatch.
    let r = trace(&mut s, json!({"colorGroup": ""})).unwrap();
    assert_eq!(fills(&s, &r["id"]), ["#28a03c", "#323cc8", "#c82828"]);
    s.execute("edit.undo", &json!({})).unwrap();
    let err = |s: &mut Session, extra: Value| trace(s, extra).unwrap_err().to_string();
    assert!(err(&mut s, json!({"colorGroup": "Nope"})).contains("no color group `Nope`"));
    assert!(err(&mut s, json!({"colorGroup": "Empty"})).contains("`Empty` has no colours"));
    assert!(err(&mut s, json!({"colorGroup": "Brand", "library": "web-safe-216"})).contains("needs `library: \"document\"`"));
    // Every other palette refuses a group too, in any mode; an empty name is no group.
    for palette in ["limited", "fullTone", "automatic"] {
        let e = err(&mut s, json!({"colorGroup": "Brand", "palette": palette}));
        assert!(e.contains("`colorGroup` needs `palette: \"documentLibrary\"`"), "{palette}: {e}");
    }
    let e = err(&mut s, json!({"colorGroup": "Brand", "mode": "grayscale", "palette": "limited"}));
    assert!(e.contains("needs `palette: \"documentLibrary\"`"), "{e}");
    // Document Library uses the group in Color mode only.
    for mode in ["blackAndWhite", "grayscale", "logo"] {
        let e = err(&mut s, json!({"colorGroup": "Brand", "mode": mode}));
        assert!(e.contains("`colorGroup` needs `mode: \"color\"`"), "{mode}: {e}");
    }
    s.execute("imageTrace.makeAndExpand", &json!({"params": {"mode": "color", "palette": "limited", "colors": 3, "colorGroup": ""}})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    trace(&mut s, json!({"mode": "grayscale", "colorGroup": ""})).unwrap();
}

#[test]
fn image_trace_create_strokes_makes_stroked_centre_lines() {
    let mut s = session();
    // A 3 px wide horizontal line from x = 10 to 90 at y = 50, placed at 2× from (100, 100).
    let r =
        vectorcraft_trace::Raster::from_fn(100, 100, |x, y| if (10..90).contains(&x) && (49..52).contains(&y) { [0, 0, 0, 255] } else { [255; 4] });
    add_image(&mut s, &r, Affine::translate((100.0, 100.0)) * Affine::scale(2.0));
    let res = s.execute("imageTrace.make", &json!({"params": {"ignoreWhite": true, "strokes": true, "strokeWidth": 5}})).unwrap();
    assert_eq!(res["paths"], 1);
    let g = NodeId(res["id"].as_u64().unwrap());
    let paths = traced_paths(&node(&s, g));
    let line = &paths[0];
    assert!(line.appearance.fill_paint().color().is_none(), "not filled");
    let st = line.appearance.items.iter().find_map(|i| if let AppearanceItem::Stroke(st) = i { Some(st) } else { None }).expect("stroked");
    assert!((st.width - 6.0).abs() < 1.0, "3 px at 2× is about 6 pt: {}", st.width);
    // (The line is cut square at its ends, and so is its stroke.)
    assert_eq!((st.cap, st.join), (LineCap::Square, LineJoin::Round));
    assert_eq!(st.paint, Paint::solid(Color::rgb8(0, 0, 0)));
    let sp = &line.path_data().unwrap().subpaths[0];
    assert!(!sp.closed);
    let (a, b) = (sp.anchors[0].p, sp.anchors[sp.anchors.len() - 1].p);
    assert!((a.y - 201.0).abs() < 1.5 && (b.y - 201.0).abs() < 1.5 && (a.x - b.x).abs() > 140.0, "{a:?} … {b:?}");
    let t = node(&s, g).extras().trace.clone().unwrap();
    assert_eq!((t["params"]["strokes"].as_bool(), t["params"]["strokeWidth"].as_f64()), (Some(true), Some(5.0)));
    // Expand keeps the stroked line.
    s.execute("imageTrace.expand", &json!({})).unwrap();
    assert!(node(&s, g).children().unwrap()[0].appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(_))));
    // Create needs Fills or Strokes, and a stroke width.
    s.execute("edit.undo", &json!({})).unwrap();
    for p in [json!({"fills": false, "strokes": false}), json!({"strokes": true, "strokeWidth": 0}), json!({"strokeWidth": -3})] {
        assert!(s.execute("imageTrace.make", &json!({"id": g.0, "params": p})).is_err(), "{p}");
    }
}

#[test]
fn image_trace_presets_query_and_errors() {
    let mut s = session();
    let r = s.execute("imageTrace.presets", &json!({})).unwrap();
    let names: Vec<&str> = r["presets"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 15);
    assert!(names.contains(&"Auto Color") && names.contains(&"Outline"));
    assert!(names.contains(&"Technical Drawing") && names.contains(&"Shades of Gray") && names.contains(&"Flat Logo"));
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    assert!(s.execute("imageTrace.make", &json!({"id": a.0})).is_err(), "not an image");
    assert!(find_command("imageTrace.make").unwrap().menu == ["Object", "Image Trace"]);
    assert!(find_command("livePaint.make").unwrap().menu == ["Object", "Live Paint"]);
}

/// A soft-edged red disc (r = 30 px at (50.3, 49.6)) on transparency, placed at (50, 50) scaled ×2.
fn soft_logo_doc(s: &mut Session) -> NodeId {
    let r = vectorcraft_trace::Raster::from_fn(100, 100, |x, y| {
        let d = (x as f64 + 0.5 - 50.3).hypot(y as f64 + 0.5 - 49.6);
        let cover = (0.5 + (30.0 - d) * 0.7).clamp(0.0, 1.0);
        [200, 40, 60, (cover * 255.0).round() as u8]
    });
    add_image(s, &r, Affine::translate((50.0, 50.0)) * Affine::scale(2.0))
}

#[test]
fn image_trace_flat_logo_preset_makes_a_true_circle() {
    let mut s = session();
    soft_logo_doc(&mut s);
    let r = s.execute("imageTrace.make", &json!({"preset": "Flat Logo"})).unwrap();
    assert_eq!((r["paths"].as_u64(), r["colors"].as_u64()), (Some(1), Some(1)));
    assert!(r["anchors"].as_u64().unwrap() <= 6, "a circle is four anchors: {r}");
    let g = NodeId(r["id"].as_u64().unwrap());
    let n = node(&s, g);
    let t = n.extras().trace.clone().expect("settings stored");
    assert_eq!((t["preset"].as_str(), t["params"]["mode"].as_str()), (Some("Flat Logo"), Some("logo")));
    let paths = traced_paths(&n);
    let (p, rule) = outline(&paths[0]);
    let want = std::f64::consts::PI * 60.0 * 60.0;
    let got = vectorcraft_pathops::area(&p, rule);
    assert!((got - want).abs() / want < 0.01, "area {got} vs {want}");
    let bb = p.bounds().unwrap();
    // The disc is at (50.3, 49.6) in a x2 image placed at (50, 50): centre (150.6, 149.2).
    assert!((bb.center().x - 150.6).abs() < 0.3 && (bb.center().y - 149.2).abs() < 0.3, "{bb:?}");
}

#[test]
fn image_trace_flat_logo_refuses_a_gradient_and_a_bad_palette() {
    let mut s = session();
    let grad = vectorcraft_trace::Raster::from_fn(64, 32, |x, y| [(x * 4) as u8, 255 - (x * 4) as u8, (y * 6) as u8, 255]);
    add_image(&mut s, &grad, Affine::IDENTITY);
    let e = s.execute("imageTrace.make", &json!({"preset": "Flat Logo"})).unwrap_err().to_string();
    assert!(e.contains("flat-colour") && e.contains("Color mode"), "{e}");
    let mut s = session();
    soft_logo_doc(&mut s);
    let e = s.execute("imageTrace.make", &json!({"preset": "Flat Logo", "params": {"logoColors": ["crimson"]}})).unwrap_err().to_string();
    assert!(e.contains("#rrggbb"), "{e}");
    // The named colours are used as given.
    let r = s.execute("imageTrace.makeAndExpand", &json!({"preset": "Flat Logo", "params": {"logoColors": ["#c8283c"]}})).unwrap();
    assert_eq!(r["colors"], 1);
}

#[test]
fn image_trace_object_remembers_its_settings() {
    let mut s = session();
    image_doc(&mut s);
    let r = s.execute("imageTrace.make", &json!({"preset": "6 Colors"})).unwrap();
    let g = NodeId(r["id"].as_u64().unwrap());
    let t = node(&s, g).extras().trace.clone().expect("settings stored");
    assert_eq!(t["preset"], "6 Colors");
    assert_eq!(t["params"]["colors"], 6);
    // Changing a parameter makes it Custom; the settings survive save/open.
    let r = s.execute("imageTrace.make", &json!({"id": g.0, "preset": "6 Colors", "params": {"colors": 9}})).unwrap();
    let g = NodeId(r["id"].as_u64().unwrap());
    let d = s.doc().unwrap().doc.clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&d, false)).unwrap();
    let t = back.node(g).unwrap().extras().trace.clone().unwrap();
    assert_eq!((t["preset"].as_str(), t["params"]["colors"].as_u64()), (Some("Custom"), Some(9)));
    // Expanded traces are plain groups.
    s.execute("imageTrace.expand", &json!({})).unwrap();
    let st = s.doc().unwrap();
    let id = st.selection.in_paint_order(&st.doc)[0];
    assert!(st.doc.node(id).unwrap().extras().trace.is_none());
}

/// A black disc (r = 30 px) on light blue, 100 × 100 px, placed at 2× from (50, 50): traced in
/// Black and White with Ignore White, the blue drops out and one disc (r = 60 pt at (150, 150))
/// remains.
fn disc_on_blue(s: &mut Session) -> NodeId {
    let r = vectorcraft_trace::Raster::from_fn(100, 100, |x, y| {
        let (dx, dy) = (x as f64 + 0.5 - 50.0, y as f64 + 0.5 - 50.0);
        if dx * dx + dy * dy <= 900.0 { [0, 0, 0, 255] } else { [170, 210, 255, 255] }
    });
    add_image(s, &r, Affine::translate((50.0, 50.0)) * Affine::scale(2.0));
    let r = s.execute("imageTrace.make", &json!({"params": {"ignoreWhite": true}})).unwrap();
    assert_eq!(r["paths"], 1);
    NodeId(r["id"].as_u64().unwrap())
}

/// The document rendered as the canvas draws it (`trace_views`) or as exports do.
fn render_doc(s: &Session, trace_views: bool) -> vectorcraft_render::Rendered {
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), trace_views, ..Default::default() };
    vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, Affine::IDENTITY, &opts)
}

#[test]
fn image_trace_view_is_stored_kept_and_validated() {
    let mut s = session();
    let g = disc_on_blue(&mut s);
    assert_eq!(node(&s, g).extras().trace.clone().unwrap()["view"], "tracingResult");
    let r = s.execute("imageTrace.setView", &json!({"view": "outlinesWithSourceImage"})).unwrap();
    assert_eq!((ids(&r), r["view"].as_str()), (vec![g], Some("outlinesWithSourceImage")));
    assert_eq!(node(&s, g).trace_view(), vectorcraft_doc::TraceView::OutlinesWithSource);
    // Setting the view it has already adds no undo step.
    let steps = s.doc().unwrap().history.undo.len();
    s.execute("imageTrace.setView", &json!({"id": g.0, "view": "outlinesWithSourceImage"})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), steps);
    // Tracing again keeps it, unless another one is asked for; it survives save and open.
    let g = NodeId(s.execute("imageTrace.make", &json!({"id": g.0, "params": {"ignoreWhite": true, "noise": 30}})).unwrap()["id"].as_u64().unwrap());
    assert_eq!(node(&s, g).trace_view(), vectorcraft_doc::TraceView::OutlinesWithSource);
    let g = NodeId(s.execute("imageTrace.make", &json!({"id": g.0, "view": "SourceImage"})).unwrap()["id"].as_u64().unwrap());
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(back.node(g).unwrap().trace_view(), vectorcraft_doc::TraceView::Source);
    // Bad views and other objects are refused.
    assert!(s.execute("imageTrace.setView", &json!({"view": "sepia"})).is_err());
    assert!(s.execute("imageTrace.setView", &json!({})).is_err());
    assert!(s.execute("imageTrace.make", &json!({"view": 3})).is_err());
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    assert!(s.execute("imageTrace.setView", &json!({"id": a.0, "view": "outlines"})).is_err());
}

#[test]
fn image_trace_views_draw_on_screen_only_and_expand_to_the_result() {
    let mut s = session();
    let g = disc_on_blue(&mut s);
    let dark = |p: [u8; 4]| p[0] < 60 && p[1] < 60 && p[2] < 60;
    let white = |p: [u8; 4]| p[0] > 245 && p[1] > 245 && p[2] > 245;
    let blue = |p: [u8; 4]| p[0] < 200 && p[2] > 245;
    let (centre, beside) = ((150, 150), (70, 70));
    let at = |img: &vectorcraft_render::Rendered, (x, y): (u32, u32)| img.pixel(x, y);
    // The outline is a 1 px line where the disc's right side is (x = 210, anti-aliased).
    let outlined = |img: &vectorcraft_render::Rendered| (205..=215).any(|x| img.pixel(x, 150)[0] < 128);
    let mut expanded = None;
    for view in vectorcraft_doc::TraceView::ALL {
        s.execute("imageTrace.setView", &json!({"id": g.0, "view": view.id()})).unwrap();
        let screen = render_doc(&s, true);
        let (c, b) = (at(&screen, centre), at(&screen, beside));
        assert_eq!(dark(c), view.shows_result() || view.shows_source(), "{view:?}: the disc's middle {c:?}");
        assert_eq!(blue(b), view.shows_source(), "{view:?}: the image's background {b:?}");
        assert!(view.shows_source() || white(b), "{view:?}: nothing beside the disc {b:?}");
        if view == vectorcraft_doc::TraceView::Outlines {
            assert!(outlined(&screen), "{view:?}: the disc's outline");
        }
        // Exports draw the tracing result whatever the view.
        let export = render_doc(&s, false);
        assert!(dark(at(&export, centre)) && white(at(&export, beside)), "{view:?}: export");
        // Expand keeps the traced shapes, as traced, whatever the view.
        s.execute("imageTrace.expand", &json!({"id": g.0})).unwrap();
        let shapes: Vec<_> = node(&s, g).children().unwrap().iter().map(|c| (c.path_data().cloned(), c.appearance.clone())).collect();
        assert_eq!(*expanded.get_or_insert_with(|| shapes.clone()), shapes, "{view:?}: expanded");
        s.execute("edit.undo", &json!({})).unwrap();
    }
}

/// #569: merging the regions of a rosette of unfilled ellipses (each off the centre, petals
/// overlapping) merges every region the drags touch into one shape whose outline is only the
/// merged area's border: no edge between merged regions stays in, as a stray line or a spur.
/// Petal `k` turns about the center by 45° × `k`, or by each angle `turns` lists for it in turn.
#[track_caller]
fn rosette_rings_merge_cleanly(turns: &[(u32, &[f64])]) {
    let mut s = session();
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    let c = [400.0, 320.0];
    let mut petals = vec![];
    for k in 0..8 {
        let id =
            s.execute("shape.ellipse", &json!({"x": c[0] - 60.0, "y": c[1] - 172.0, "width": 120, "height": 324})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        let angles = turns.iter().find(|t| t.0 == k).map_or_else(|| vec![45.0 * f64::from(k)], |t| t.1.to_vec());
        for angle in angles {
            s.execute("object.rotate", &json!({"angle": angle, "origin": c})).unwrap();
        }
        petals.push(id);
    }
    s.execute("select.set", &json!({"ids": petals})).unwrap();
    // Drags around the ring of petals, from its inside out; the outer ones miss the petals.
    let mut merged = None;
    for radius in [110.0, 150.0, 190.0, 230.0] {
        let pts: Vec<[f64; 2]> = (0..=120)
            .map(|i| {
                let a = std::f64::consts::TAU * f64::from(i) / 120.0;
                [c[0] + radius * a.cos(), c[1] + radius * a.sin()]
            })
            .collect();
        if let Ok(r) = s.execute("shapeBuilder.merge", &json!({"points": pts})) {
            merged = r["merged"].as_u64().map(NodeId);
        }
    }
    let (ring, rule) = outline(&node(&s, merged.expect("the drags merged")));
    let inside = |p: kurbo::Point| kurbo::Shape::winding(&ring.to_bezpath(), p) != 0;
    for k in 0..8 {
        let a = std::f64::consts::FRAC_PI_4 * f64::from(k) - std::f64::consts::FRAC_PI_2;
        let tip = kurbo::Point::new(c[0] + 165.0 * a.cos(), c[1] + 165.0 * a.sin());
        assert!(inside(tip), "petal {k}'s tip at {tip:?} is in the merged shape");
    }
    // What is left of the petals stays out of the merged area (a piece left over it is a stray
    // outline inside it).
    let merged = merged.unwrap();
    for id in s.doc().unwrap().selection.objects.clone().into_iter().filter(|id| *id != merged) {
        let (p, r) = outline(&node(&s, id));
        let over = vectorcraft_pathops::boolean(&p, r, &ring, rule, vectorcraft_pathops::BoolOp::Intersect);
        let a = vectorcraft_pathops::area(&over, FillRule::NonZero);
        assert!(a < 1.0, "{id:?} overlaps the merged shape by {a} pt²");
    }
    // No outline is a sliver or carries a spur (2 × area / length: its mean width).
    for id in s.doc().unwrap().selection.objects.clone() {
        let (p, r) = outline(&node(&s, id));
        for sp in &p.subpaths {
            let one = PathData::new(vec![sp.clone()]);
            let width = 2.0 * vectorcraft_pathops::area(&one, r) / one.length();
            assert!(width > 1.0, "{id:?} has an outline {width:.3} pt wide on average, {:.1} pt long", one.length());
        }
    }
    // The ring is one outline around the centre (the regions the drags missed aside): the
    // petals' tips are no outlines of their own beside it, apart by a hairline.
    let big = ring.subpaths.iter().filter(|sp| vectorcraft_pathops::area(&PathData::new(vec![(*sp).clone()]), rule) > 100.0).count();
    assert_eq!(big, 2, "{:?}", ring.subpaths.iter().map(|sp| vectorcraft_pathops::area(&PathData::new(vec![sp.clone()]), rule)).collect::<Vec<_>>());
    // An inner edge would be a second outline over the merged area: normalising would drop it.
    let clean = vectorcraft_pathops::normalize(&ring, rule);
    assert!((ring.length() - clean.length()).abs() < 1.0, "{} pt of outline, {} without inner edges", ring.length(), clean.length());
}

#[test]
fn shape_builder_merges_rosette_rings_cleanly() {
    rosette_rings_merge_cleanly(&[]);
}

/// #1770: the rosette merges as cleanly with a petal turned by an equivalent angle (-180° for 180°,
/// -855° for 225°, -45° for 315°) or in two steps, which moves its outline by a rounding error.
#[test]
fn shape_builder_merges_rosette_rings_cleanly_however_a_petal_turns() {
    rosette_rings_merge_cleanly(&[(4, &[-180.0])]);
    rosette_rings_merge_cleanly(&[(5, &[-855.0])]);
    rosette_rings_merge_cleanly(&[(7, &[-45.0])]);
    rosette_rings_merge_cleanly(&[(4, &[100.0, 80.0])]);
}
