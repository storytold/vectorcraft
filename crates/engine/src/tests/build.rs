//! Tests for Shape Builder, Live Paint and Image Trace commands and tools.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::appearance::{Effect, FillLayer, StrokeLayer};
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

fn parent(s: &Session, id: NodeId) -> Option<NodeId> {
    s.doc().unwrap().doc.parent_of(id)
}

/// The merge replaces only the shapes it uses: a selected type object stays as it was.
#[test]
fn shape_builder_merge_keeps_a_selected_type_object() {
    let mut s = session();
    let (a, b) = two(&mut s);
    let t = NodeId(s.execute("text.create", &json!({"x": 300, "y": 300, "text": "Label"})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [a.0, b.0, t.0]})).unwrap();
    let before = node(&s, t);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[25, 50], [125, 50]]})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node(t), Some(&before));
    assert_eq!(layer_children(&s), 2, "the merged shape and the type");
    assert!(ids(&r).contains(&t), "{r}");
}

/// A hidden member of a selected group stays hidden in its group, with the merged shape.
#[test]
fn shape_builder_merge_keeps_a_hidden_group_member() {
    let mut s = session();
    let (a, b) = two(&mut s);
    let c = rect(&mut s, 300.0, 300.0, 50.0, 50.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0, c.0]})).unwrap();
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.hide", &json!({"ids": [c.0]})).unwrap();
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[25, 50], [125, 50]]})).unwrap();
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert_eq!((parent(&s, c), node(&s, c).visible, parent(&s, merged)), (Some(g), false, Some(g)));
    assert_eq!(node(&s, g).children().unwrap().len(), 2);
}

/// A locked member of a selected group is neither cut nor moved, and does not cut the others.
#[test]
fn shape_builder_merge_leaves_a_locked_group_member() {
    let mut s = session();
    let (_, b) = two(&mut s);
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.lock", &json!({"ids": [b.0]})).unwrap();
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    let before = node(&s, b);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[25, 50], [75, 50]]})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node(b), Some(&before));
    assert_eq!(parent(&s, b), Some(g));
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert!((area(&s, merged) - 10000.0).abs() < 1.0, "A whole: {}", area(&s, merged));
}

/// In a clip group, the clipping path stays and the merged shape stays inside the group.
#[test]
fn shape_builder_merge_keeps_a_clip_groups_clipping_path() {
    let mut s = session();
    let (a, b) = two(&mut s);
    let m = rect(&mut s, 0.0, 0.0, 120.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0, m.0]})).unwrap();
    let g = NodeId(s.execute("object.clippingMask.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let r = s.execute("shapeBuilder.merge", &json!({"ids": [g.0], "points": [[25, 50], [125, 50]]})).unwrap();
    let merged = NodeId(r["merged"].as_u64().unwrap());
    let gn = node(&s, g);
    let NodeKind::Group { children, clip: true } = &gn.kind else { panic!("a clip group: {:?}", gn.kind) };
    assert_eq!(children.first().map(|c| c.id), Some(m), "the clipping path");
    assert_eq!(parent(&s, merged), Some(g));
}

/// A group goes when deleting regions leaves it empty, and stays while anything is left in it.
#[test]
fn shape_builder_erase_removes_only_a_group_it_empties() {
    let mut s = session();
    two(&mut s);
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[25, 50]], "erase": true})).unwrap();
    assert_eq!((ids(&r), node(&s, g).children().unwrap().len()), (vec![g], 2));
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[60, 50], [140, 50]], "erase": true})).unwrap();
    assert!(ids(&r).is_empty() && s.doc().unwrap().doc.node(g).is_none(), "{r}");
    assert_eq!(layer_children(&s), 0);
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

/// A Shape Builder click that builds nothing puts the reason in the status bar and leaves the
/// document, its undo history and the selection as they were; a click that merges shows no status.
#[test]
fn shape_builder_click_that_builds_nothing_shows_why() {
    use vectorcraft_tools::builder::{NO_REGION, SELECT_ALL_PATHS, SELECT_PATHS};
    let mut s = session();
    let (a, _) = two(&mut s);
    let v = ViewInfo::default();
    s.select_tool("shapeBuilder", v).unwrap();
    let click = |s: &mut Session, x: f64, y: f64| {
        s.pointer(&PointerEvent::new(PointerKind::Down, x, y), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, x, y), v).unwrap()
    };
    let state = |s: &Session| {
        let st = s.doc().unwrap();
        (st.revision, st.history.undo.len(), st.history.redo.len(), st.selection.objects.clone(), st.is_dirty())
    };
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let before = state(&s);
    assert_eq!(click(&mut s, 125.0, 50.0), vec![UiRequest::Status(SELECT_ALL_PATHS.into())]);
    assert_eq!(click(&mut s, 400.0, 400.0), vec![UiRequest::Status(NO_REGION.into())]);
    assert_eq!(state(&s), before, "nothing changed");
    s.execute("select.none", &json!({})).unwrap();
    let before = state(&s);
    assert_eq!(click(&mut s, 75.0, 50.0), vec![UiRequest::Status(SELECT_PATHS.into())]);
    assert_eq!(state(&s), before, "nothing changed");
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    assert_eq!(click(&mut s, 75.0, 50.0), vec![UiRequest::Status(String::new())], "a click that merges clears the status");
    assert_eq!(s.journal.last().unwrap().0, "shapeBuilder.merge");
    assert!(click(&mut s, 25.0, 50.0).is_empty(), "the next merge shows no status");
    assert_eq!(s.journal.last().unwrap().0, "shapeBuilder.merge");
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

/// 40 squares 20 pt wide in a row, each 10 pt right of the one before (160 segments, which the
/// Shape Builder merges on another thread), all selected.
fn square_row(s: &mut Session) -> Vec<NodeId> {
    let ids: Vec<NodeId> = (0..40).map(|k| rect(s, f64::from(k) * 10.0, 0.0, 20.0, 20.0)).collect();
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    ids
}

/// The active document of `s` without its dates, so documents made in two sessions at different
/// times compare equal when their art is.
fn undated(s: &Session) -> vectorcraft_doc::Document {
    let mut d = (*s.doc().unwrap().doc).clone();
    (d.metadata.created, d.metadata.modified) = (None, None);
    d
}

/// The Shape Builder over [`square_row`], its map built, after a click at (15, 10): the merge is
/// being computed on another thread, and the document is as it was.
fn click_row(s: &mut Session, v: ViewInfo) {
    s.select_tool("shapeBuilder", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Move, 15.0, 10.0), v).unwrap();
    s.tool_poll(v, true).unwrap();
    let revision = s.doc().unwrap().revision;
    s.pointer(&PointerEvent::new(PointerKind::Down, 15.0, 10.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 15.0, 10.0), v).unwrap();
    assert!(s.tool_editing() && s.tool_waiting());
    assert_eq!(s.doc().unwrap().revision, revision, "the release returns before the merge is applied");
}

/// A click of the Shape Builder on a large selection merges on another thread, and the merge the
/// next poll applies is the command's own, as one undo step.
#[test]
fn shape_builder_merges_a_large_selection_on_another_thread() {
    use vectorcraft_tools::builder::merges_computed;
    let v = ViewInfo::default();
    let mut s = session();
    square_row(&mut s);
    click_row(&mut s, v);
    let computed = merges_computed();
    s.tool_poll(v, true).unwrap();
    assert!(!s.tool_editing());
    assert_eq!(merges_computed(), computed, "the command takes the merge computed on another thread");
    assert_eq!(s.journal.last().unwrap().0, "shapeBuilder.merge");
    assert_eq!(layer_children(&s), 41, "the overlap and what is left of the two squares over it");
    let mut direct = session();
    square_row(&mut direct);
    direct.execute("shapeBuilder.merge", &json!({"points": [[15, 10]]})).unwrap();
    assert!(undated(&s) == undated(&direct), "the same document as the command gives");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(layer_children(&s), 40, "one undo step");
}

/// What comes after a click whose merge is being computed on another thread waits for the merge
/// and runs on the merged art, as when the merge ran on the calling thread: a command (an edit,
/// an undo, a selection, a save, a batch, Lock Guides, the proof views, a preference, Add Font
/// Files), an option change, another tool, a save made outside a command, preferences applied
/// outside a command, a dialog's preview, a scrubbed field's undo scope, a Cmd-click or Cmd-drag
/// with the selection tool. The merge is the one computed on the other thread.
#[test]
fn shape_builder_applies_a_merge_before_what_comes_next() {
    use vectorcraft_tools::builder::merges_computed;
    type Next = (&'static str, fn(&mut Session, &[NodeId], &std::path::Path));
    let v = ViewInfo::default();
    let next: Vec<Next> = vec![
        ("Select All Instances", |s, _, _| {
            s.execute("symbol.selectInstances", &json!({"name": "S"})).unwrap();
            assert_eq!(s.doc().unwrap().selection.objects.len(), 1);
        }),
        ("an edit", |s, _, _| {
            s.execute("object.move", &json!({"dx": 0, "dy": 100})).unwrap();
        }),
        ("an undo", |s, _, _| {
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(layer_children(s), 40, "the undo takes the merge back");
            s.execute("edit.redo", &json!({})).unwrap();
        }),
        ("another selection", |s, _, _| {
            s.execute("select.inverse", &json!({})).unwrap();
        }),
        ("Select All", |s, _, _| {
            s.execute("select.all", &json!({})).unwrap();
            assert_eq!(s.doc().unwrap().selection.objects.len(), 41);
        }),
        ("a save", |s, _, dir| {
            s.execute("file.saveAs", &json!({"path": dir.join("a.vectorcraft").to_string_lossy()})).unwrap();
            assert!(!s.doc().unwrap().is_dirty(), "the file holds the merge");
        }),
        ("a save outside a command", |s, _, dir| {
            let p = json!({"path": dir.join("b.vectorcraft").to_string_lossy()});
            let plan = cmd::fileio::save_plan(s, cmd::fileio::SaveMode::SaveAs, &p).unwrap();
            assert_eq!(layer_children(s), 40, "planning a save leaves the document");
            let job = cmd::fileio::save_job(s, plan).unwrap();
            assert_eq!(layer_children(s), 41, "the snapshot is taken after the merge");
            drop(job);
        }),
        ("a batch", |s, _, _| {
            s.execute("command.batch", &json!({"commands": [{"command": "object.move", "params": {"dx": 0, "dy": 50}}]})).unwrap();
        }),
        ("an option change", |s, _, _| {
            s.set_tool_option_cmd(&json!({"key": "gapDetection", "value": true})).unwrap();
        }),
        ("another tool", |s, _, _| {
            s.select_tool("selection", ViewInfo::default()).unwrap();
        }),
        ("a closed document", |s, _, _| {
            s.execute("file.close", &json!({})).unwrap();
        }),
        ("Lock Guides", |s, _, _| {
            s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
        }),
        ("an option set from a panel", |s, _, _| {
            s.set_tool_option("gapDetection", &json!(true));
        }),
        ("a dialog's preview, canceled", |s, _, _| {
            s.begin_interaction("Move").unwrap();
            s.preview("object.move", &json!({"dx": 7, "dy": 0})).unwrap();
            s.preview("object.move", &json!({"dx": 9, "dy": 0})).unwrap();
            s.cancel_interaction().unwrap();
            assert_eq!(layer_children(s), 41, "canceling the preview keeps the merge");
        }),
        ("a dialog's preview, kept", |s, _, _| {
            s.begin_interaction("Move").unwrap();
            s.preview("object.move", &json!({"dx": 7, "dy": 0})).unwrap();
            s.commit_interaction().unwrap();
            let labels: Vec<&str> = s.doc().unwrap().history.undo.iter().rev().take(2).map(|e| e.label.as_str()).collect();
            assert_eq!(labels, ["Move", "Shape Builder"]);
        }),
        ("a scrubbed field, canceled", |s, _, _| {
            let scope = s.begin_undo_scope().unwrap();
            s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
            s.end_undo_scope(scope, true);
            assert_eq!(
                s.doc().unwrap().history.undo.last().map(|e| e.label.as_str()),
                Some("Shape Builder"),
                "the scrub's step is undone, not the merge"
            );
        }),
        ("a held click, then another tool", |s, _, _| {
            let v = ViewInfo::default();
            for kind in [PointerKind::Down, PointerKind::Up] {
                s.pointer(&PointerEvent::new(kind, 55.0, 10.0), v).unwrap();
            }
            assert!(s.tool_editing(), "the click is held");
            s.select_tool("selection", v).unwrap();
            let labels: Vec<&str> = s.doc().unwrap().history.undo.iter().rev().take(2).map(|e| e.label.as_str()).collect();
            assert_eq!(labels, ["Shape Builder", "Shape Builder"], "the held click merges with the Shape Builder first");
        }),
        ("a held click, then another document", |s, _, _| {
            let v = ViewInfo::default();
            for kind in [PointerKind::Down, PointerKind::Up] {
                s.pointer(&PointerEvent::new(kind, 55.0, 10.0), v).unwrap();
            }
            let first = s.active_index().unwrap();
            s.add_document(vectorcraft_doc::Document::new(100.0, 100.0), None);
            assert!(!s.tool_editing());
            let labels: Vec<&str> = s.documents()[first].history.undo.iter().rev().take(2).map(|e| e.label.as_str()).collect();
            assert_eq!(labels, ["Shape Builder", "Shape Builder"], "the held click merges in its own document first");
            s.set_active(first);
        }),
        ("a Cmd-click", |s, _, _| {
            let v = ViewInfo::default();
            let cmd = Mods { cmd: true, ..Default::default() };
            assert_eq!(s.cursor(Point::new(500.0, 300.0), cmd, v), vectorcraft_tools::Cursor::Busy, "Cmd shows the merge is computed");
            for kind in [PointerKind::Down, PointerKind::Up] {
                s.pointer(&PointerEvent::new(kind, 500.0, 300.0).with_mods(cmd), v).unwrap();
            }
            assert!(s.tool_editing(), "the click is held");
            while s.tool_waiting() {
                s.tool_poll(v, true).unwrap();
            }
            assert!(s.doc().unwrap().selection.is_empty(), "the lent selection tool deselects after the merge");
            assert_eq!(s.tool_id(), "shapeBuilder");
        }),
        ("a Cmd-drag", |s, _, _| {
            let v = ViewInfo::default();
            let cmd = Mods { cmd: true, ..Default::default() };
            for (kind, y) in [(PointerKind::Down, 10.0), (PointerKind::Drag, 60.0), (PointerKind::Drag, 110.0), (PointerKind::Up, 110.0)] {
                s.pointer(&PointerEvent::new(kind, 305.0, y).with_mods(cmd), v).unwrap();
            }
            assert!(s.tool_editing(), "the drag is held");
            while s.tool_waiting() {
                s.tool_poll(v, true).unwrap();
            }
            let labels: Vec<&str> = s.doc().unwrap().history.undo.iter().rev().take(2).map(|e| e.label.as_str()).collect();
            assert_eq!(labels, ["Move", "Shape Builder"]);
        }),
        // The proof views apply the merge first by command id (`WAIT_FOR_TOOL_EDIT`), also when
        // the view does not change and no document gets a new revision. They are left off, as
        // the other tests of the process-wide proof view expect.
        ("Separations Preview", |s, _, _| {
            s.execute("view.separationsPreview", &json!({"on": false})).unwrap();
        }),
        ("Overprint Preview", |s, _, _| {
            s.execute("view.overprintPreview", &json!({"on": false})).unwrap();
        }),
        ("Proof Colors", |s, _, _| {
            s.execute("view.proofColors", &json!({"on": false})).unwrap();
        }),
        ("Proof Setup", |s, _, _| {
            s.execute("view.proofSetup", &json!({})).unwrap();
        }),
        ("a proof view that changes", |s, _, _| {
            let _v = crate::tests_colormgmt::DefaultView::new();
            s.execute("view.proofColors", &json!({"on": true})).unwrap();
            let on = s.doc().unwrap().revision;
            s.execute("view.proofColors", &json!({"on": false})).unwrap();
            assert!(s.doc().unwrap().revision > on, "the view changes and the documents get new revisions");
        }),
        ("a preference", |s, _, _| {
            let every = s.prefs.gridline_every;
            s.execute("prefs.set", &json!({"key": "gridlineEvery", "value": every + 10.0})).unwrap();
        }),
        ("Reset Preferences", |s, _, _| {
            s.execute("prefs.reset", &json!({})).unwrap();
        }),
        ("preferences applied outside a command", |s, _, _| {
            let mut p = s.prefs.clone();
            p.gridline_every += 10.0;
            s.apply_prefs(p);
        }),
        ("Add Font Files", |s, _, _| {
            // No files: the command fails after the merge.
            assert!(s.execute("text.addFontFiles", &json!({"files": []})).is_err());
        }),
        ("a call its schema refuses", |s, _, _| {
            // The refused call adds no undo step of its own: the merge's is the last one.
            let e = s.execute("view.saved.new", &json!({"center": [0, 0], "zoom": 0})).unwrap_err().to_string();
            assert!(e.contains("`zoom`"), "{e}");
            assert!(s.doc().unwrap().doc.views.is_empty());
            assert_eq!(s.doc().unwrap().history.undo.last().map(|e| e.label.as_str()), Some("Shape Builder"));
        }),
    ];
    let _proof = crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = vectorcraft_testkit::TempDir::new("shape-builder-merge-first");
    for (what, then) in next {
        let mut s = session();
        if what == "Select All Instances" {
            let far = rect(&mut s, 500.0, 500.0, 20.0, 20.0);
            s.execute("symbol.new", &json!({"name": "S", "ids": [far.0]})).unwrap();
        }
        let ids = square_row(&mut s);
        click_row(&mut s, v);
        let computed = merges_computed();
        let journal = s.journal.len();
        then(&mut s, &ids, &dir);
        assert!(!s.tool_editing(), "{what}");
        assert_eq!(merges_computed(), computed, "{what}: the merge computed on another thread");
        assert_eq!(s.journal.since(journal).first().map(|j| j.0.as_str()), Some("shapeBuilder.merge"), "{what}: the merge comes first");
        if what != "a closed document" {
            assert!(layer_children(&s) >= 41, "{what}: the merge stays");
        }
    }
}

/// Gestures and arrow keys that come while a merge is computed on another thread are held and
/// made after it, in order: the document is the one they make one after the other, and the
/// events count as made once they are and the merge they start is applied.
#[test]
fn shape_builder_holds_gestures_made_during_a_merge() {
    type Gesture = fn(&mut Session, ViewInfo);
    let cases: [(&str, bool, Gesture); 6] = [
        ("a second click", false, |s, v| {
            for kind in [PointerKind::Down, PointerKind::Up] {
                s.pointer(&PointerEvent::new(kind, 55.0, 10.0), v).unwrap();
            }
        }),
        ("a drag", false, |s, v| {
            for (kind, x) in [(PointerKind::Down, 75.0), (PointerKind::Drag, 85.0), (PointerKind::Drag, 95.0), (PointerKind::Up, 95.0)] {
                s.pointer(&PointerEvent::new(kind, x, 10.0), v).unwrap();
            }
        }),
        ("two clicks", false, |s, v| {
            for x in [55.0, 135.0] {
                for kind in [PointerKind::Down, PointerKind::Up] {
                    s.pointer(&PointerEvent::new(kind, x, 10.0), v).unwrap();
                }
            }
        }),
        ("a Cmd-click", false, |s, v| {
            let cmd = Mods { cmd: true, ..Default::default() };
            for kind in [PointerKind::Down, PointerKind::Up] {
                s.pointer(&PointerEvent::new(kind, 500.0, 300.0).with_mods(cmd), v).unwrap();
            }
        }),
        ("a Cmd-drag and a click", false, |s, v| {
            let cmd = Mods { cmd: true, ..Default::default() };
            for (kind, y) in [(PointerKind::Down, 10.0), (PointerKind::Drag, 110.0), (PointerKind::Up, 110.0)] {
                s.pointer(&PointerEvent::new(kind, 305.0, y).with_mods(cmd), v).unwrap();
            }
            for kind in [PointerKind::Down, PointerKind::Up] {
                s.pointer(&PointerEvent::new(kind, 55.0, 10.0), v).unwrap();
            }
        }),
        ("an arrow key", true, |s, v| {
            s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
        }),
    ];
    let v = ViewInfo::default();
    let start = |swatches: bool| {
        let mut s = session();
        if swatches {
            s.select_tool("shapeBuilder", v).unwrap();
            s.set_tool_option_cmd(&json!({"key": "pickColorFrom", "value": "swatches"})).unwrap();
        }
        square_row(&mut s);
        click_row(&mut s, v);
        s
    };
    for (what, swatches, send) in cases {
        // One after the other: the merge applied before the gesture, and each merge after its own.
        let mut one = start(swatches);
        while one.tool_waiting() {
            one.tool_poll(v, true).unwrap();
        }
        send(&mut one, v);
        while one.tool_waiting() {
            one.tool_poll(v, true).unwrap();
        }
        // While the merge is computed.
        let mut s = start(swatches);
        let revision = s.doc().unwrap().revision;
        let from = s.input_count();
        send(&mut s, v);
        let to = s.input_count();
        assert!(s.tool_editing() && !s.input_applied(from, to), "{what}: held");
        assert_eq!(s.doc().unwrap().revision, revision, "{what}: nothing made before the merge");
        while s.tool_waiting() {
            s.tool_poll(v, true).unwrap();
        }
        assert!(!s.tool_editing() && s.input_applied(from, to), "{what}");
        assert!(undated(&s) == undated(&one), "{what}: the document the gestures make one after the other");
        assert_eq!(s.doc().unwrap().selection, one.doc().unwrap().selection, "{what}");
        assert_eq!(s.paint.fill, one.paint.fill, "{what}");
        let labels = |s: &Session| s.doc().unwrap().history.undo.iter().map(|e| e.label.clone()).collect::<Vec<_>>();
        assert_eq!(labels(&s), labels(&one), "{what}");
        // The click's merge alone.
        let mut merged = start(swatches);
        while merged.tool_waiting() {
            merged.tool_poll(v, true).unwrap();
        }
        let changed = labels(&s).len() > labels(&merged).len()
            || s.paint.fill != merged.paint.fill
            || s.doc().unwrap().selection != merged.doc().unwrap().selection;
        assert!(changed, "{what}: the gesture did something");
    }
}

/// A press held while a merge is computed on another thread, then dropped (a long touch, a
/// canceled touch, a second finger), is never made: once the merge is applied, the tool is at
/// rest and the document is the one the same input makes, dropped the same way, after the merge.
/// A held click before the press is still made, and so is a perspective plane key held after it.
#[test]
fn shape_builder_drops_a_press_held_during_a_merge() {
    use vectorcraft_tools::distort::perspective::{PerspectiveGrid, Plane};
    type Gesture = fn(&mut Session, ViewInfo);
    fn press(s: &mut Session, v: ViewInfo, x: f64, mods: Mods) {
        s.pointer(&PointerEvent::new(PointerKind::Down, x, 10.0).with_mods(mods), v).unwrap();
    }
    fn click(s: &mut Session, v: ViewInfo) {
        for kind in [PointerKind::Down, PointerKind::Up] {
            s.pointer(&PointerEvent::new(kind, 55.0, 10.0), v).unwrap();
        }
    }
    // What a case sends, and whether the perspective grid is shown.
    let cases: [(&str, bool, Gesture); 7] = [
        ("a press", false, |s, v| press(s, v, 75.0, Mods::default())),
        ("a drag", false, |s, v| {
            press(s, v, 75.0, Mods::default());
            s.pointer(&PointerEvent::new(PointerKind::Drag, 95.0, 10.0), v).unwrap();
        }),
        ("a Cmd-press", false, |s, v| press(s, v, 305.0, Mods { cmd: true, ..Default::default() })),
        ("a press and a key", false, |s, v| {
            press(s, v, 75.0, Mods::default());
            s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
        }),
        ("a press and an arrow key", false, |s, v| {
            press(s, v, 75.0, Mods::default());
            s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
        }),
        ("a press and a plane key", true, |s, v| {
            press(s, v, 75.0, Mods::default());
            s.tool_key(ToolKey::Digit(3), Mods::default(), v).unwrap();
        }),
        ("a click, then a press", false, |s, v| {
            click(s, v);
            press(s, v, 135.0, Mods::default());
        }),
    ];
    let v = ViewInfo::default();
    let start = |grid: bool| {
        let mut s = session();
        s.select_tool("shapeBuilder", v).unwrap();
        s.set_tool_option_cmd(&json!({"key": "pickColorFrom", "value": "swatches"})).unwrap();
        if grid {
            s.execute("perspective.grid.preset", &json!({"kind": 2})).unwrap();
        }
        square_row(&mut s);
        click_row(&mut s, v);
        s
    };
    // A tool left in the middle of a gesture keeps waiting: the polls stop after a few.
    let settle = |s: &mut Session| {
        for _ in 0..8 {
            if !s.tool_waiting() {
                return;
            }
            s.tool_poll(v, true).unwrap();
        }
    };
    let labels = |s: &Session| s.doc().unwrap().history.undo.iter().map(|e| e.label.clone()).collect::<Vec<_>>();
    let plane = |s: &Session| PerspectiveGrid::from_doc(&s.doc().unwrap().doc).unwrap().plane;
    for (what, grid, send) in cases {
        // The same input after the merge, dropped the same way.
        let mut one = start(grid);
        settle(&mut one);
        send(&mut one, v);
        one.drop_tool_press(v).unwrap();
        settle(&mut one);
        let mut s = start(grid);
        if grid {
            assert_ne!(plane(&s), Plane::Right, "{what}: the plane before the key");
        }
        let from = s.input_count();
        send(&mut s, v);
        let to = s.input_count();
        assert!(s.tool_editing() && !s.input_applied(from, to), "{what}: held");
        s.drop_tool_press(v).unwrap();
        settle(&mut s);
        assert!(!s.tool_waiting() && !s.tool_editing() && s.input_applied(from, to), "{what}");
        assert!(!s.tool_busy(), "{what}: the tool is at rest");
        assert_eq!(s.tool_id(), "shapeBuilder", "{what}: no tool is lent");
        assert!(undated(&s) == undated(&one), "{what}: the document the input makes after the merge");
        assert_eq!(labels(&s), labels(&one), "{what}");
        assert_eq!(s.doc().unwrap().selection, one.doc().unwrap().selection, "{what}");
        assert_eq!(s.paint.fill, one.paint.fill, "{what}");
        if grid {
            assert_eq!(plane(&one), Plane::Right, "{what}: the plane key is made after the merge");
            assert_eq!(plane(&s), Plane::Right, "{what}: the plane key is made");
        }
    }
}

/// When `tool.setOption` names no tool, the option is set on the tool that was active when the
/// call arrived (the Shape Builder), even when finishing its merge replays a held Cmd press that
/// switches to the Selection tool.
#[test]
fn shape_builder_option_set_while_a_cmd_press_is_held_goes_to_the_shape_builder() {
    let v = ViewInfo::default();
    let mut s = session();
    s.select_tool("shapeBuilder", v).unwrap();
    s.set_tool_option_cmd(&json!({"key": "pickColorFrom", "value": "swatches"})).unwrap();
    square_row(&mut s);
    click_row(&mut s, v);
    s.pointer(&PointerEvent::new(PointerKind::Down, 305.0, 10.0).with_mods(Mods { cmd: true, ..Default::default() }), v).unwrap();
    assert!(s.tool_editing(), "the press is held");
    let out = s.set_tool_option_cmd(&json!({"key": "pickColorFrom", "value": "artwork"})).unwrap();
    assert_eq!(s.tool_id(), "selection", "the held press lent the selection tool");
    assert_eq!(s.lender.as_ref().map(|t| t.id()), Some("shapeBuilder"));
    assert_eq!(out["pickColorFrom"], json!("artwork"));
    assert_eq!(s.lender.as_ref().map(|t| t.options()["pickColorFrom"].clone()), Some(json!("artwork")));
    // Named while it waits, the Shape Builder is read where it waits, not made afresh from the
    // preferences.
    s.lender.as_mut().unwrap().set_option("pickColorFrom", &json!("swatches"));
    let read = s.set_tool_option_cmd(&json!({"tool": "shapeBuilder"})).unwrap();
    assert_eq!(read["pickColorFrom"], json!("swatches"), "read from the waiting tool");
    // A value set on it counts as set on the tool chosen.
    s.tool_set_since_reset = false;
    s.set_tool_option_cmd(&json!({"tool": "shapeBuilder", "key": "highlightFill", "value": false})).unwrap();
    assert!(s.tool_set_since_reset(), "the tool chosen was set");
    assert_eq!(s.lender.as_ref().map(|t| t.options()["highlightFill"].clone()), Some(json!(false)));
}

/// A new revision of the document made outside a command while a merge is computed on another
/// thread keeps the merge when the selected art is as it was (the next poll applies it, as the
/// command computes it), and drops it when the art changed.
#[test]
fn shape_builder_keeps_a_merge_over_a_new_revision_of_the_same_art() {
    use vectorcraft_tools::builder::merges_computed;
    let v = ViewInfo::default();
    let mut s = session();
    square_row(&mut s);
    click_row(&mut s, v);
    let computed = merges_computed();
    s.active_mut().unwrap().revision += 1;
    s.tool_poll(v, true).unwrap();
    assert!(!s.tool_editing());
    assert_eq!(merges_computed(), computed, "the merge computed on another thread");
    assert_eq!(s.journal.last().unwrap().0, "shapeBuilder.merge");
    let mut direct = session();
    square_row(&mut direct);
    direct.execute("shapeBuilder.merge", &json!({"points": [[15, 10]]})).unwrap();
    assert!(undated(&s) == undated(&direct), "the same document as the command gives");
    // The art changed: the merge is dropped.
    let mut s = session();
    let ids = square_row(&mut s);
    click_row(&mut s, v);
    let journal = s.journal.len();
    let st = s.active_mut().unwrap();
    Arc::make_mut(&mut st.doc).node_mut(ids[1]).unwrap().transform(Affine::translate((0.0, 50.0)), false);
    st.revision += 1;
    s.tool_poll(v, true).unwrap();
    assert!(!s.tool_editing());
    assert_eq!(s.journal.len(), journal, "nothing merged");
    assert_eq!(layer_children(&s), 40);
}

/// A merge computed for the tool Cmd lent the selection tool's drag away from is applied before
/// a command too.
#[test]
fn shape_builder_merge_of_the_lending_tool_comes_first() {
    let v = ViewInfo::default();
    let mut s = session();
    square_row(&mut s);
    click_row(&mut s, v);
    let lent = s.make_tool("selection");
    s.lender = Some(std::mem::replace(&mut s.tool, lent));
    assert!(s.tool_editing());
    let journal = s.journal.len();
    s.execute("object.move", &json!({"dx": 0, "dy": 100})).unwrap();
    assert!(!s.tool_editing());
    assert_eq!(s.journal.since(journal).first().map(|j| j.0.as_str()), Some("shapeBuilder.merge"));
    assert_eq!(s.tool_id(), "selection", "the lent tool stays active until the release");
    assert_eq!(s.lender.as_ref().map(|t| t.id()), Some("shapeBuilder"));
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

/// A line from (100, 200) to (300, 200) and one from (200, 100) to (200, 300), with an unfilled
/// square from (100, 100) to (300, 300) in front of them, selected; the top row merged, which puts
/// the merged shape in front of the lines with its bottom edge along the first line. Returns the
/// first line and the merged shape.
fn lines_under_a_merged_row(s: &mut Session) -> (NodeId, NodeId) {
    let h = line(s, (100.0, 200.0), (300.0, 200.0));
    let v = line(s, (200.0, 100.0), (200.0, 300.0));
    let sq = rect(s, 100.0, 100.0, 200.0, 200.0);
    s.edit("t", |d, _| {
        for id in [h, v, sq] {
            d.node_mut(id).unwrap().appearance.set_fill(Paint::None);
        }
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [h.0, v.0, sq.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[150, 150], [250, 150]]})).unwrap();
    (h, NodeId(r["merged"].as_u64().unwrap()))
}

/// The left half of `h` (from x = 100 to 200) is gone.
fn left_half_gone(s: &Session, h: NodeId) -> bool {
    s.doc().unwrap().doc.node(h).is_none_or(|n| outline(n).0.bounds().is_none_or(|b| b.x0 > 199.0))
}

#[test]
fn shape_builder_drag_from_a_merged_shape_over_a_line_under_its_edge_makes_one_region() {
    let mut s = session();
    let (h, m1) = lines_under_a_merged_row(&mut s);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[250, 150], [150, 250]]})).unwrap();
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert!(s.doc().unwrap().doc.node(m1).is_none(), "the merged row is part of the new shape");
    assert!((area(&s, merged) - 30000.0).abs() < 1.0);
    assert!(left_half_gone(&s, h), "the line's stretch between the merged regions is removed");
    let r = s.execute("shapeBuilder.regions", &json!({})).unwrap();
    let ours = r["regions"].as_array().unwrap().iter().filter(|x| x["sources"] == json!([merged.0])).count();
    assert_eq!(ours, 1, "{r}");

    // Erasing from the merged row into the cell below takes the line's stretch between them too.
    let mut s = session();
    let (h, _) = lines_under_a_merged_row(&mut s);
    s.execute("shapeBuilder.merge", &json!({"points": [[250, 150], [150, 250]], "erase": true})).unwrap();
    assert!(left_half_gone(&s, h));
}

#[test]
fn shape_builder_removes_the_stretch_of_every_line_under_a_merged_edge() {
    // Two lines along the same stretch, under the bottom edge of the merged top row: merging the
    // row with the cell below takes the stretch between them from both lines.
    for erase in [false, true] {
        let mut s = session();
        let h1 = line(&mut s, (100.0, 200.0), (300.0, 200.0));
        let h2 = line(&mut s, (100.0, 200.0), (300.0, 200.0));
        let v = line(&mut s, (200.0, 100.0), (200.0, 300.0));
        let sq = rect(&mut s, 100.0, 100.0, 200.0, 200.0);
        s.edit("t", |d, _| {
            for id in [h1, h2, v, sq] {
                d.node_mut(id).unwrap().appearance.set_fill(Paint::None);
            }
            Ok(())
        })
        .unwrap();
        s.execute("select.set", &json!({"ids": [h1.0, h2.0, v.0, sq.0]})).unwrap();
        s.execute("shapeBuilder.merge", &json!({"points": [[150, 150], [250, 150]]})).unwrap();
        s.execute("shapeBuilder.merge", &json!({"points": [[250, 150], [150, 250]], "erase": erase})).unwrap();
        let d = &s.doc().unwrap().doc;
        let left: Vec<Rect> = d
            .children(d.default_layer())
            .into_iter()
            .flatten()
            .filter(|n| !outline(n).0.is_closed())
            .filter_map(|n| n.geometric_bounds())
            .filter(|b| b.height() < 1e-6 && (b.y0 - 200.0).abs() < 1e-6 && b.x0 < 199.0)
            .collect();
        assert!(left.is_empty(), "erase {erase}: {left:?}");
    }
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

/// The fill rows of an open path's appearance (bottom first) for the open-path tests below, with
/// a 1 pt black stroke on top: a painted fill under an unpainted top fill, a painted fill that is
/// hidden, one with an opacity of 0, and a painted fill under a hidden painted top fill.
fn open_path_fill_stacks() -> [(&'static str, Vec<AppearanceItem>, bool); 4] {
    use vectorcraft_doc::{FillLayer, StrokeLayer};
    let fill = |p: Paint, visible: bool, opacity: f32| {
        let mut f = FillLayer::new(p);
        f.visible = visible;
        f.opacity = opacity;
        AppearanceItem::Fill(f)
    };
    let red = || Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let blue = || Paint::solid(Color::rgb(0.0, 0.0, 1.0));
    let stroke = || AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::BLACK), 1.0));
    [
        ("a painted fill under an unpainted top fill", vec![fill(red(), true, 1.0), fill(Paint::None, true, 1.0), stroke()], true),
        ("a hidden painted fill", vec![fill(red(), false, 1.0), stroke()], false),
        ("a painted fill with an opacity of 0", vec![fill(red(), true, 0.0), stroke()], false),
        ("a painted fill under a hidden painted top fill", vec![fill(red(), true, 1.0), fill(blue(), false, 1.0), stroke()], true),
    ]
}

/// Consider Open Filled Paths as Closed: an open path counts as filled when any of its fill rows
/// draws (shown, painted and above 0% opacity), whichever row is on top. The map, a merge inside
/// the area and what a merge keeps of the path follow the rows that draw; with the option off, none
/// of the paths closes.
#[test]
fn shape_builder_closes_open_paths_by_the_fill_rows_that_draw() {
    for (what, items, closes) in open_path_fill_stacks() {
        let mut s = session();
        let r = s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}]})).unwrap();
        let p = NodeId(r["id"].as_u64().unwrap());
        s.edit("t", |d, _| {
            d.node_mut(p).unwrap().appearance.items = items;
            Ok(())
        })
        .unwrap();
        s.execute("select.set", &json!({"ids": [p.0]})).unwrap();
        let r = s.execute("shapeBuilder.regions", &json!({})).unwrap();
        assert_eq!(
            (r["regions"].as_array().unwrap().len(), r["lines"].as_array().unwrap().len()),
            if closes { (1, 0) } else { (0, 1) },
            "{what}: {r}"
        );
        let r = s.execute("shapeBuilder.regions", &json!({"openFilledAsClosed": false})).unwrap();
        assert_eq!((r["regions"].as_array().unwrap().len(), r["lines"].as_array().unwrap().len()), (0, 1), "{what}, option off: {r}");
        let n = node(&s, p);
        for on in [true, false] {
            let inputs = vectorcraft_tools::builder::merge_inputs(&[&n], on);
            assert_eq!(inputs.iter().map(|i| i.fill_closes).collect::<Vec<_>>(), vec![closes && on], "{what}, option {on}");
        }
        let merged = s.execute("shapeBuilder.merge", &json!({"points": [[75, 25]]}));
        if closes {
            assert!((area(&s, NodeId(merged.unwrap()["merged"].as_u64().unwrap())) - 5000.0).abs() < 1e-6, "{what}");
        } else {
            assert!(merged.is_err(), "{what}: the open path encloses no region");
        }
    }
}

/// Clicking Stroke Splits the Path on the middle side of an open U cuts it along the outline its
/// drawn fill closes: with a fill row that draws, what is left runs on from the cut round the
/// implied closing edge (the top side, then the bottom side); without one, what is left is the
/// open path's two ends in path order (the bottom side, then the top side).
#[test]
fn shape_builder_stroke_split_closes_open_paths_by_the_fill_rows_that_draw() {
    for (what, items, closes) in open_path_fill_stacks() {
        let mut s = session();
        let r =
            s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}, {"x": 0, "y": 100}]})).unwrap();
        let p = NodeId(r["id"].as_u64().unwrap());
        s.edit("t", |d, _| {
            d.node_mut(p).unwrap().appearance.items = items;
            Ok(())
        })
        .unwrap();
        s.execute("select.set", &json!({"ids": [p.0]})).unwrap();
        let r = s.execute("shapeBuilder.merge", &json!({"points": [[101, 50]], "splitStroke": true})).unwrap();
        let out = ids(&r);
        assert_eq!(shapes_of(&s, &out), vec![(false, 4, 200.0), (false, 2, 100.0)], "{what}");
        let (rest, _) = outline(&node(&s, out[0]));
        let starts: Vec<(f64, f64)> = rest.subpaths.iter().map(|sp| (sp.anchors[0].p.x.round(), sp.anchors[0].p.y.round())).collect();
        let order = if closes { vec![(100.0, 100.0), (0.0, 0.0)] } else { vec![(0.0, 0.0), (100.0, 100.0)] };
        assert_eq!(starts, order, "{what}");
    }
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

/// The colors of the cursor swatch preview's chips, left to right, and which one is ringed.
fn swatch_chips(s: &mut Session, v: ViewInfo) -> Vec<([u8; 4], bool)> {
    let mut chips: Vec<(f64, [u8; 4], bool)> = s
        .overlays(v)
        .into_iter()
        .filter_map(|o| match o {
            vectorcraft_tools::Overlay::Swatch { p, color, selected } => Some((p.x, color, selected)),
            _ => None,
        })
        .collect();
    chips.sort_by(|a, b| a.0.total_cmp(&b.0));
    chips.into_iter().map(|(_, c, sel)| (c, sel)).collect()
}

/// A solid paint's color as the chips draw it.
fn chip_color(p: &Paint) -> [u8; 4] {
    match p {
        Paint::Solid { color, .. } => color.to_rgba8(1.0),
        _ => panic!("not a solid color: {p:?}"),
    }
}

/// Live Paint Bucket: Left and Right step the active paint through the document's color swatches
/// (the selected art keeps its paint), the Cursor Swatch Preview (on at first) shows the active
/// paint ringed between the swatches before and after it, and with it off the arrows still step.
#[test]
fn live_paint_bucket_arrow_keys_step_the_active_paint_through_the_swatches() {
    let mut s = session();
    let (a, _) = two(&mut s);
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    assert_eq!(s.tool_options_of("livePaintBucket")["cursorSwatchPreview"], json!(true));
    assert!(s.tool_claims_key(ToolKey::Right, v) && s.tool_claims_key(ToolKey::Left, v));
    assert!(!s.tool_claims_key(ToolKey::Up, v), "Up and Down keep their own use");
    let list: Vec<(String, Paint)> =
        vectorcraft_tools::builder::color_swatches(&s.doc().unwrap().doc).iter().map(|sw| (sw.name.clone(), sw.paint.clone())).collect();
    assert!(list.len() >= 4);
    let n = list.len();
    let stroke = s.paint.stroke.clone();
    s.execute("paint.setFill", &json!({"swatch": list[0].0, "ids": []})).unwrap();
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    assert_eq!(s.paint.fill, list[2].1);
    s.tool_key(ToolKey::Left, Mods::default(), v).unwrap();
    assert_eq!(s.paint.fill, list[1].1);
    assert_eq!(s.paint.stroke, stroke, "the stroke stays");
    assert_eq!(node(&s, a).appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), "A keeps its fill");
    s.pointer(&PointerEvent::new(PointerKind::Move, 300.0, 300.0), v).unwrap();
    assert_eq!(swatch_chips(&mut s, v), vec![(chip_color(&list[0].1), false), (chip_color(&list[1].1), true), (chip_color(&list[2].1), false)]);
    let alt = Mods { alt: true, ..Default::default() };
    s.pointer(&PointerEvent::new(PointerKind::Move, 300.0, 300.0).with_mods(alt), v).unwrap();
    assert!(swatch_chips(&mut s, v).is_empty(), "Alt samples with the eyedropper and shows no chips");
    s.pointer(&PointerEvent::new(PointerKind::Move, 300.0, 300.0), v).unwrap();
    // Past the first swatch, Left wraps around to the last.
    s.tool_key(ToolKey::Left, Mods::default(), v).unwrap();
    s.tool_key(ToolKey::Left, Mods::default(), v).unwrap();
    assert_eq!(s.paint.fill, list[n - 1].1);
    assert_eq!(swatch_chips(&mut s, v).iter().map(|c| c.0).collect::<Vec<_>>(), [&list[n - 2].1, &list[n - 1].1, &list[0].1].map(chip_color));
    // With the Stroke proxy in front, the arrows step the stroke and the chips show it.
    s.execute("paint.toggleActive", &json!({"fill": false})).unwrap();
    let fill = s.paint.fill.clone();
    s.execute("paint.setStroke", &json!({"swatch": list[1].0, "ids": []})).unwrap();
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    assert_eq!((&s.paint.fill, &s.paint.stroke), (&fill, &list[2].1));
    assert!(!s.fill_active, "the Stroke proxy stays in front");
    assert_eq!(swatch_chips(&mut s, v)[1], (chip_color(&list[2].1), true));
    assert_eq!(node(&s, a).appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), "A keeps its paint");
    // Cursor Swatch Preview off hides the chips; the arrows still step.
    s.set_tool_option_cmd(&json!({"tool": "livePaintBucket", "values": {"cursorSwatchPreview": false}})).unwrap();
    assert!(swatch_chips(&mut s, v).is_empty());
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    assert_eq!(s.paint.stroke, list[3].1);
}

/// The Live Paint Bucket and the Shape Builder take Left and Right for swatches only without
/// modifiers: Shift, Alt and Cmd/Ctrl arrows are left to their shortcuts and change no paint.
#[test]
fn swatch_arrow_keys_leave_modified_arrows_to_the_shortcuts() {
    let mut s = session();
    two(&mut s);
    let v = ViewInfo::default();
    let first = vectorcraft_tools::builder::color_swatches(&s.doc().unwrap().doc)[0].name.clone();
    s.execute("paint.setFill", &json!({"swatch": first, "ids": []})).unwrap();
    for tool in ["livePaintBucket", "shapeBuilder"] {
        s.select_tool(tool, v).unwrap();
        if tool == "shapeBuilder" {
            s.set_tool_option_cmd(&json!({"tool": tool, "values": {"pickColorFrom": "swatches"}})).unwrap();
        }
        let fill = s.paint.fill.clone();
        assert!(s.tool_claims_key_with(ToolKey::Right, Mods::default(), v), "{tool}");
        for m in [Mods { shift: true, ..Default::default() }, Mods { alt: true, ..Default::default() }, Mods { cmd: true, ..Default::default() }] {
            for key in [ToolKey::Left, ToolKey::Right] {
                assert!(!s.tool_claims_key_with(key, m, v), "{tool} {key:?} {m:?}");
                s.tool_key(key, m, v).unwrap();
                assert_eq!(s.paint.fill, fill, "{tool} {key:?} {m:?}");
            }
        }
        s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
        assert_ne!(s.paint.fill, fill, "{tool}: a plain Right steps");
    }
}

/// Two swatches of the same color: the Bucket's arrows step through both, and the fill and the
/// stroke each step on from the swatch picked for them.
#[test]
fn live_paint_bucket_arrow_keys_pass_swatches_of_the_same_color_for_fill_and_stroke() {
    let mut s = session();
    two(&mut s);
    for name in ["Twin A", "Twin B"] {
        s.execute("swatch.new", &json!({"name": name, "color": "#123456"})).unwrap();
    }
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    let names: Vec<String> = vectorcraft_tools::builder::color_swatches(&s.doc().unwrap().doc).iter().map(|sw| sw.name.clone()).collect();
    let a = names.iter().position(|n| n == "Twin A").unwrap();
    assert_eq!(names[a + 1], "Twin B");
    let after = vectorcraft_tools::builder::color_swatches(&s.doc().unwrap().doc)[(a + 2) % names.len()].paint.clone();
    // The fill goes to Twin B.
    s.execute("paint.setFill", &json!({"swatch": "Twin A", "ids": []})).unwrap();
    let twin = s.paint.fill.clone();
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    assert_eq!(s.paint.fill, twin, "Twin B");
    // The stroke starts at Twin A: Right goes to Twin B, then past it.
    s.execute("paint.setStroke", &json!({"swatch": "Twin A", "ids": []})).unwrap();
    assert!(!s.fill_active);
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    assert_eq!(s.paint.stroke, twin, "Twin B for the stroke");
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    assert_eq!(s.paint.stroke, after, "the swatch after Twin B");
    // Back on the fill, which is still Twin B: Right goes past it.
    s.execute("paint.toggleActive", &json!({"fill": true})).unwrap();
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    assert_eq!(s.paint.fill, after, "the fill steps on from Twin B");
}

/// A color that is no swatch shows alone and Right picks the first swatch, Left the last; a paint
/// that is not a solid color shows no chip; one swatch steps to itself; with no color swatch the
/// tool leaves the arrows alone; during a fill drag the arrows change nothing.
#[test]
fn live_paint_bucket_arrow_keys_handle_unmatched_paint_few_swatches_and_drags() {
    let mut s = session();
    live(&mut s);
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Move, 300.0, 300.0), v).unwrap();
    let list: Vec<Paint> = vectorcraft_tools::builder::color_swatches(&s.doc().unwrap().doc).iter().map(|sw| sw.paint.clone()).collect();
    let odd = Paint::solid(Color::rgb8(0x12, 0x34, 0x57));
    assert!(!list.contains(&odd));
    s.paint.fill = odd.clone();
    assert_eq!(swatch_chips(&mut s, v), vec![(chip_color(&odd), true)]);
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    assert_eq!(s.paint.fill, list[0]);
    s.paint.fill = odd.clone();
    s.tool_key(ToolKey::Left, Mods::default(), v).unwrap();
    assert_eq!(s.paint.fill, list[list.len() - 1]);
    s.paint.fill = Paint::None;
    assert!(swatch_chips(&mut s, v).is_empty());

    // During a fill drag the arrows are not taken and change nothing.
    s.paint.fill = list[0].clone();
    s.pointer(&PointerEvent::new(PointerKind::Down, 25.0, 50.0), v).unwrap();
    assert!(s.tool_busy());
    assert!(!s.tool_claims_key(ToolKey::Right, v));
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    assert_eq!(s.paint.fill, list[0]);
    assert!(swatch_chips(&mut s, v).is_empty(), "no chips during a drag");
    s.pointer(&PointerEvent::new(PointerKind::Up, 25.0, 50.0), v).unwrap();

    // One color swatch: both arrows pick it, and it stands on both sides of itself.
    let one = list[0].clone();
    s.edit("t", |d, _| {
        d.swatch_groups.clear();
        d.swatches.retain(|sw| sw.paint == one);
        d.swatches.truncate(1);
        Ok(())
    })
    .unwrap();
    assert_eq!(vectorcraft_tools::builder::color_swatches(&s.doc().unwrap().doc).len(), 1);
    s.paint.fill = odd;
    s.tool_key(ToolKey::Left, Mods::default(), v).unwrap();
    assert_eq!(s.paint.fill, one);
    s.pointer(&PointerEvent::new(PointerKind::Move, 301.0, 300.0), v).unwrap();
    assert_eq!(swatch_chips(&mut s, v), vec![(chip_color(&one), false), (chip_color(&one), true), (chip_color(&one), false)]);
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    assert_eq!(s.paint.fill, one);

    // No color swatch: the arrows are left to the canvas and the paint stays.
    s.edit("t", |d, _| {
        d.swatches.clear();
        Ok(())
    })
    .unwrap();
    assert!(!s.tool_claims_key(ToolKey::Right, v) && !s.tool_claims_key(ToolKey::Left, v));
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    assert_eq!(s.paint.fill, one);
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

/// Make and Expand preserve the colors of compound paths sharing a curved boundary.
#[test]
fn live_paint_make_and_expand_keep_abutting_compound_colors() {
    use kurbo::Shape as _;
    use vectorcraft_geom::shapes;

    let mut s = session();
    let mut ring = shapes::ellipse(Rect::new(0.0, 0.0, 1.0, 1.0));
    let mut disk = shapes::ellipse(Rect::new(0.2, 0.2, 0.8, 0.8));
    let mut hole = disk.clone();
    hole.reverse();
    ring.subpaths.extend(hole.subpaths);
    disk.subpaths.extend(shapes::rectangle(Rect::new(1.0, 0.0, 1.3, 1.0)).subpaths);
    let compound = |s: &mut Session, path: &PathData, color: &str| {
        let id = s.execute("path.create", &json!({"d": path.to_bezpath().to_svg()})).unwrap()["id"].as_u64().unwrap();
        s.execute("paint.setFill", &json!({"id": id, "color": color})).unwrap();
        s.execute("paint.setStroke", &json!({"id": id, "none": true})).unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap())
    };
    let ink = compound(&mut s, &ring, "#000000");
    let blue = compound(&mut s, &disk, "#2878f0");
    s.execute("select.set", &json!({"ids": [ink.0, blue.0]})).unwrap();
    let history = s.doc().unwrap().history.undo.len();
    let made = s.execute("livePaint.make", &json!({})).unwrap();
    assert_eq!(made["faces"], 3);
    assert_eq!(s.doc().unwrap().history.undo.len(), history + 1);
    let group = NodeId(made["id"].as_u64().unwrap());
    let paint_at = |n: &Node, p: Point| {
        n.children()
            .unwrap()
            .iter()
            .filter(|c| c.visible && !c.appearance.fill_paint().is_none())
            .filter(|c| c.path_data().is_some_and(|path| path.to_bezpath().winding(p) != 0))
            .map(|c| c.appearance.fill_paint())
            .collect::<Vec<_>>()
    };
    let check_colors = |n: &Node| {
        assert_eq!(paint_at(n, Point::new(0.1, 0.5)), vec![Paint::solid(Color::BLACK)]);
        let blue = Paint::solid(Color::from_hex("#2878f0").unwrap());
        assert_eq!(paint_at(n, Point::new(0.5, 0.5)), vec![blue.clone()]);
        assert_eq!(paint_at(n, Point::new(1.15, 0.5)), vec![blue]);
    };
    check_colors(&node(&s, group));
    let info = s.execute("livePaint.info", &json!({"group": group.0})).unwrap();
    assert_eq!(info["sources"], 2);
    assert_eq!(info["faces"].as_array().unwrap().len(), 3);
    s.execute("livePaint.expand", &json!({})).unwrap();
    let expanded = node(&s, group);
    assert!(!vectorcraft_tools::builder::is_live_paint(&expanded));
    assert_eq!(expanded.children().unwrap().len(), 3);
    assert_eq!(s.doc().unwrap().history.undo.len(), history + 2);
    check_colors(&expanded);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(vectorcraft_tools::builder::is_live_paint(&node(&s, group)));
    check_colors(&node(&s, group));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(layer_children(&s), 2);
    assert!(matches!(node(&s, ink).kind, NodeKind::Compound { .. }));
    assert!(matches!(node(&s, blue).kind, NodeKind::Compound { .. }));
    s.execute("edit.redo", &json!({})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    check_colors(&node(&s, group));
}

/// Gives `id` the appearance rows `items` (paint order) and returns them.
fn set_rows(s: &mut Session, id: NodeId, items: Vec<AppearanceItem>) -> Vec<AppearanceItem> {
    s.edit("t", |d, _| {
        d.node_mut(id).unwrap().appearance = vectorcraft_doc::Appearance { items: items.clone(), ..Default::default() }.into();
        Ok(())
    })
    .unwrap();
    items
}

/// A fill row of `p`, changed by `f`.
fn fill_row(p: Paint, f: impl FnOnce(&mut FillLayer)) -> AppearanceItem {
    let mut l = FillLayer::new(p);
    f(&mut l);
    AppearanceItem::Fill(l)
}

/// A stroke row of `p` `width` wide, changed by `f`.
fn stroke_row(p: Paint, width: f64, f: impl FnOnce(&mut StrokeLayer)) -> AppearanceItem {
    let mut l = StrokeLayer::new(p, width);
    f(&mut l);
    AppearanceItem::Stroke(l)
}

/// The faces and edges of Live Paint group `g` whose bounds lie within x `x0`..`x1`.
fn parts_within(s: &Session, g: NodeId, x0: f64, x1: f64) -> (Vec<Node>, Vec<Node>) {
    let n = node(s, g);
    let within = |c: &&Arc<Node>| c.geometric_bounds().is_some_and(|b| b.x0 >= x0 - 1e-6 && b.x1 <= x1 + 1e-6);
    let faces = vectorcraft_tools::builder::faces(&n).into_iter().filter(within).map(|c| (**c).clone()).collect();
    let edges = vectorcraft_tools::builder::edges(&n).into_iter().filter(within).map(|c| (**c).clone()).collect();
    (faces, edges)
}

/// Make gives each edge every stroke row of its source that draws, a lower one under an
/// unpainted or hidden top row included, with the rows' settings and the source's opacity and
/// blending mode; rows that draw nothing stay only with the stored source, which Release returns.
#[test]
fn live_paint_make_keeps_lower_drawn_stroke_rows() {
    let mut s = session();
    let (a, b) = two_at(&mut s, 200.0);
    let black = Paint::solid(Color::BLACK);
    let green = Paint::solid(Color::rgb(0.0, 1.0, 0.0));
    let thick = stroke_row(black.clone(), 3.0, |l| {
        l.opacity = 0.5;
        l.blend = BlendMode::Multiply;
        l.cap = LineCap::Round;
    });
    // A: the top stroke has no paint.
    let a_rows =
        set_rows(&mut s, a, vec![fill_row(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), |_| {}), thick.clone(), stroke_row(Paint::None, 1.0, |_| {})]);
    // B: the top stroke is hidden; a zero-weight and a transparent stroke draw nothing either.
    set_rows(
        &mut s,
        b,
        vec![
            stroke_row(green.clone(), 2.0, |_| {}),
            stroke_row(black.clone(), 0.0, |_| {}),
            stroke_row(black, 2.0, |l| l.opacity = 0.0),
            stroke_row(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 5.0, |l| l.visible = false),
        ],
    );
    s.edit("t", |d, _| {
        let n = d.node_mut(a).unwrap();
        n.opacity = 0.75;
        n.blend = BlendMode::Screen;
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let (_, a_edges) = parts_within(&s, g, 0.0, 100.0);
    let (_, b_edges) = parts_within(&s, g, 200.0, 300.0);
    assert!(!a_edges.is_empty() && !b_edges.is_empty());
    for e in &a_edges {
        assert_eq!(e.appearance.items, vec![thick.clone()], "A's edge takes the stroke that draws");
        assert_eq!((e.opacity, e.blend), (0.75, BlendMode::Screen));
    }
    for e in &b_edges {
        assert_eq!(e.appearance.items, vec![stroke_row(green.clone(), 2.0, |_| {})], "B's edge takes only its green stroke");
        assert_eq!((e.opacity, e.blend), (1.0, BlendMode::Normal));
    }
    // Release returns the sources with every row they had.
    let released = ids(&s.execute("livePaint.release", &json!({})).unwrap());
    assert!(released.contains(&a));
    assert_eq!(node(&s, a).appearance.items, a_rows);
}

/// When Make cuts a dashed stroke with arrowheads, a width profile and a brush into edges, each
/// edge's dashes go on from where it starts along the path, the arrowheads stay at the path's own
/// ends, and the profile and brush, which spread over the whole path, are left off the pieces.
#[test]
fn live_paint_make_fits_dashes_and_arrowheads_to_each_edge() {
    use vectorcraft_doc::{Arrowhead, Dash};
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let l = line(&mut s, (-20.0, 50.0), (120.0, 50.0));
    let dash = Dash { pattern: vec![10.0, 5.0], offset: 2.0, align_corners: true };
    let row = stroke_row(Paint::solid(Color::BLACK), 2.0, |st| {
        st.dash = Some(dash.clone());
        st.start_arrow = Some(Arrowhead::Triangle);
        st.end_arrow = Some(Arrowhead::Circle);
        st.brush = Some("Charcoal".into());
    });
    let rows = set_rows(&mut s, l, vec![row]);
    s.execute("select.set", &json!({"ids": [a.0, l.0]})).unwrap();
    let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let gn = node(&s, g);
    let mut pieces: Vec<(f64, f64, vectorcraft_doc::StrokeLayer)> = vectorcraft_tools::builder::edges(&gn)
        .iter()
        .filter_map(|e| {
            let st = e.appearance.stroke().filter(|st| st.dash.is_some())?.clone();
            let sp = e.path_data()?.subpaths.first()?;
            Some((sp.anchors.first()?.p.x, sp.anchors.last()?.p.x, st))
        })
        .collect();
    pieces.sort_by(|p, q| p.0.total_cmp(&q.0));
    let ends: Vec<(f64, f64)> = pieces.iter().map(|p| ((p.0 * 1e3).round() / 1e3, (p.1 * 1e3).round() / 1e3)).collect();
    assert_eq!(ends, vec![(-20.0, 0.0), (0.0, 100.0), (100.0, 120.0)], "each piece runs the way the line does");
    let offsets: Vec<f64> = pieces.iter().map(|p| (p.2.dash.as_ref().unwrap().offset * 1e3).round() / 1e3).collect();
    assert_eq!(offsets, vec![2.0, 22.0, 122.0], "the dashes go on along the line");
    assert!(pieces.iter().all(|p| !p.2.dash.as_ref().unwrap().align_corners && p.2.brush.is_none()));
    let arrows: Vec<_> = pieces.iter().map(|p| (p.2.start_arrow.clone(), p.2.end_arrow.clone())).collect();
    assert_eq!(arrows, vec![(Some(Arrowhead::Triangle), None), (None, None), (None, Some(Arrowhead::Circle))]);
    // A path that crosses itself: the pieces that start at the crossing take their place on the
    // path from where they run, not from the first time the path passes that point.
    let mut s3 = session();
    let anchors = [(0.0, 0.0), (100.0, 100.0), (100.0, 0.0), (0.0, 100.0)].map(|(x, y)| json!({"x": x, "y": y}));
    let z = NodeId(s3.execute("path.create", &json!({"anchors": anchors, "closed": false})).unwrap()["id"].as_u64().unwrap());
    set_rows(&mut s3, z, rows.clone());
    s3.execute("select.set", &json!({"ids": [z.0]})).unwrap();
    let g3 = NodeId(s3.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let mut cut: Vec<(f64, Option<Arrowhead>, Option<Arrowhead>)> = vectorcraft_tools::builder::edges(&node(&s3, g3))
        .iter()
        .filter_map(|e| {
            let st = e.appearance.stroke()?;
            Some(((st.dash.as_ref()?.offset * 1e3).round() / 1e3, st.start_arrow.clone(), st.end_arrow.clone()))
        })
        .collect();
    cut.sort_by(|p, q| p.0.total_cmp(&q.0));
    let d = 2.0 + 50.0 * 2f64.sqrt();
    let r = |v: f64| (v * 1e3).round() / 1e3;
    assert_eq!(
        cut,
        vec![(2.0, Some(Arrowhead::Triangle), None), (r(d), None, None), (r(d + 100.0 + 100.0 * 2f64.sqrt()), None, Some(Arrowhead::Circle))]
    );
    // An uncut path keeps all of it.
    let mut s2 = session();
    let l2 = line(&mut s2, (0.0, 0.0), (50.0, 0.0));
    set_rows(&mut s2, l2, rows.clone());
    s2.execute("select.set", &json!({"ids": [l2.0]})).unwrap();
    let g2 = NodeId(s2.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let edges: Vec<_> = vectorcraft_tools::builder::edges(&node(&s2, g2)).iter().map(|e| e.appearance.items.clone()).collect();
    assert_eq!(edges, vec![rows]);
}

/// When Gap Options or Merge cuts an edge in two, each piece keeps the edge's dashes going on from
/// where it starts along the edge, an arrowhead only at the end of the edge it ends at, and no brush.
#[test]
fn live_paint_gap_options_and_merge_fit_an_edge_look_to_each_piece() {
    use vectorcraft_doc::{Arrowhead, Dash};
    let mut s = session();
    let l = line(&mut s, (0.0, 50.0), (100.0, 50.0));
    let stub = line(&mut s, (50.0, 0.0), (50.0, 48.0));
    let row = stroke_row(Paint::solid(Color::BLACK), 2.0, |st| {
        st.dash = Some(Dash { pattern: vec![10.0, 5.0], offset: 2.0, align_corners: false });
        st.start_arrow = Some(Arrowhead::Triangle);
        st.end_arrow = Some(Arrowhead::Circle);
        st.brush = Some("Charcoal".into());
    });
    let rows = set_rows(&mut s, l, vec![row]);
    s.execute("select.set", &json!({"ids": [l.0, stub.0]})).unwrap();
    let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let dashed = |s: &Session, g: NodeId| {
        let mut out: Vec<(f64, f64, vectorcraft_doc::StrokeLayer)> = vectorcraft_tools::builder::edges(&node(s, g))
            .iter()
            .filter_map(|e| {
                let st = e.appearance.stroke().filter(|st| st.dash.is_some())?.clone();
                let sp = e.path_data()?.subpaths.first()?;
                Some(((sp.anchors.first()?.p.x * 1e3).round() / 1e3, (sp.anchors.last()?.p.x * 1e3).round() / 1e3, st))
            })
            .collect();
        out.sort_by(|p, q| p.0.total_cmp(&q.0));
        out
    };
    let before = dashed(&s, g);
    assert_eq!(before.len(), 1, "nothing cuts the line yet");
    assert_eq!(vec![AppearanceItem::Stroke(before[0].2.clone())], rows);
    let r = s.execute("livePaint.gapOptions", &json!({"id": g.0, "closeGaps": true})).unwrap();
    assert_eq!(r["closed"], json!(1));
    let pieces = dashed(&s, g);
    let ends: Vec<(f64, f64)> = pieces.iter().map(|p| (p.0, p.1)).collect();
    assert_eq!(ends, vec![(0.0, 50.0), (50.0, 100.0)], "the line is cut where the gap closes");
    let offsets: Vec<f64> = pieces.iter().map(|p| (p.2.dash.as_ref().unwrap().offset * 1e3).round() / 1e3).collect();
    assert_eq!(offsets, vec![2.0, 52.0], "the dashes go on along the line");
    let arrows: Vec<_> = pieces.iter().map(|p| (p.2.start_arrow.clone(), p.2.end_arrow.clone())).collect();
    assert_eq!(arrows, vec![(Some(Arrowhead::Triangle), None), (None, Some(Arrowhead::Circle))]);
    assert!(pieces.iter().all(|p| p.2.brush.is_none()), "a brush spreads over the whole line");
    // Merge: the piece of the edge that an added path cuts off, which takes the edge's look, gets
    // it fitted to where it lies along the edge in the same way.
    let mut s = session();
    let l = line(&mut s, (0.0, 50.0), (100.0, 50.0));
    set_rows(&mut s, l, rows.clone());
    s.execute("select.set", &json!({"ids": [l.0]})).unwrap();
    let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let cut = line(&mut s, (25.0, 0.0), (25.0, 100.0));
    s.execute("livePaint.merge", &json!({"ids": [g.0, cut.0]})).unwrap();
    let pieces = dashed(&s, g);
    let ends: Vec<(f64, f64)> = pieces.iter().map(|p| (p.0, p.1)).collect();
    assert_eq!(ends, vec![(0.0, 25.0), (25.0, 100.0)], "the added line cuts the edge");
    let offsets: Vec<f64> = pieces.iter().map(|p| (p.2.dash.as_ref().unwrap().offset * 1e3).round() / 1e3).collect();
    assert_eq!(offsets, vec![2.0, 27.0], "the dashes go on along the line");
    let arrows: Vec<_> = pieces.iter().map(|p| (p.2.start_arrow.clone(), p.2.end_arrow.clone())).collect();
    assert_eq!(arrows, vec![(Some(Arrowhead::Triangle), None), (None, Some(Arrowhead::Circle))]);
    assert!(pieces.iter().all(|p| p.2.brush.is_none()), "a brush spreads over the whole line");
    // A repainted edge that Merge cuts keeps its paint on every piece.
    let mut s = session();
    let l = line(&mut s, (0.0, 50.0), (100.0, 50.0));
    s.execute("select.set", &json!({"ids": [l.0]})).unwrap();
    let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [50, 50], "color": "#ff0000", "width": 3})).unwrap();
    let cut = line(&mut s, (50.0, 0.0), (50.0, 100.0));
    s.execute("livePaint.merge", &json!({"ids": [g.0, cut.0]})).unwrap();
    let gn = node(&s, g);
    let red = Paint::solid(Color::from_hex("#ff0000").unwrap());
    let mut on_line: Vec<(f64, bool)> = vectorcraft_tools::builder::edges(&gn)
        .iter()
        .filter_map(|e| {
            let sp = e.path_data()?.subpaths.first()?;
            let (a, z) = (sp.anchors.first()?.p, sp.anchors.last()?.p);
            ((a.y - 50.0).abs() < 1e-6 && (z.y - 50.0).abs() < 1e-6)
                .then(|| ((a.x.min(z.x) * 1e3).round() / 1e3, e.appearance.stroke().is_some_and(|st| st.paint == red)))
        })
        .collect();
    on_line.sort_by(|p, q| p.0.total_cmp(&q.0));
    assert_eq!(on_line, vec![(0.0, true), (50.0, true)], "both pieces keep the edge's paint");
}

/// The edge Make builds from a closed path that nothing cuts starts at the anchor the path starts
/// at and runs the way the path does, whichever anchor that is, so its dashes and brush lie where
/// they do on the path, and it keeps the path's stroke row as it is.
#[test]
fn live_paint_make_starts_a_closed_edge_where_its_path_starts() {
    use vectorcraft_doc::Dash;
    let corners = [(50.0, 0.0), (100.0, 50.0), (50.0, 100.0), (0.0, 50.0)];
    for clockwise in [true, false] {
        for start in 0..4 {
            let mut s = session();
            let mut pts: Vec<(f64, f64)> = (0..4).map(|k| corners[(k + start) % 4]).collect();
            if !clockwise {
                pts.reverse();
            }
            let anchors: Vec<_> = pts.iter().map(|(x, y)| json!({"x": x, "y": y})).collect();
            let z = NodeId(s.execute("path.create", &json!({"anchors": anchors, "closed": true})).unwrap()["id"].as_u64().unwrap());
            let row = stroke_row(Paint::solid(Color::BLACK), 2.0, |st| {
                st.dash = Some(Dash { pattern: vec![10.0, 5.0], offset: 2.0, align_corners: false });
                st.brush = Some("Charcoal".into());
            });
            let rows = set_rows(&mut s, z, vec![row]);
            s.execute("select.set", &json!({"ids": [z.0]})).unwrap();
            let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
            let gn = node(&s, g);
            let edges = vectorcraft_tools::builder::edges(&gn);
            let [e] = edges.as_slice() else { panic!("one edge, got {}", edges.len()) };
            let sp = e.path_data().unwrap().subpaths.first().unwrap().clone();
            let at: Vec<(f64, f64)> = sp.anchors.iter().map(|a| ((a.p.x * 1e3).round() / 1e3, (a.p.y * 1e3).round() / 1e3)).collect();
            assert!(sp.closed);
            assert_eq!(at, pts, "clockwise {clockwise}, start {start}");
            assert_eq!(e.appearance.items, rows, "clockwise {clockwise}, start {start}");
        }
    }
    // A closed path that crosses itself is cut into loops, and the loop that holds the path's
    // first anchor is still only part of the path: it keeps no brush.
    let mut s = session();
    let anchors = [(0.0, 0.0), (100.0, 100.0), (100.0, 0.0), (0.0, 100.0)].map(|(x, y)| json!({"x": x, "y": y}));
    let z = NodeId(s.execute("path.create", &json!({"anchors": anchors, "closed": true})).unwrap()["id"].as_u64().unwrap());
    let row = stroke_row(Paint::solid(Color::BLACK), 2.0, |st| {
        st.dash = Some(Dash { pattern: vec![10.0, 5.0], offset: 2.0, align_corners: false });
        st.brush = Some("Charcoal".into());
    });
    set_rows(&mut s, z, vec![row]);
    s.execute("select.set", &json!({"ids": [z.0]})).unwrap();
    let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let gn = node(&s, g);
    let edges = vectorcraft_tools::builder::edges(&gn);
    assert!(edges.len() > 1, "the crossing cuts the path, got {} edge", edges.len());
    assert!(edges.iter().all(|e| e.appearance.stroke().is_some_and(|st| st.brush.is_none())), "no loop is the whole path");
}

/// When Make cuts a stroke whose only setting that depends on direction is a gradient along or
/// across it, each edge runs the way its source path does, so the gradient keeps its direction.
#[test]
fn live_paint_make_keeps_the_direction_of_gradient_strokes() {
    use vectorcraft_doc::StrokeGradientMode;
    for mode in [StrokeGradientMode::Along, StrokeGradientMode::Across] {
        for (x0, x1) in [(-20.0, 120.0), (120.0, -20.0)] {
            let mut s = session();
            let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
            let l = line(&mut s, (x0, 50.0), (x1, 50.0));
            let gradient = Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::default())));
            set_rows(&mut s, l, vec![stroke_row(gradient, 2.0, |st| st.gradient_mode = mode)]);
            s.execute("select.set", &json!({"ids": [a.0, l.0]})).unwrap();
            let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
            let pieces: Vec<(f64, f64)> = vectorcraft_tools::builder::edges(&node(&s, g))
                .iter()
                .filter_map(|e| {
                    e.appearance.stroke().filter(|st| st.gradient_mode == mode)?;
                    let sp = e.path_data()?.subpaths.first()?;
                    Some((sp.anchors.first()?.p.x, sp.anchors.last()?.p.x))
                })
                .collect();
            assert_eq!(pieces.len(), 3, "{mode:?}, from {x0}");
            assert!(
                pieces.iter().all(|&(p, q)| (q - p).signum() == (x1 - x0).signum()),
                "{mode:?}, from {x0}: each piece runs the way the line does: {pieces:?}"
            );
        }
    }
}

/// Make gives each face every fill row of its source that draws, in order and with their
/// opacity, blending mode, overprint and effects, and the source's gradients keep their fit on
/// the source; Expand keeps the rows, and Undo and Redo bring the group back with them.
#[test]
fn live_paint_make_keeps_all_drawn_fill_rows_and_attributes() {
    let mut s = session();
    let (a, b) = two(&mut s);
    let lower = fill_row(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), |l| {
        l.opacity = 0.4;
        l.blend = BlendMode::Multiply;
        l.overprint = true;
        l.effects.push(Effect { id: "distort.roughen".into(), params: Value::Null, visible: true });
    });
    // A: two fills that draw, with a hidden one between them and an unpainted one above them.
    set_rows(
        &mut s,
        a,
        vec![
            lower.clone(),
            fill_row(Paint::solid(Color::rgb(0.0, 1.0, 0.0)), |l| l.visible = false),
            fill_row(Paint::None, |_| {}),
            stroke_row(Paint::None, 1.0, |_| {}),
        ],
    );
    // The top fill (and the stroke) become a gradient fitted to A's bounds.
    let (want_fill, _) = paint_with_gradients(&mut s, a);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let want = vec![lower, fill_row(want_fill, |_| {})];
    let rows_within = |s: &Session, x0: f64, x1: f64| parts_within(s, g, x0, x1).0.iter().map(|f| f.appearance.items.clone()).collect::<Vec<_>>();
    assert_eq!(rows_within(&s, 0.0, 50.0), vec![want.clone()], "A's face takes both fills that draw, the gradient at its fit on A");
    let blue = vec![fill_row(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), |_| {})];
    assert_eq!(rows_within(&s, 50.0, 100.0), vec![blue], "B is in front");
    s.execute("livePaint.expand", &json!({})).unwrap();
    let expanded = node(&s, g);
    assert!(!vectorcraft_tools::builder::is_live_paint(&expanded));
    let kept =
        expanded.children().unwrap().iter().filter(|c| c.geometric_bounds().is_some_and(|b| b.x1 <= 50.0 + 1e-6)).any(|c| c.appearance.items == want);
    assert!(kept, "Expand keeps both fills");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(rows_within(&s, 0.0, 50.0), vec![want.clone()]);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(rows_within(&s, 0.0, 50.0), vec![want]);
}

/// A face takes the fill rows of the front-most covering source whose fill draws: a source in
/// front whose fill is hidden or transparent, or whose opacity is 0, does not hide the paint of the
/// source behind it, and a face that no such source covers has no paint.
#[test]
fn live_paint_make_uses_frontmost_source_with_drawn_fill() {
    for front in ["hidden", "clear", "transparent object"] {
        let mut s = session();
        let (a, b) = two(&mut s);
        s.edit("t", |d, _| {
            let n = d.node_mut(b).unwrap();
            let mut app = (*n.appearance).clone();
            match front {
                "hidden" => app.fill_mut().unwrap().visible = false,
                "clear" => app.fill_mut().unwrap().opacity = 0.0,
                _ => n.opacity = 0.0,
            }
            n.appearance = app.into();
            Ok(())
        })
        .unwrap();
        s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
        let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
        let rows_within = |x0: f64, x1: f64| parts_within(&s, g, x0, x1).0.iter().map(|f| f.appearance.items.clone()).collect::<Vec<_>>();
        let red = vec![fill_row(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), |_| {})];
        assert_eq!(rows_within(50.0, 100.0), vec![red], "{front}: the overlap shows A");
        assert_eq!(rows_within(100.0, 150.0), vec![vec![fill_row(Paint::None, |_| {})]], "{front}: B alone paints nothing");
        assert_eq!(parts_within(&s, g, 100.0, 150.0).0[0].opacity, 1.0, "{front}");
    }
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

/// The undo and redo depths, the unsaved state and the revision of the active document.
fn history_state(s: &Session) -> (usize, usize, bool, u64) {
    let st = s.doc().unwrap();
    (st.history.undo.len(), st.history.redo.len(), st.is_dirty(), st.revision)
}

/// Draw a rectangle and undo it, so a redo step is waiting, then mark the document saved with `g`
/// alone selected.
fn redo_waiting_and_saved(s: &mut Session, g: NodeId) {
    rect(s, 300.0, 300.0, 10.0, 10.0);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    s.doc_mut().unwrap().mark_saved();
}

/// The active document and its [`history_state`].
type Snapshot = (vectorcraft_doc::Document, (usize, usize, bool, u64));

fn snapshot(s: &Session) -> Snapshot {
    ((*s.doc().unwrap().doc).clone(), history_state(s))
}

/// After a Live Paint edit that changed nothing: the document, the undo and redo steps, the saved
/// state and the revision are as `before`, and Redo still brings back the rectangle
/// [`redo_waiting_and_saved`] undid. Undoes the rectangle again, marks the document saved and
/// takes a new `before`.
fn assert_unchanged(s: &mut Session, before: &mut Snapshot, what: &str) {
    assert!(*s.doc().unwrap().doc == before.0, "{what}: the document is as it was");
    assert_eq!(history_state(s), before.1, "{what}: no undo step, the redo step kept, still saved");
    let children = layer_children(s);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(layer_children(s), children + 1, "{what}: Redo brings back the rectangle");
    s.execute("edit.undo", &json!({})).unwrap();
    s.doc_mut().unwrap().mark_saved();
    *before = snapshot(s);
}

#[test]
fn live_paint_fill_same_paint_is_no_op() {
    let mut s = session();
    let g = live(&mut s);
    let drag = json!({"group": g.0, "point": [25, 50], "points": [[25, 50], [125, 50]], "color": "#00ff00"});
    s.execute("livePaint.fill", &drag).unwrap();
    redo_waiting_and_saved(&mut s, g);
    // A color used since stays the newest recent color and the last color.
    let blue = Color::rgb(0.0, 0.0, 1.0);
    s.remember_paint_now(&Paint::solid(blue));
    let recent = (s.recent_colors.clone(), s.last_solid);
    let mut before = snapshot(&s);
    let click = s.execute("livePaint.fill", &json!({"group": g.0, "point": [75, 50], "color": "#00ff00"})).unwrap();
    let face = vectorcraft_tools::builder::face_at(&node(&s, g), Point::new(75.0, 50.0)).unwrap().id;
    assert_eq!(click, json!({"group": g.0, "face": face.0, "count": 1}), "the usual reply");
    assert_unchanged(&mut s, &mut before, "a click on a face with the paint");
    s.execute("livePaint.fill", &drag).unwrap();
    assert_unchanged(&mut s, &mut before, "a drag over faces with the paint");
    // Without a group, the click finds the Live Paint group among the selected objects.
    s.execute("livePaint.fill", &json!({"point": [125, 50], "color": "#00ff00"})).unwrap();
    assert_unchanged(&mut s, &mut before, "a click in the selected group");
    assert_eq!((s.recent_colors.clone(), s.last_solid), recent, "the recent colors are as they were");
    // A drag that reaches one face without the paint is one undo step and remembers the color.
    let red = Paint::solid(Color::from_hex("#ff0000").unwrap());
    for x in [25, 75] {
        s.execute("livePaint.fill", &json!({"group": g.0, "point": [x, 50], "color": "#ff0000"})).unwrap();
    }
    s.remember_paint_now(&Paint::solid(blue));
    let depth = s.doc().unwrap().history.undo.len();
    s.execute("livePaint.fill", &json!({"group": g.0, "point": [25, 50], "points": [[25, 50], [125, 50]], "color": "#ff0000"})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    assert_eq!(Paint::solid(s.last_solid), red);
    let gn = node(&s, g);
    for x in [25.0, 75.0, 125.0] {
        assert_eq!(vectorcraft_tools::builder::face_at(&gn, Point::new(x, 50.0)).unwrap().appearance.fill_paint(), red, "{x}");
    }
}

#[test]
fn live_paint_bucket_click_with_the_face_paint_is_no_op() {
    let mut s = session();
    let g = live(&mut s);
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.fill = Paint::solid(Color::rgb(0.0, 1.0, 1.0));
    let click = |s: &mut Session, x: f64| {
        s.pointer(&PointerEvent::new(PointerKind::Down, x, 50.0), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, x, 50.0), v).unwrap();
    };
    click(&mut s, 125.0);
    redo_waiting_and_saved(&mut s, g);
    let mut before = snapshot(&s);
    click(&mut s, 125.0);
    assert_unchanged(&mut s, &mut before, "a Bucket click on a face with the paint");
    // A drag over faces with the paint changes nothing either.
    click(&mut s, 25.0);
    click(&mut s, 75.0);
    redo_waiting_and_saved(&mut s, g);
    let mut before = snapshot(&s);
    s.pointer(&PointerEvent::new(PointerKind::Down, 25.0, 50.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 125.0, 50.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 125.0, 50.0), v).unwrap();
    assert_unchanged(&mut s, &mut before, "a Bucket drag over faces with the paint");
}

#[test]
fn live_paint_stroke_edge_same_paint_and_width_is_no_op() {
    let mut s = session();
    let g = live(&mut s);
    let call = json!({"group": g.0, "point": [100, 50], "color": "#ff0000", "width": 3});
    let first = s.execute("livePaint.strokeEdge", &call).unwrap();
    redo_waiting_and_saved(&mut s, g);
    s.remember_paint_now(&Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
    let last = s.last_solid;
    let mut before = snapshot(&s);
    assert_eq!(s.execute("livePaint.strokeEdge", &call).unwrap(), first, "the usual reply");
    assert_unchanged(&mut s, &mut before, "an edge with the paint and the width");
    assert_eq!(s.last_solid, last, "the last color is as it was");
    // Another width is an edit.
    s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [100, 50], "color": "#ff0000", "width": 2})).unwrap();
    assert_eq!(history_state(&s).0, before.1.0 + 1);
}

/// With `scope`, faces and edges that already have the paint (and the width) are skipped: when
/// every face or edge the call reaches has it, nothing changes, and when some do, only the others
/// are painted, in one undo step. `count` is every face or edge reached.
#[test]
fn live_paint_scoped_fill_and_stroke_skip_parts_with_the_paint() {
    let mut s = session();
    let g = live(&mut s);
    s.execute("livePaint.fill", &json!({"group": g.0, "point": [25, 50], "points": [[25, 50], [125, 50]], "color": "#00ff00"})).unwrap();
    let edges =
        s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [100, 50], "scope": "samePaint", "color": "#ff0000", "width": 3})).unwrap();
    redo_waiting_and_saved(&mut s, g);
    let mut before = snapshot(&s);
    let same = json!({"group": g.0, "point": [75, 50], "scope": "samePaint", "color": "#00ff00"});
    assert_eq!(s.execute("livePaint.fill", &same).unwrap()["count"], 3);
    assert_unchanged(&mut s, &mut before, "a samePaint fill where every face has the paint");
    let again = json!({"group": g.0, "point": [100, 50], "scope": "samePaint", "color": "#ff0000", "width": 3});
    assert_eq!(s.execute("livePaint.strokeEdge", &again).unwrap()["count"], edges["count"]);
    assert_unchanged(&mut s, &mut before, "a samePaint stroke where every edge has the paint and the width");
    // One face of the three without the paint: only it is filled, in one undo step.
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let red = Paint::solid(Color::from_hex("#ff0000").unwrap());
    s.execute("livePaint.fill", &json!({"group": g.0, "point": [25, 50], "color": "#ff0000"})).unwrap();
    let depth = history_state(&s).0;
    let like = serde_json::to_value(&green).unwrap();
    let r = s.execute("livePaint.fill", &json!({"group": g.0, "point": [25, 50], "scope": "samePaint", "match": like, "color": "#00ff00"})).unwrap();
    assert_eq!(r["count"], 3);
    assert_eq!(history_state(&s).0, depth + 1);
    let fill_at = |s: &Session, x: f64| vectorcraft_tools::builder::face_at(&node(s, g), Point::new(x, 50.0)).unwrap().appearance.fill_paint();
    for x in [25.0, 75.0, 125.0] {
        assert_eq!(fill_at(&s, x), green, "{x}");
    }
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(fill_at(&s, 25.0), red);
    assert_eq!((fill_at(&s, 75.0), fill_at(&s, 125.0)), (green.clone(), green));
}

#[test]
fn live_paint_gap_options_same_values_are_no_op() {
    let mut s = session();
    let g = live_square_with_a_gap(&mut s);
    let set = json!({"detect": true, "size": "small", "color": "#123456"});
    let first = s.execute("livePaint.gapOptions", &set).unwrap();
    redo_waiting_and_saved(&mut s, g);
    let mut before = snapshot(&s);
    assert_eq!(s.execute("livePaint.gapOptions", &set).unwrap(), first, "the usual reply");
    assert_unchanged(&mut s, &mut before, "the options the group has");
    s.execute("livePaint.gapOptions", &json!({})).unwrap();
    assert_unchanged(&mut s, &mut before, "no options given");
    // Close Gaps With Paths with a gap to close is an edit.
    let r = s.execute("livePaint.gapOptions", &json!({"closeGaps": true})).unwrap();
    assert_eq!(r["closed"], 1);
    assert_eq!(history_state(&s).0, before.1.0 + 1);
}

#[test]
fn live_paint_gap_options_close_without_gaps_is_no_op() {
    let mut s = session();
    let g = live(&mut s);
    redo_waiting_and_saved(&mut s, g);
    let mut before = snapshot(&s);
    let r = s.execute("livePaint.gapOptions", &json!({"id": g.0, "closeGaps": true})).unwrap();
    assert_eq!((ids(&r), &r["faces"], &r["closed"], gaps_of(&r).len()), (vec![g], &json!(3), &json!(0), 0));
    assert_unchanged(&mut s, &mut before, "no gap to close");
    // With other options, the faces keep their ids when no path is added.
    let faces = |s: &Session| vectorcraft_tools::builder::faces(&node(s, g)).iter().map(|f| f.id).collect::<Vec<_>>();
    let ids_before = faces(&s);
    s.execute("livePaint.gapOptions", &json!({"id": g.0, "closeGaps": true, "color": "#00ff00"})).unwrap();
    assert_eq!(history_state(&s).0, before.1.0 + 1);
    assert_eq!(faces(&s), ids_before);
}

#[test]
fn live_paint_make_existing_group_is_no_op() {
    let mut s = session();
    let g = live(&mut s);
    redo_waiting_and_saved(&mut s, g);
    let mut before = snapshot(&s);
    let r = s.execute("livePaint.make", &json!({})).unwrap();
    assert_eq!((NodeId(r["id"].as_u64().unwrap()), &r["faces"]), (g, &json!(3)));
    assert_eq!(r["edges"].as_u64().unwrap(), vectorcraft_tools::builder::edges(&node(&s, g)).len() as u64);
    assert_unchanged(&mut s, &mut before, "Make with the group alone");
}

#[test]
fn live_paint_merge_single_group_is_no_op() {
    let mut s = session();
    let g = live(&mut s);
    let other = rect(&mut s, 300.0, 0.0, 50.0, 50.0);
    redo_waiting_and_saved(&mut s, g);
    // Given by id, the group is selected without an undo step.
    s.execute("select.set", &json!({"ids": [other.0]})).unwrap();
    let before = snapshot(&s);
    let r = s.execute("livePaint.merge", &json!({"ids": [g.0]})).unwrap();
    assert_eq!((NodeId(r["id"].as_u64().unwrap()), &r["faces"]), (g, &json!(3)));
    assert_eq!(s.doc().unwrap().selection.objects, vec![g]);
    assert!(*s.doc().unwrap().doc == before.0);
    let (undo, redo, dirty, _) = history_state(&s);
    assert_eq!((undo, redo, dirty), (before.1.0, before.1.1, before.1.2));
    // A path to add is an edit.
    s.execute("livePaint.merge", &json!({"ids": [g.0, other.0]})).unwrap();
    assert_eq!(history_state(&s).0, before.1.0 + 1);
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

/// A Live Paint group of 50 pt squares side by side, 50 pt tall, with their left sides at each of
/// `xs`.
fn live_squares(s: &mut Session, xs: &[f64]) -> NodeId {
    let ids: Vec<u64> = xs.iter().map(|&x| rect(s, x, 0.0, 50.0, 50.0).0).collect();
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap())
}

/// The ids of the faces of `g` that hold `p`, in the order given.
fn faces_at(s: &Session, g: NodeId, ps: &[(f64, f64)]) -> Vec<NodeId> {
    ps.iter().map(|&(x, y)| vectorcraft_tools::builder::face_at(&node(s, g), Point::new(x, y)).unwrap().id).collect()
}

/// The edges of `g` whose bounds lie within `r`.
fn edges_in(s: &Session, g: NodeId, r: Rect) -> Vec<NodeId> {
    let r = r.inflate(1e-6, 1e-6);
    vectorcraft_tools::builder::edges(&node(s, g))
        .iter()
        .filter(|e| e.geometric_bounds().is_some_and(|bb| r.contains(Point::new(bb.x0, bb.y0)) && r.contains(Point::new(bb.x1, bb.y1))))
        .map(|e| e.id)
        .collect()
}

/// Give `ids` stroke paint `paint` at weight 1 (none: no stroke), and `fill` to the faces in `faces`.
fn paint_parts(s: &mut Session, ids: &[NodeId], paint: &Paint, faces: &[(NodeId, Paint)]) {
    s.edit("t", |d, _| {
        for id in ids {
            let n = d.node_mut(*id).unwrap();
            n.appearance.set_stroke(paint.clone());
            if let Some(st) = n.appearance.stroke_mut() {
                st.width = 1.0;
            }
        }
        for (id, fill) in faces {
            d.node_mut(*id).unwrap().appearance.set_fill(fill.clone());
        }
        Ok(())
    })
    .unwrap();
}

fn fill_of(s: &Session, id: NodeId) -> Paint {
    node(s, id).appearance.fill_paint()
}

/// A double-click on a face fills it and the faces reached across edges that draw no stroke; a
/// stroked edge and a hidden face stop the flood.
#[test]
fn live_paint_bucket_double_click_floods_across_unstroked_edges() {
    let mut s = session();
    let g = live_squares(&mut s, &[0.0, 50.0, 100.0, 150.0]);
    let [a, b, c, d] = faces_at(&s, g, &[(25.0, 25.0), (75.0, 25.0), (125.0, 25.0), (175.0, 25.0)])[..] else { panic!("four faces") };
    let all = edges_in(&s, g, Rect::new(-1.0, -1.0, 201.0, 51.0));
    paint_parts(&mut s, &all, &Paint::None, &[]);
    let stroked = edges_in(&s, g, Rect::new(150.0, 0.0, 150.0, 50.0));
    assert!(!stroked.is_empty());
    paint_parts(&mut s, &stroked, &Paint::solid(Color::rgb(0.0, 0.0, 0.0)), &[]);
    let white = fill_of(&s, d);
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    assert_ne!(white, green);
    let depth = s.doc().unwrap().history.undo.len();
    let r = s.execute("livePaint.fill", &json!({"group": g.0, "point": [25, 25], "color": "#00ff00", "scope": "connected"})).unwrap();
    assert_eq!((&r["face"], &r["count"]), (&json!(a.0), &json!(3)));
    assert_eq!([a, b, c, d].map(|f| fill_of(&s, f)), [green.clone(), green.clone(), green.clone(), white.clone()]);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1, "one undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!([a, b, c].iter().all(|f| fill_of(&s, *f) != green));
    // A hidden face is not filled and does not pass the flood on.
    s.execute("object.setProps", &json!({"ids": [b.0], "visible": false})).unwrap();
    let r = s.execute("livePaint.fill", &json!({"group": g.0, "point": [25, 25], "color": "#00ff00", "scope": "connected"})).unwrap();
    assert_eq!(r["count"], 1);
    assert_eq!([a, b, c].map(|f| fill_of(&s, f) == green), [true, false, false]);
    while s.doc().unwrap().history.undo.len() > depth {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    // The tool: two clicks and the double-click that follows them.
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.fill = green.clone();
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick] {
        s.pointer(&PointerEvent::new(kind, 125.0, 25.0), v).unwrap();
    }
    assert_eq!([a, b, c, d].map(|f| fill_of(&s, f)), [green.clone(), green.clone(), green, white]);
}

/// An edge that draws no stroke (fully transparent, or with its stroke hidden) lets a flood across
/// although it stores a stroke paint, and an edge scope compares the stroke paint each edge draws
/// with.
#[test]
fn live_paint_bucket_scopes_follow_the_stroke_an_edge_draws() {
    let mut s = session();
    let g = live_squares(&mut s, &[0.0, 50.0, 100.0]);
    let black = Paint::solid(Color::rgb(0.0, 0.0, 0.0));
    let all = edges_in(&s, g, Rect::new(-1.0, -1.0, 151.0, 51.0));
    paint_parts(&mut s, &all, &black, &[]);
    let clear = edges_in(&s, g, Rect::new(50.0, 0.0, 50.0, 50.0));
    let hidden = edges_in(&s, g, Rect::new(100.0, 0.0, 100.0, 50.0));
    assert!(!clear.is_empty() && !hidden.is_empty());
    s.edit("t", |d, _| {
        for id in &clear {
            d.node_mut(*id).unwrap().opacity = 0.0;
        }
        for id in &hidden {
            d.node_mut(*id).unwrap().appearance.stroke_mut().unwrap().visible = false;
        }
        Ok(())
    })
    .unwrap();
    let r = s.execute("livePaint.fill", &json!({"group": g.0, "point": [25, 25], "color": "#00ff00", "scope": "connected"})).unwrap();
    assert_eq!(r["count"], 3, "the flood crosses both middle edges");
    s.execute("edit.undo", &json!({})).unwrap();
    let stroke = |s: &mut Session, at: [f64; 2]| {
        let r = s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": at, "color": "#00ff00", "width": 3, "scope": "samePaint"})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        r["count"].clone()
    };
    assert_eq!(stroke(&mut s, [25.0, 0.0]), json!(all.len() - clear.len() - hidden.len()), "the edges that draw black");
    assert_eq!(stroke(&mut s, [50.0, 25.0]), json!(clear.len() + hidden.len()), "the edges that draw no stroke");
    // With the top edge between them fully transparent too, the connected scope walks from the edge
    // at x = 50 through it to the edge at x = 100, and reaches no edge that draws black.
    let top = edges_in(&s, g, Rect::new(50.0, 0.0, 100.0, 0.0));
    assert_eq!(top.len(), 1);
    s.edit("t", |d, _| {
        d.node_mut(top[0]).unwrap().opacity = 0.0;
        Ok(())
    })
    .unwrap();
    let r =
        s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [50, 25], "color": "#00ff00", "width": 3, "scope": "connected"})).unwrap();
    assert_eq!(r["count"], json!(clear.len() + top.len() + hidden.len()));
}

/// `livePaint.strokeEdge` leaves an edge one stroke, shown, with the paint, so the edge then draws
/// with it, whether its topmost shown stroke sat under a hidden one or every stroke was hidden.
/// Painting again changes nothing.
#[test]
fn live_paint_stroke_edge_paints_the_shown_stroke() {
    let mut s = session();
    let g = live_squares(&mut s, &[0.0, 50.0]);
    let black = Paint::solid(Color::rgb(0.0, 0.0, 0.0));
    let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let all = edges_in(&s, g, Rect::new(-1.0, -1.0, 101.0, 51.0));
    paint_parts(&mut s, &all, &black, &[]);
    let middle = edges_in(&s, g, Rect::new(50.0, 0.0, 50.0, 50.0));
    let left: Vec<NodeId> = edges_in(&s, g, Rect::new(0.0, 0.0, 50.0, 50.0)).into_iter().filter(|e| !middle.contains(e)).collect();
    let (&[m], &[l]) = (&middle[..], &left[..]) else { panic!("one middle edge and one other edge round the left face") };
    // The middle edge draws red under a hidden black stroke; every stroke of the left face's other
    // edge is hidden.
    s.edit("t", |d, _| {
        let n = d.node_mut(m).unwrap();
        n.appearance.items.insert(0, AppearanceItem::Stroke(vectorcraft_doc::appearance::StrokeLayer::new(red.clone(), 1.0)));
        n.appearance.stroke_mut().unwrap().visible = false;
        d.node_mut(l).unwrap().appearance.stroke_mut().unwrap().visible = false;
        Ok(())
    })
    .unwrap();
    let drawn = |s: &Session, id: NodeId| vectorcraft_tools::builder::lp_stroke_paint(&node(s, id));
    assert_eq!(drawn(&s, m), red);
    let paint = |s: &mut Session, at: [f64; 2]| {
        s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": at, "color": "#00ff00", "width": 2})).unwrap();
    };
    for (id, at) in [(m, [50.0, 25.0]), (l, [0.0, 25.0])] {
        paint(&mut s, at);
        assert_eq!(drawn(&s, id), green, "{id}");
        let depth = history_state(&s).0;
        paint(&mut s, at);
        assert_eq!(history_state(&s).0, depth, "painting {id} again");
    }
    assert_eq!(node(&s, m).appearance.items.len(), 1, "one stroke in place of the hidden and the shown one");
    assert_eq!(node(&s, l).appearance.items.len(), 1, "one shown stroke in place of the hidden one");
}

/// A face with two fills that draw, under a hidden top fill: the Bucket replaces every fill with
/// one shown, fully opaque fill of the paint, keeps the face's strokes, opacity and their order,
/// and a click with the paint the face then has records nothing. A fill of none leaves no fill
/// that draws. The route that makes the objects into a group first paints the same way.
#[test]
fn live_paint_bucket_replaces_every_fill_row() {
    use vectorcraft_doc::appearance::{FillLayer, StrokeLayer};
    let mut s = session();
    let g = live_squares(&mut s, &[0.0]);
    let [f] = faces_at(&s, g, &[(25.0, 25.0)])[..] else { panic!("one face") };
    let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let blue = Paint::solid(Color::rgb(0.0, 0.0, 1.0));
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let stroke = StrokeLayer::new(blue.clone(), 3.0);
    s.edit("t", |d, _| {
        let n = d.node_mut(f).unwrap();
        n.opacity = 0.5;
        let mut half = FillLayer::new(blue.clone());
        half.opacity = 0.5;
        let mut hidden = FillLayer::new(green.clone());
        hidden.visible = false;
        n.appearance.items = vec![
            AppearanceItem::Fill(FillLayer::new(red.clone())),
            AppearanceItem::Stroke(stroke.clone()),
            AppearanceItem::Fill(half),
            AppearanceItem::Fill(hidden),
        ];
        Ok(())
    })
    .unwrap();
    let depth = history_state(&s).0;
    s.execute("livePaint.fill", &json!({"group": g.0, "point": [25, 25], "color": "#00ff00"})).unwrap();
    assert_eq!(history_state(&s).0, depth + 1, "a hidden top fill with the paint does not stop the fill");
    let n = node(&s, f);
    assert_eq!(n.appearance.items, vec![AppearanceItem::Stroke(stroke.clone()), AppearanceItem::Fill(FillLayer::new(green.clone()))]);
    assert_eq!(n.opacity, 0.5, "the face keeps its own opacity");
    s.recent_colors.clear();
    s.execute("livePaint.fill", &json!({"group": g.0, "point": [25, 25], "color": "#00ff00"})).unwrap();
    assert_eq!((history_state(&s).0, s.recent_colors.len()), (depth + 1, 0), "painting again records nothing");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("livePaint.fill", &json!({"group": g.0, "point": [25, 25], "none": true})).unwrap();
    let n = node(&s, f);
    assert!(!n.appearance.has_drawn_fill(), "no fill draws after a fill of none");
    assert_eq!(n.appearance.items, vec![AppearanceItem::Stroke(stroke), AppearanceItem::Fill(FillLayer::new(Paint::None))]);

    // Objects made into a Live Paint group by the click: each object's two fills become one.
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 50.0, 50.0);
    s.edit("t", |d, _| {
        d.node_mut(id).unwrap().appearance.items.insert(0, AppearanceItem::Fill(FillLayer::new(red.clone())));
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let r = s.execute("livePaint.fill", &json!({"point": [25, 25], "color": "#00ff00"})).unwrap();
    let face = NodeId(r["face"].as_u64().unwrap());
    let fills: Vec<Paint> = node(&s, face).appearance.items.iter().filter(|i| i.is_fill()).map(|i| i.paint().clone()).collect();
    assert_eq!(fills, vec![green]);
}

/// An edge with two strokes that draw: `livePaint.strokeEdge` replaces them with one stroke of the
/// paint and width, keeping the topmost one's dashes; a stroke of none leaves no stroke that
/// draws, and `livePaint.info` then lists that one row.
#[test]
fn live_paint_stroke_edge_replaces_every_stroke_row() {
    use vectorcraft_doc::appearance::{Dash, StrokeLayer};
    let mut s = session();
    let g = live_squares(&mut s, &[0.0, 50.0]);
    let middle = edges_in(&s, g, Rect::new(50.0, 0.0, 50.0, 50.0));
    let [m] = middle[..] else { panic!("one middle edge") };
    let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let dash = Dash { pattern: vec![4.0, 2.0], offset: 0.0, align_corners: false };
    s.edit("t", |d, _| {
        let mut top = StrokeLayer::new(green.clone(), 2.0);
        top.opacity = 0.5;
        top.dash = Some(dash.clone());
        d.node_mut(m).unwrap().appearance.items = vec![AppearanceItem::Stroke(StrokeLayer::new(red.clone(), 6.0)), AppearanceItem::Stroke(top)];
        Ok(())
    })
    .unwrap();
    let depth = history_state(&s).0;
    s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [50, 25], "color": "#00ff00", "width": 2})).unwrap();
    assert_eq!(history_state(&s).0, depth + 1, "a half-transparent top stroke with the paint does not stop the stroke");
    let mut want = StrokeLayer::new(green.clone(), 2.0);
    want.dash = Some(dash);
    assert_eq!(node(&s, m).appearance.items, vec![AppearanceItem::Stroke(want)]);
    s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [50, 25], "color": "#00ff00", "width": 2})).unwrap();
    assert_eq!(history_state(&s).0, depth + 1, "painting again records nothing");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [50, 25], "none": true, "width": 2})).unwrap();
    assert!(!node(&s, m).appearance.has_drawn_stroke(), "no stroke draws after a stroke of none");
    let info = s.execute("livePaint.info", &json!({"group": g.0})).unwrap();
    let edge = info["edges"].as_array().unwrap().iter().find(|e| e["id"] == json!(m.0)).unwrap().clone();
    let rows = edge["appearance"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((&rows[0]["kind"], &rows[0]["drawn"], &edge["stroke"]), (&json!("stroke"), &json!(false), &json!(Paint::None.label())));
}

/// With Cmd held, the Live Paint Bucket's presses go to the selection tool Cmd lends, and the
/// double-click and triple-click that follow them paint nothing.
#[test]
fn live_paint_bucket_cmd_double_and_triple_click_paint_nothing() {
    let mut s = session();
    let g = live_squares(&mut s, &[0.0, 50.0, 100.0]);
    let faces = faces_at(&s, g, &[(25.0, 25.0), (75.0, 25.0), (125.0, 25.0)]);
    let all = edges_in(&s, g, Rect::new(-1.0, -1.0, 151.0, 51.0));
    paint_parts(&mut s, &all, &Paint::None, &[]);
    let before: Vec<Paint> = faces.iter().map(|f| fill_of(&s, *f)).collect();
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.fill = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let cmd = Mods { cmd: true, ..Default::default() };
    let (down, up) = (PointerKind::Down, PointerKind::Up);
    for kind in [down, up, down, up, PointerKind::DoubleClick, down, up, PointerKind::TripleClick] {
        s.pointer(&PointerEvent::new(kind, 75.0, 25.0).with_mods(cmd), v).unwrap();
    }
    assert_eq!(faces.iter().map(|f| fill_of(&s, *f)).collect::<Vec<_>>(), before);
    assert_eq!(s.tool_id(), "livePaintBucket");
}

/// The flood crosses curved edges, and reaches a face 0.012 pt wide.
#[test]
fn live_paint_bucket_double_click_floods_curved_and_narrow_faces() {
    let mut s = session();
    let ids: Vec<u64> = [0.0, 60.0]
        .iter()
        .map(|&x| s.execute("shape.ellipse", &json!({"x": x, "y": 0, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap())
        .collect();
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let all = edges_in(&s, g, Rect::new(-1.0, -1.0, 161.0, 101.0));
    paint_parts(&mut s, &all, &Paint::None, &[]);
    let r = s.execute("livePaint.fill", &json!({"group": g.0, "point": [20, 50], "color": "#00ff00", "scope": "connected"})).unwrap();
    assert_eq!(r["count"], 3, "both lunes and the lens between them");

    let mut s = session();
    let ids = [rect(&mut s, 0.0, 0.0, 50.0, 50.0).0, rect(&mut s, 49.988, 0.0, 50.0, 50.0).0];
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let [a, sliver, c] = faces_at(&s, g, &[(25.0, 25.0), (49.994, 25.0), (75.0, 25.0)])[..] else { panic!("three faces") };
    assert!(a != sliver && sliver != c);
    let all = edges_in(&s, g, Rect::new(-1.0, -1.0, 101.0, 51.0));
    paint_parts(&mut s, &all, &Paint::None, &[]);
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let r = s.execute("livePaint.fill", &json!({"group": g.0, "point": [25, 25], "color": "#00ff00", "scope": "connected"})).unwrap();
    assert_eq!(r["count"], 3);
    assert_eq!([a, sliver, c].map(|f| fill_of(&s, f)), [green.clone(), green.clone(), green]);
}

/// The flood crosses a hidden edge, and an edge that a Gap Detection gap meets partway along, but
/// not the gap.
#[test]
fn live_paint_bucket_double_click_floods_across_hidden_edges_and_past_gaps() {
    let mut s = session();
    let g = live_squares(&mut s, &[0.0, 50.0]);
    let black = Paint::solid(Color::rgb(0.0, 0.0, 0.0));
    let all = edges_in(&s, g, Rect::new(-1.0, -1.0, 101.0, 51.0));
    paint_parts(&mut s, &all, &black, &[]);
    let fill = |s: &mut Session, g: NodeId, at: [f64; 2]| {
        s.execute("livePaint.fill", &json!({"group": g.0, "point": at, "color": "#00ff00", "scope": "connected"})).unwrap()["count"].clone()
    };
    assert_eq!(fill(&mut s, g, [25.0, 25.0]), 1, "a stroked edge stops the flood");
    s.execute("edit.undo", &json!({})).unwrap();
    let middle = edges_in(&s, g, Rect::new(50.0, 0.0, 50.0, 50.0));
    s.execute("object.setProps", &json!({"ids": middle.iter().map(|e| e.0).collect::<Vec<_>>(), "visible": false})).unwrap();
    assert_eq!(fill(&mut s, g, [25.0, 25.0]), 2, "a hidden edge does not");

    // Square B (100..200 by 0..100) over square C (100..200 by 100..200), and a line down the middle
    // of B that stops 2 pt short of their shared edge: Gap Detection closes it, so B is two faces
    // that each run along part of the shared edge, which is one edge segment.
    let mut s = session();
    let mut ids = vec![rect(&mut s, 100.0, 0.0, 100.0, 100.0).0, rect(&mut s, 100.0, 100.0, 100.0, 100.0).0];
    let line = s.execute("path.create", &json!({"anchors": [{"x": 150, "y": 0}, {"x": 150, "y": 98}]})).unwrap();
    ids.push(line["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("livePaint.gapOptions", &json!({"id": g.0, "detect": true, "size": "small"})).unwrap();
    let [b1, b2, c] = faces_at(&s, g, &[(125.0, 50.0), (175.0, 50.0), (150.0, 150.0)])[..] else { panic!("three faces") };
    assert!(b1 != b2 && b2 != c && b1 != c);
    let all = edges_in(&s, g, Rect::new(99.0, -1.0, 201.0, 201.0));
    paint_parts(&mut s, &all, &black, &[]);
    let shared = edges_in(&s, g, Rect::new(100.0, 100.0, 200.0, 100.0));
    assert!(!shared.is_empty());
    paint_parts(&mut s, &shared, &Paint::None, &[]);
    assert_eq!(fill(&mut s, g, [150.0, 150.0]), 3, "from C across the shared edge to both faces of B");
    s.execute("edit.undo", &json!({})).unwrap();
    // With the line and the shared edge stroked, only the gap lies between the two faces of B.
    paint_parts(&mut s, &shared, &black, &[]);
    assert_eq!(fill(&mut s, g, [125.0, 50.0]), 1, "the flood does not pass the gap");

    // B alone, its outline drawing no stroke and the line stroked: the two faces of B run along
    // different parts of the outline's bottom segment, on the same side, and the flood does not
    // join them there.
    let mut s = session();
    let mut ids = vec![rect(&mut s, 100.0, 0.0, 100.0, 100.0).0];
    let line = s.execute("path.create", &json!({"anchors": [{"x": 150, "y": 0}, {"x": 150, "y": 98}]})).unwrap();
    ids.push(line["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("livePaint.gapOptions", &json!({"id": g.0, "detect": true, "size": "small"})).unwrap();
    assert_eq!(vectorcraft_tools::builder::faces(&node(&s, g)).len(), 2);
    let all = edges_in(&s, g, Rect::new(99.0, -1.0, 201.0, 101.0));
    paint_parts(&mut s, &all, &Paint::None, &[]);
    let line = edges_in(&s, g, Rect::new(150.0, 0.0, 150.0, 98.0));
    assert!(!line.is_empty() && line.len() < all.len());
    paint_parts(&mut s, &line, &black, &[]);
    assert_eq!(fill(&mut s, g, [125.0, 50.0]), 1, "the gap parts the two faces along the bottom");
}

/// The flood crosses an edge into each face along it when a face's outline runs along the edge in
/// one segment and the edge is split where another line meets it.
#[test]
fn live_paint_bucket_double_click_floods_along_edges_split_at_a_junction() {
    // A square with a line across its middle, and a line from the top at x = 30 that crosses it and
    // stops 20 pt below. The bottom face's outline runs along the middle line in one segment, from
    // x = 0 to 100; the middle line's edges are split at x = 30.
    let mut s = session();
    let mut ids = vec![rect(&mut s, 0.0, 0.0, 100.0, 100.0).0];
    for anchors in [json!([{"x": 0, "y": 50}, {"x": 100, "y": 50}]), json!([{"x": 30, "y": 0}, {"x": 30, "y": 70}])] {
        ids.push(s.execute("path.create", &json!({ "anchors": anchors })).unwrap()["id"].as_u64().unwrap());
    }
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let [left, right, bottom] = faces_at(&s, g, &[(15.0, 25.0), (65.0, 25.0), (65.0, 75.0)])[..] else { panic!("three faces") };
    assert!(left != right && right != bottom && left != bottom);
    let all = edges_in(&s, g, Rect::new(-1.0, -1.0, 101.0, 101.0));
    paint_parts(&mut s, &all, &Paint::None, &[]);
    let down = edges_in(&s, g, Rect::new(30.0, 0.0, 30.0, 70.0));
    assert!(!down.is_empty() && down.len() < all.len());
    paint_parts(&mut s, &down, &Paint::solid(Color::rgb(0.0, 0.0, 0.0)), &[]);
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let r = s.execute("livePaint.fill", &json!({"group": g.0, "point": [65, 75], "color": "#00ff00", "scope": "connected"})).unwrap();
    assert_eq!(r["count"], 3, "from the bottom face to both faces above the line");
    assert_eq!([left, right, bottom].map(|f| fill_of(&s, f)), [green.clone(), green.clone(), green.clone()]);
    s.execute("edit.undo", &json!({})).unwrap();
    let r = s.execute("livePaint.fill", &json!({"group": g.0, "point": [15, 25], "color": "#00ff00", "scope": "connected"})).unwrap();
    assert_eq!(r["count"], 3, "from the upper left face down to the bottom face and up again");
}

/// A triple-click on a face fills every face of the group with that face's fill, joined to it or
/// not. Through the tool, the fill compared is the face's fill before the first click.
#[test]
fn live_paint_bucket_triple_click_fills_every_face_of_the_same_fill() {
    let mut s = session();
    let g = live_squares(&mut s, &[0.0, 50.0, 100.0, 150.0]);
    let [a, b, c, d] = faces_at(&s, g, &[(25.0, 25.0), (75.0, 25.0), (125.0, 25.0), (175.0, 25.0)])[..] else { panic!("four faces") };
    let (red, blue) = (Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let all = edges_in(&s, g, Rect::new(-1.0, -1.0, 201.0, 51.0));
    paint_parts(&mut s, &all, &Paint::None, &[(a, red.clone()), (b, blue.clone()), (c, red.clone()), (d, red.clone())]);
    let stroked = edges_in(&s, g, Rect::new(100.0, 0.0, 100.0, 50.0));
    paint_parts(&mut s, &stroked, &Paint::solid(Color::rgb(0.0, 0.0, 0.0)), &[]);
    let depth = s.doc().unwrap().history.undo.len();
    let r = s.execute("livePaint.fill", &json!({"group": g.0, "point": [25, 25], "color": "#00ff00", "scope": "samePaint"})).unwrap();
    assert_eq!(r["count"], 3);
    assert_eq!([a, b, c, d].map(|f| fill_of(&s, f)), [green.clone(), blue.clone(), green.clone(), green.clone()]);
    s.execute("edit.undo", &json!({})).unwrap();
    // The tool: the first click fills the face, the double-click floods across x = 50 into B, and
    // the triple-click fills the faces that were red.
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.fill = green.clone();
    for kind in [
        PointerKind::Down,
        PointerKind::Up,
        PointerKind::Down,
        PointerKind::Up,
        PointerKind::DoubleClick,
        PointerKind::Down,
        PointerKind::Up,
        PointerKind::TripleClick,
    ] {
        s.pointer(&PointerEvent::new(kind, 25.0, 25.0), v).unwrap();
    }
    assert_eq!([a, b, c, d].map(|f| fill_of(&s, f)), [green.clone(), green.clone(), green.clone(), green.clone()]);
    // A match given by the caller is the fill compared.
    while s.doc().unwrap().history.undo.len() > depth {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert_eq!([a, b, c, d].map(|f| fill_of(&s, f)), [red.clone(), blue.clone(), red.clone(), red.clone()]);
    let r = s
        .execute(
            "livePaint.fill",
            &json!({"group": g.0, "point": [25, 25], "color": "#00ff00", "scope": "samePaint", "match": serde_json::to_value(&blue).unwrap()}),
        )
        .unwrap();
    assert_eq!(r["count"], 2);
    assert_eq!([a, b, c, d].map(|f| fill_of(&s, f)), [green.clone(), green, red.clone(), red]);
}

/// A triple-click whose first click is released across an edge, in the face it then clicks twice,
/// matches that face's fill from before the first click: the release painted it too.
#[test]
fn live_paint_bucket_triple_click_after_a_release_across_an_edge_matches_the_fill_before() {
    let mut s = session();
    let g = live_squares(&mut s, &[0.0, 50.0, 100.0, 150.0]);
    let [a, b, c, d] = faces_at(&s, g, &[(25.0, 25.0), (75.0, 25.0), (125.0, 25.0), (175.0, 25.0)])[..] else { panic!("four faces") };
    let (red, blue) = (Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let all = edges_in(&s, g, Rect::new(-1.0, -1.0, 201.0, 51.0));
    paint_parts(&mut s, &all, &Paint::solid(Color::rgb(0.0, 0.0, 0.0)), &[(a, red.clone()), (b, red.clone()), (c, blue.clone()), (d, red.clone())]);
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.fill = green.clone();
    s.pointer(&PointerEvent::new(PointerKind::Down, 45.0, 25.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 55.0, 25.0), v).unwrap();
    assert_eq!([a, b].map(|f| fill_of(&s, f)), [green.clone(), green.clone()], "the release across x = 50 fills both faces");
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick, PointerKind::Down, PointerKind::Up, PointerKind::TripleClick] {
        s.pointer(&PointerEvent::new(kind, 55.0, 25.0), v).unwrap();
    }
    assert_eq!([a, b, c, d].map(|f| fill_of(&s, f)), [green.clone(), green.clone(), blue, green]);
}

/// A triple-click whose third press lands in another face and is released in the face its first
/// two clicks painted still matches that face's fill from before the first click.
#[test]
fn live_paint_bucket_triple_click_released_from_another_face_matches_the_fill_before() {
    let mut s = session();
    let g = live_squares(&mut s, &[0.0, 50.0, 100.0, 150.0]);
    let [a, b, c, d] = faces_at(&s, g, &[(25.0, 25.0), (75.0, 25.0), (125.0, 25.0), (175.0, 25.0)])[..] else { panic!("four faces") };
    let (red, blue) = (Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let all = edges_in(&s, g, Rect::new(-1.0, -1.0, 201.0, 51.0));
    paint_parts(&mut s, &all, &Paint::solid(Color::rgb(0.0, 0.0, 0.0)), &[(a, red.clone()), (b, red.clone()), (c, blue.clone()), (d, red.clone())]);
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.fill = green.clone();
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick] {
        s.pointer(&PointerEvent::new(kind, 55.0, 25.0), v).unwrap();
    }
    s.pointer(&PointerEvent::new(PointerKind::Down, 45.0, 25.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 55.0, 25.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::TripleClick, 55.0, 25.0), v).unwrap();
    assert_eq!([a, b, c, d].map(|f| fill_of(&s, f)), [green.clone(), green.clone(), blue, green]);
}

/// A triple-click whose first click makes the Live Paint group and is released across an edge, in
/// the face it then clicks twice, matches the fill that face was made with.
#[test]
fn live_paint_bucket_triple_click_after_making_the_group_across_an_edge_matches_the_made_fill() {
    let mut s = session();
    let ids: Vec<NodeId> = [0.0, 50.0, 200.0, 300.0].iter().map(|&x| rect(&mut s, x, 0.0, 50.0, 50.0)).collect();
    let (red, blue) = (Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0));
    for (id, c) in ids.iter().zip([red, red, blue, red]) {
        set_fill(&mut s, *id, c);
    }
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.fill = green.clone();
    s.pointer(&PointerEvent::new(PointerKind::Down, 45.0, 25.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 55.0, 25.0), v).unwrap();
    let g = vectorcraft_tools::builder::live_paint_at(&s.doc().unwrap().doc, Point::new(25.0, 25.0)).expect("the click made a group");
    let faces = faces_at(&s, g, &[(25.0, 25.0), (75.0, 25.0), (225.0, 25.0), (325.0, 25.0)]);
    assert_eq!(faces[..2].iter().map(|f| fill_of(&s, *f)).collect::<Vec<_>>(), [green.clone(), green.clone()], "the release fills both faces");
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick, PointerKind::Down, PointerKind::Up, PointerKind::TripleClick] {
        s.pointer(&PointerEvent::new(kind, 55.0, 25.0), v).unwrap();
    }
    assert_eq!(faces.iter().map(|f| fill_of(&s, *f)).collect::<Vec<_>>(), [green.clone(), green.clone(), Paint::solid(blue), green]);
}

/// When the first click makes the Live Paint group and is released in a face covered by a source
/// path with no fill in front of a filled one, the face was made with the filled source's fill, as
/// Make gives it, so the triple-click there matches that fill and not none.
#[test]
fn live_paint_bucket_triple_click_after_making_the_group_under_an_unfilled_path_matches_the_made_fill() {
    let mut s = session();
    let red = Color::rgb(1.0, 0.0, 0.0);
    let (a, b, d) = (rect(&mut s, 0.0, 0.0, 50.0, 50.0), rect(&mut s, 50.0, 0.0, 50.0, 50.0), rect(&mut s, 200.0, 0.0, 50.0, 50.0));
    for id in [a, b, d] {
        set_fill(&mut s, id, red);
    }
    let (c, e) = (rect(&mut s, 60.0, 10.0, 30.0, 30.0), rect(&mut s, 300.0, 0.0, 50.0, 50.0));
    s.edit("t", |doc, _| {
        for id in [c, e] {
            doc.node_mut(id).unwrap().appearance.set_fill(Paint::None);
        }
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0, d.0, c.0, e.0]})).unwrap();
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.fill = green.clone();
    s.pointer(&PointerEvent::new(PointerKind::Down, 45.0, 25.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 75.0, 25.0), v).unwrap();
    let g = vectorcraft_tools::builder::live_paint_at(&s.doc().unwrap().doc, Point::new(25.0, 25.0)).expect("the click made a group");
    let faces = faces_at(&s, g, &[(75.0, 25.0), (225.0, 25.0), (325.0, 25.0)]);
    assert_eq!(fill_of(&s, faces[0]), green, "the release fills the face under the unfilled path");
    assert_eq!([faces[1], faces[2]].map(|f| fill_of(&s, f)), [Paint::solid(red), Paint::None], "Make gave the far faces red and none");
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick, PointerKind::Down, PointerKind::Up, PointerKind::TripleClick] {
        s.pointer(&PointerEvent::new(kind, 75.0, 25.0), v).unwrap();
    }
    assert_eq!([faces[1], faces[2]].map(|f| fill_of(&s, f)), [green, Paint::None], "the triple-click matches the red the face was made with");
}

/// A triple-click whose first click adds paths to a selected Live Paint group and is released in a
/// face of that group matches the face's fill, not the edge around it.
#[test]
fn live_paint_bucket_triple_click_after_merging_across_an_edge_matches_the_face_fill() {
    let mut s = session();
    let g = live_squares(&mut s, &[0.0]);
    let [a] = faces_at(&s, g, &[(25.0, 25.0)])[..] else { panic!("one face") };
    let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let all = edges_in(&s, g, Rect::new(-1.0, -1.0, 51.0, 51.0));
    paint_parts(&mut s, &all, &Paint::solid(Color::rgb(0.0, 0.0, 0.0)), &[(a, red.clone())]);
    let (next, far) = (rect(&mut s, 50.0, 0.0, 50.0, 50.0), rect(&mut s, 200.0, 0.0, 50.0, 50.0));
    set_fill(&mut s, next, Color::rgb(0.0, 0.0, 1.0));
    set_fill(&mut s, far, Color::rgb(1.0, 0.0, 0.0));
    s.execute("select.set", &json!({"ids": [g.0, next.0, far.0]})).unwrap();
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.fill = green.clone();
    s.pointer(&PointerEvent::new(PointerKind::Down, 55.0, 25.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 45.0, 25.0), v).unwrap();
    let g = vectorcraft_tools::builder::live_paint_at(&s.doc().unwrap().doc, Point::new(25.0, 25.0)).expect("the click merged the paths");
    let faces = faces_at(&s, g, &[(25.0, 25.0), (225.0, 25.0)]);
    assert_eq!(fill_of(&s, faces[0]), green, "the release fills the group's face");
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick, PointerKind::Down, PointerKind::Up, PointerKind::TripleClick] {
        s.pointer(&PointerEvent::new(kind, 45.0, 25.0), v).unwrap();
    }
    assert_eq!(fill_of(&s, faces[1]), green, "the far face had the red the face had");
}

/// When the first click merges two selected Live Paint groups, a face of the group in front takes
/// its source path's fill, so a triple-click there after a release across an edge matches that fill
/// and not the paint the face had before the merge.
#[test]
fn live_paint_bucket_triple_click_after_merging_two_groups_matches_the_merged_fill() {
    let mut s = session();
    let back = live_squares(&mut s, &[0.0]);
    let source = rect(&mut s, 100.0, 0.0, 50.0, 50.0);
    set_fill(&mut s, source, Color::rgb(1.0, 0.0, 0.0));
    s.execute("select.set", &json!({"ids": [source.0]})).unwrap();
    let front = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let [f] = faces_at(&s, front, &[(125.0, 25.0)])[..] else { panic!("one face") };
    let blue = Paint::solid(Color::rgb(0.0, 0.0, 1.0));
    paint_parts(&mut s, &[], &Paint::None, &[(f, blue.clone())]);
    let (next, far) = (rect(&mut s, 150.0, 0.0, 50.0, 50.0), rect(&mut s, 300.0, 0.0, 50.0, 50.0));
    set_fill(&mut s, next, Color::rgb(1.0, 1.0, 0.0));
    set_fill(&mut s, far, Color::rgb(1.0, 0.0, 0.0));
    s.execute("select.set", &json!({"ids": [back.0, front.0, next.0, far.0]})).unwrap();
    let merged = NodeId(s.execute("livePaint.merge", &json!({})).unwrap()["id"].as_u64().unwrap());
    let [m] = faces_at(&s, merged, &[(125.0, 25.0)])[..] else { panic!("one face") };
    assert_eq!(fill_of(&s, m), Paint::solid(Color::rgb(1.0, 0.0, 0.0)), "Merge gives the front group's face its source's fill");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [back.0, front.0, next.0, far.0]})).unwrap();
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.fill = green.clone();
    s.pointer(&PointerEvent::new(PointerKind::Down, 155.0, 25.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 145.0, 25.0), v).unwrap();
    let g = vectorcraft_tools::builder::live_paint_at(&s.doc().unwrap().doc, Point::new(125.0, 25.0)).expect("the click merged the groups");
    let faces = faces_at(&s, g, &[(125.0, 25.0), (325.0, 25.0)]);
    assert_eq!(fill_of(&s, faces[0]), green, "the release fills the front group's face");
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick, PointerKind::Down, PointerKind::Up, PointerKind::TripleClick] {
        s.pointer(&PointerEvent::new(kind, 145.0, 25.0), v).unwrap();
    }
    assert_eq!(fill_of(&s, faces[1]), green, "the far face had the red the merge gave the face");
}

/// When the first click merges paths into a selected Live Paint group whose face has an unplaced
/// gradient, Merge fixes the gradient to the face's fit, so a triple-click there after a release
/// across an edge matches the fixed gradient: a far face whose source has that same fixed gradient
/// takes the bucket's paint.
#[test]
fn live_paint_bucket_triple_click_after_merging_matches_the_fixed_gradient() {
    use vectorcraft_color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let stops = vec![GradientStop::new(0.0, Color::BLACK), GradientStop::new(1.0, Color::WHITE)];
    let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    let mut s = session();
    let g = live_squares(&mut s, &[0.0]);
    let [f] = faces_at(&s, g, &[(25.0, 25.0)])[..] else { panic!("one face") };
    paint_parts(&mut s, &[], &Paint::None, &[(f, gradient.clone())]);
    let (next, far) = (rect(&mut s, 50.0, 0.0, 50.0, 50.0), rect(&mut s, 200.0, 0.0, 50.0, 50.0));
    set_fill(&mut s, next, Color::rgb(1.0, 1.0, 0.0));
    s.execute("select.set", &json!({"ids": [g.0, next.0]})).unwrap();
    let merged = NodeId(s.execute("livePaint.merge", &json!({})).unwrap()["id"].as_u64().unwrap());
    let [m] = faces_at(&s, merged, &[(25.0, 25.0)])[..] else { panic!("one face") };
    let fixed = fill_of(&s, m);
    assert_ne!(fixed, gradient, "Merge fixes the face's gradient to its fit");
    s.execute("edit.undo", &json!({})).unwrap();
    s.edit("t", |d, _| {
        d.node_mut(far).unwrap().appearance.set_fill(fixed.clone());
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [g.0, next.0, far.0]})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    s.paint.fill = green.clone();
    s.pointer(&PointerEvent::new(PointerKind::Down, 55.0, 25.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 45.0, 25.0), v).unwrap();
    let g = vectorcraft_tools::builder::live_paint_at(&s.doc().unwrap().doc, Point::new(45.0, 25.0)).expect("the click merged the paths");
    let [far_face] = faces_at(&s, g, &[(225.0, 25.0)])[..] else { panic!("the far face") };
    assert_eq!(fill_of(&s, far_face), fixed, "the far face keeps its source's fixed gradient");
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick, PointerKind::Down, PointerKind::Up, PointerKind::TripleClick] {
        s.pointer(&PointerEvent::new(kind, 45.0, 25.0), v).unwrap();
    }
    assert_eq!(fill_of(&s, far_face), green, "the far face had the fixed gradient the merge gave the face");
}

/// When the first click merges a line that splits a painted face of the selected Live Paint group,
/// Merge keeps the face's paint in one part only and the other part takes its source path's fill;
/// a triple-click in that other part after a release across an edge matches the source's fill.
#[test]
fn live_paint_bucket_triple_click_after_a_merge_splits_a_face_matches_the_part_fill() {
    let mut s = session();
    let source = rect(&mut s, 0.0, 0.0, 50.0, 50.0);
    set_fill(&mut s, source, Color::rgb(1.0, 0.0, 0.0));
    s.execute("select.set", &json!({"ids": [source.0]})).unwrap();
    let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let [f] = faces_at(&s, g, &[(25.0, 25.0)])[..] else { panic!("one face") };
    paint_parts(&mut s, &[], &Paint::None, &[(f, Paint::solid(Color::rgb(0.0, 0.0, 1.0)))]);
    let line = NodeId(s.execute("shape.line", &json!({"x1": 40, "y1": -10, "x2": 40, "y2": 60})).unwrap()["id"].as_u64().unwrap());
    let next = rect(&mut s, 50.0, 0.0, 50.0, 50.0);
    set_fill(&mut s, next, Color::rgb(1.0, 1.0, 0.0));
    let (red_far, blue_far) = (rect(&mut s, 200.0, 0.0, 50.0, 50.0), rect(&mut s, 300.0, 0.0, 50.0, 50.0));
    set_fill(&mut s, red_far, Color::rgb(1.0, 0.0, 0.0));
    set_fill(&mut s, blue_far, Color::rgb(0.0, 0.0, 1.0));
    let ids = json!({"ids": [g.0, line.0, next.0, red_far.0, blue_far.0]});
    s.execute("select.set", &ids).unwrap();
    let merged = NodeId(s.execute("livePaint.merge", &json!({})).unwrap()["id"].as_u64().unwrap());
    let parts = faces_at(&s, merged, &[(20.0, 25.0), (45.0, 25.0)]);
    assert_eq!(
        parts.iter().map(|p| fill_of(&s, *p)).collect::<Vec<_>>(),
        [Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::solid(Color::rgb(1.0, 0.0, 0.0))],
        "Merge keeps the paint in the part that holds the face's inside point"
    );
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &ids).unwrap();
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.fill = green.clone();
    s.pointer(&PointerEvent::new(PointerKind::Down, 55.0, 25.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 45.0, 25.0), v).unwrap();
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick, PointerKind::Down, PointerKind::Up, PointerKind::TripleClick] {
        s.pointer(&PointerEvent::new(kind, 45.0, 25.0), v).unwrap();
    }
    let g = vectorcraft_tools::builder::live_paint_at(&s.doc().unwrap().doc, Point::new(45.0, 25.0)).expect("the click merged the paths");
    let far = faces_at(&s, g, &[(225.0, 25.0), (325.0, 25.0)]);
    assert_eq!(far.iter().map(|p| fill_of(&s, *p)).collect::<Vec<_>>(), [green, Paint::solid(Color::rgb(0.0, 0.0, 1.0))]);
}

/// A triple-click whose first click makes the Live Paint group from the selected paths matches the
/// fill the face was made with.
#[test]
fn live_paint_bucket_triple_click_after_making_the_group_matches_the_made_fill() {
    let mut s = session();
    let ids: Vec<NodeId> = [0.0, 100.0, 200.0].iter().map(|&x| rect(&mut s, x, 0.0, 50.0, 50.0)).collect();
    let (red, blue) = (Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0));
    for (id, c) in ids.iter().zip([red, red, blue]) {
        set_fill(&mut s, *id, c);
    }
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.fill = green.clone();
    for kind in [
        PointerKind::Down,
        PointerKind::Up,
        PointerKind::Down,
        PointerKind::Up,
        PointerKind::DoubleClick,
        PointerKind::Down,
        PointerKind::Up,
        PointerKind::TripleClick,
    ] {
        s.pointer(&PointerEvent::new(kind, 25.0, 25.0), v).unwrap();
    }
    let g = vectorcraft_tools::builder::live_paint_at(&s.doc().unwrap().doc, Point::new(25.0, 25.0)).expect("the clicks made a group");
    let faces = faces_at(&s, g, &[(25.0, 25.0), (125.0, 25.0), (225.0, 25.0)]);
    assert_eq!(faces.iter().map(|f| fill_of(&s, *f)).collect::<Vec<_>>(), [green.clone(), green, Paint::solid(blue)]);
}

/// A double-click on an edge paints the edges joined to it that have its stroke, and a
/// triple-click every edge of the group with that stroke; an edge with another stroke is left.
#[test]
fn live_paint_bucket_double_and_triple_click_paint_joined_and_same_stroke_edges() {
    let mut s = session();
    let g = live_squares(&mut s, &[0.0, 50.0, 200.0]);
    let black = Paint::solid(Color::rgb(0.0, 0.0, 0.0));
    let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let joined = edges_in(&s, g, Rect::new(0.0, 0.0, 100.0, 50.0));
    let apart = edges_in(&s, g, Rect::new(200.0, 0.0, 250.0, 50.0));
    let middle = edges_in(&s, g, Rect::new(50.0, 0.0, 50.0, 50.0));
    assert!(!apart.is_empty() && !middle.is_empty());
    paint_parts(&mut s, &joined, &black, &[]);
    paint_parts(&mut s, &apart, &black, &[]);
    paint_parts(&mut s, &middle, &red, &[]);
    let stroke = |s: &Session, ids: &[NodeId]| ids.iter().map(|id| node(s, *id).appearance.stroke_paint()).collect::<Vec<_>>();
    let outer: Vec<NodeId> = joined.iter().copied().filter(|e| !middle.contains(e)).collect();
    let r =
        s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [25, 0], "color": "#00ff00", "width": 3, "scope": "connected"})).unwrap();
    assert_eq!(r["count"], outer.len());
    assert!(stroke(&s, &outer).iter().all(|p| *p == green));
    assert!(outer.iter().all(|e| node(&s, *e).appearance.stroke_width() == 3.0));
    assert!(stroke(&s, &middle).iter().all(|p| *p == red), "another stroke is left");
    assert!(stroke(&s, &apart).iter().all(|p| *p == black), "edges not joined are left");
    s.execute("edit.undo", &json!({})).unwrap();
    let r = s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [25, 0], "color": "#00ff00", "scope": "samePaint"})).unwrap();
    assert_eq!(r["count"], outer.len() + apart.len());
    assert!(stroke(&s, &outer).iter().chain(&stroke(&s, &apart)).all(|p| *p == green));
    assert!(stroke(&s, &middle).iter().all(|p| *p == red));
    // A hidden edge is not painted.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("object.setProps", &json!({"ids": [apart[0].0], "visible": false})).unwrap();
    let r = s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [25, 0], "color": "#00ff00", "scope": "samePaint"})).unwrap();
    assert_eq!(r["count"], outer.len() + apart.len() - 1);
    assert_eq!(node(&s, apart[0]).appearance.stroke_paint(), black);
}

/// A double-click whose two clicks land on two joined edges paints every edge joined to them that
/// had their stroke: the edge the first click painted still joins the others.
#[test]
fn live_paint_bucket_double_click_across_a_junction_paints_the_joined_edges() {
    // A square with a line across its middle, cut at x = 30 and x = 70 by two lines from the top: the
    // middle line's three edges join end to end.
    let mut s = session();
    let mut ids = vec![rect(&mut s, 0.0, 0.0, 100.0, 100.0).0];
    for anchors in [
        json!([{"x": 0, "y": 50}, {"x": 100, "y": 50}]),
        json!([{"x": 30, "y": 0}, {"x": 30, "y": 70}]),
        json!([{"x": 70, "y": 0}, {"x": 70, "y": 70}]),
    ] {
        ids.push(s.execute("path.create", &json!({ "anchors": anchors })).unwrap()["id"].as_u64().unwrap());
    }
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let (black, red) = (Paint::solid(Color::rgb(0.0, 0.0, 0.0)), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let all = edges_in(&s, g, Rect::new(-1.0, -1.0, 101.0, 101.0));
    paint_parts(&mut s, &all, &red, &[]);
    let middle = edges_in(&s, g, Rect::new(-1.0, 49.0, 101.0, 51.0));
    assert_eq!(middle.len(), 3, "the middle line's three edges");
    paint_parts(&mut s, &middle, &black, &[]);
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.stroke = green.clone();
    let shift = Mods { shift: true, ..Default::default() };
    for (kind, x) in
        [(PointerKind::Down, 50.0), (PointerKind::Up, 50.0), (PointerKind::Down, 85.0), (PointerKind::Up, 85.0), (PointerKind::DoubleClick, 85.0)]
    {
        s.pointer(&PointerEvent::new(kind, x, 50.0).with_mods(shift), v).unwrap();
    }
    assert_eq!(middle.iter().map(|e| node(&s, *e).appearance.stroke_paint()).collect::<Vec<_>>(), [green.clone(), green.clone(), green]);
    // `through` takes the group's edge ids, each once, with scope "connected" only; a face's id, an
    // unknown id or an edge given twice is refused and changes nothing.
    let face = vectorcraft_tools::builder::faces(&node(&s, g)).first().map(|f| f.id).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let depth = s.doc().unwrap().history.undo.len();
    for p in [
        json!({"group": g.0, "point": [15, 50], "scope": "connected", "through": "x"}),
        json!({"group": g.0, "point": [15, 50], "scope": "connected", "through": [-1]}),
        json!({"group": g.0, "point": [15, 50], "scope": "samePaint", "through": []}),
        json!({"group": g.0, "point": [15, 50], "through": []}),
        json!({"group": g.0, "point": [15, 50], "scope": "connected", "through": [face.0]}),
        json!({"group": g.0, "point": [15, 50], "scope": "connected", "through": [999_999_999]}),
        json!({"group": g.0, "point": [15, 50], "scope": "connected", "through": [middle[1].0, middle[1].0]}),
    ] {
        assert!(s.execute("livePaint.strokeEdge", &p).is_err(), "{p}");
    }
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth);
    let r = s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [15, 50], "scope": "connected", "through": [middle[1].0]})).unwrap();
    assert_eq!(r["count"], 3, "an edge of the group is taken");
}

/// A double-click whose second press lands on the next joined edge and is released back on the
/// edge the first click painted (at the end of the line) matches that edge's stroke paint from
/// before the first click, and the edge the second press painted still joins the edge past it.
#[test]
fn live_paint_bucket_double_click_released_back_on_the_first_edge_paints_the_joined_edges() {
    let mut s = session();
    let mut ids = vec![rect(&mut s, 0.0, 0.0, 100.0, 100.0).0];
    for anchors in [
        json!([{"x": 0, "y": 50}, {"x": 100, "y": 50}]),
        json!([{"x": 30, "y": 0}, {"x": 30, "y": 70}]),
        json!([{"x": 70, "y": 0}, {"x": 70, "y": 70}]),
    ] {
        ids.push(s.execute("path.create", &json!({ "anchors": anchors })).unwrap()["id"].as_u64().unwrap());
    }
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let (black, red) = (Paint::solid(Color::rgb(0.0, 0.0, 0.0)), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let all = edges_in(&s, g, Rect::new(-1.0, -1.0, 101.0, 101.0));
    paint_parts(&mut s, &all, &red, &[]);
    let middle = edges_in(&s, g, Rect::new(-1.0, 49.0, 101.0, 51.0));
    assert_eq!(middle.len(), 3, "the middle line's three edges");
    paint_parts(&mut s, &middle, &black, &[]);
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.stroke = green.clone();
    let shift = Mods { shift: true, ..Default::default() };
    for (kind, x) in
        [(PointerKind::Down, 15.0), (PointerKind::Up, 15.0), (PointerKind::Down, 50.0), (PointerKind::Up, 15.0), (PointerKind::DoubleClick, 15.0)]
    {
        s.pointer(&PointerEvent::new(kind, x, 50.0).with_mods(shift), v).unwrap();
    }
    assert_eq!(middle.iter().map(|e| node(&s, *e).appearance.stroke_paint()).collect::<Vec<_>>(), [green.clone(), green.clone(), green]);
}

/// With Paint Fills and Paint Strokes both on, a double-click on an edge after a click that filled
/// a face whose fill was the edge's stroke paint paints the joined edges: the face is not sent as
/// an edge to join through.
#[test]
fn live_paint_bucket_edge_double_click_after_filling_a_face_paints_the_joined_edges() {
    let mut s = session();
    let g = live_squares(&mut s, &[0.0, 50.0]);
    let [a, b] = faces_at(&s, g, &[(25.0, 25.0), (75.0, 25.0)])[..] else { panic!("two faces") };
    let black = Paint::solid(Color::rgb(0.0, 0.0, 0.0));
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let all = edges_in(&s, g, Rect::new(-1.0, -1.0, 101.0, 51.0));
    paint_parts(&mut s, &all, &black, &[(a, black.clone()), (b, black.clone())]);
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.set_tool_option("paintStrokes", &json!(true));
    s.paint.fill = green.clone();
    s.paint.stroke = green.clone();
    for (kind, x) in [(PointerKind::Down, 25.0), (PointerKind::Up, 25.0), (PointerKind::Down, 50.0), (PointerKind::Up, 50.0)] {
        s.pointer(&PointerEvent::new(kind, x, 25.0), v).unwrap();
    }
    assert_eq!(fill_of(&s, a), green, "the first click fills the face");
    s.pointer(&PointerEvent::new(PointerKind::DoubleClick, 50.0, 25.0), v).unwrap();
    assert!(all.iter().all(|e| node(&s, *e).appearance.stroke_paint() == green), "every joined edge is painted");
    assert_eq!(fill_of(&s, b), black);
}

/// A triple-click whose second press holds Cmd (it goes to the selection tool Cmd lends) matches
/// the fill the face had before the first click: the lent press counts as one that paints nothing.
#[test]
fn live_paint_bucket_triple_click_with_a_cmd_press_between_matches_the_fill_before() {
    let mut s = session();
    let g = live_squares(&mut s, &[0.0, 50.0, 100.0, 150.0]);
    let [a, b, c, d] = faces_at(&s, g, &[(25.0, 25.0), (75.0, 25.0), (125.0, 25.0), (175.0, 25.0)])[..] else { panic!("four faces") };
    let (red, blue) = (Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let all = edges_in(&s, g, Rect::new(-1.0, -1.0, 201.0, 51.0));
    paint_parts(&mut s, &all, &Paint::solid(Color::rgb(0.0, 0.0, 0.0)), &[(a, red.clone()), (b, red.clone()), (c, blue.clone()), (d, red.clone())]);
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.fill = green.clone();
    let cmd = Mods { cmd: true, ..Default::default() };
    for (kind, m) in [
        (PointerKind::Down, Mods::default()),
        (PointerKind::Up, Mods::default()),
        (PointerKind::Down, cmd),
        (PointerKind::Up, cmd),
        (PointerKind::Down, Mods::default()),
        (PointerKind::Up, Mods::default()),
        (PointerKind::TripleClick, Mods::default()),
    ] {
        s.pointer(&PointerEvent::new(kind, 25.0, 25.0).with_mods(m), v).unwrap();
    }
    assert_eq!(s.tool_id(), "livePaintBucket");
    assert_eq!([a, b, c, d].map(|f| fill_of(&s, f)), [green.clone(), green.clone(), blue, green]);
}

/// A `scope` that is not one of the three, a `match` that is not a paint or is given with a scope
/// that compares none, and a scope with a drag's points are refused; nothing changes.
#[test]
fn live_paint_scope_and_match_are_checked() {
    let mut s = session();
    let g = live(&mut s);
    let none = serde_json::to_value(Paint::None).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let depth = s.doc().unwrap().history.undo.len();
    for (cmd, p) in [
        ("livePaint.fill", json!({"group": g.0, "point": [75, 50], "scope": "all"})),
        ("livePaint.fill", json!({"group": g.0, "point": [75, 50], "scope": 2})),
        ("livePaint.fill", json!({"group": g.0, "point": [75, 50], "match": none})),
        ("livePaint.fill", json!({"group": g.0, "point": [75, 50], "scope": "connected", "match": none})),
        ("livePaint.fill", json!({"group": g.0, "point": [75, 50], "scope": "samePaint", "match": "red"})),
        ("livePaint.fill", json!({"group": g.0, "point": [75, 50], "scope": "connected", "points": [[75, 50], [125, 50]]})),
        ("livePaint.strokeEdge", json!({"group": g.0, "point": [100, 50], "scope": "Connected"})),
        ("livePaint.strokeEdge", json!({"group": g.0, "point": [100, 50], "scope": "one", "match": none})),
        ("livePaint.strokeEdge", json!({"group": g.0, "point": [100, 50], "scope": "samePaint", "match": {"type": "plaid"}})),
    ] {
        assert!(s.execute(cmd, &p).is_err(), "{cmd} {p}");
    }
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth);
    // "one" and null are a click's scope.
    for scope in [json!("one"), Value::Null] {
        let r = s.execute("livePaint.fill", &json!({"group": g.0, "point": [75, 50], "scope": scope})).unwrap();
        assert_eq!(r["count"], 1);
    }
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

/// Set the appearance rows of the face of Live Paint group `g` at `face` (or the edge nearest
/// `edge`) to `items`, and give it `opacity`; returns that child's id.
fn set_live_part(s: &mut Session, g: NodeId, face: Option<(f64, f64)>, edge: Option<(f64, f64)>, items: Vec<AppearanceItem>, opacity: f32) -> NodeId {
    use vectorcraft_tools::builder::{edge_near, face_at};
    let n = node(s, g);
    let part = match (face, edge) {
        (Some((x, y)), _) => face_at(&n, Point::new(x, y)),
        (_, Some((x, y))) => edge_near(&n, Point::new(x, y), 0.5),
        _ => None,
    };
    let id = part.unwrap().id;
    s.edit("t", |d, _| {
        let c = d.node_mut(id).unwrap();
        c.appearance.items = items;
        c.opacity = opacity;
        Ok(())
    })
    .unwrap();
    id
}

/// Expand keeps a face or edge when a fill or stroke row draws it, whatever row is on top: a
/// painted lower row under an unpainted top row is kept, and a part whose only painted row is
/// hidden, transparent or 0 pt wide, or whose own opacity is 0, is dropped.
#[test]
fn live_paint_expand_keeps_the_faces_and_edges_that_draw() {
    use vectorcraft_doc::{FillLayer, StrokeLayer};
    let fill = |p: Paint| AppearanceItem::Fill(FillLayer::new(p));
    let stroke = |p: Paint, w: f64| AppearanceItem::Stroke(StrokeLayer::new(p, w));
    let red = || Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let hidden = |mut i: AppearanceItem| {
        match &mut i {
            AppearanceItem::Fill(f) => f.visible = false,
            AppearanceItem::Stroke(st) => st.visible = false,
        }
        i
    };
    let clear = |mut i: AppearanceItem| {
        match &mut i {
            AppearanceItem::Fill(f) => f.opacity = 0.0,
            AppearanceItem::Stroke(st) => st.opacity = 0.0,
        }
        i
    };
    let mut s = session();
    let g = live(&mut s);
    let lower_fill = set_live_part(&mut s, g, Some((25.0, 50.0)), None, vec![fill(red()), fill(Paint::None)], 1.0);
    let hidden_fill = set_live_part(&mut s, g, Some((125.0, 50.0)), None, vec![hidden(fill(red()))], 1.0);
    let clear_fill = set_live_part(&mut s, g, Some((75.0, 50.0)), None, vec![clear(fill(red()))], 1.0);
    let lower_stroke = set_live_part(&mut s, g, None, Some((0.0, 50.0)), vec![stroke(red(), 2.0), stroke(Paint::None, 1.0)], 1.0);
    let hidden_stroke = set_live_part(&mut s, g, None, Some((150.0, 50.0)), vec![hidden(stroke(red(), 2.0))], 1.0);
    let thin_stroke = set_live_part(&mut s, g, None, Some((100.0, 50.0)), vec![stroke(red(), 0.0)], 1.0);
    let clear_stroke = set_live_part(&mut s, g, None, Some((50.0, 50.0)), vec![clear(stroke(red(), 2.0))], 1.0);
    let clear_edge = set_live_part(&mut s, g, None, Some((75.0, 0.0)), vec![stroke(red(), 2.0)], 0.0);
    let parts = [lower_fill, hidden_fill, clear_fill, lower_stroke, hidden_stroke, thin_stroke, clear_stroke, clear_edge];
    assert_eq!(parts.iter().collect::<std::collections::HashSet<_>>().len(), parts.len());
    let before = node(&s, g);
    s.execute("livePaint.expand", &json!({})).unwrap();
    let n = node(&s, g);
    let kept: Vec<NodeId> = n.children().unwrap().iter().map(|c| c.id).collect();
    let has = |id: NodeId| kept.contains(&id);
    assert!(has(lower_fill) && has(lower_stroke), "a painted lower row draws: {kept:?}");
    for id in [hidden_fill, clear_fill, hidden_stroke, thin_stroke, clear_stroke, clear_edge] {
        assert!(!has(id), "{id:?} draws nothing: {kept:?}");
    }
    // The kept parts keep every row.
    let rows = |n: &Node, id: NodeId| n.children().unwrap().iter().find(|c| c.id == id).unwrap().appearance.items.clone();
    assert_eq!(rows(&n, lower_fill), rows(&before, lower_fill));
    assert_eq!(rows(&n, lower_stroke), rows(&before, lower_stroke));
}

/// `livePaint.info` lists every fill and stroke row of each face and edge, bottom first, and says
/// which rows draw by Expand's rule, while `fill`, `stroke` and `width` stay the top rows.
#[test]
fn live_paint_info_lists_every_appearance_row() {
    use vectorcraft_doc::{FillLayer, StrokeLayer};
    let fill = |p: Paint| AppearanceItem::Fill(FillLayer::new(p));
    let stroke = |p: Paint, w: f64| AppearanceItem::Stroke(StrokeLayer::new(p, w));
    let red = || Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let blue = || Paint::solid(Color::rgb(0.0, 0.0, 1.0));
    let green = || Paint::solid(Color::rgb(0.0, 1.0, 0.0));
    let hidden = |mut i: AppearanceItem| {
        match &mut i {
            AppearanceItem::Fill(f) => f.visible = false,
            AppearanceItem::Stroke(st) => st.visible = false,
        }
        i
    };
    let faded = |mut i: AppearanceItem, o: f32| {
        match &mut i {
            AppearanceItem::Fill(f) => f.opacity = o,
            AppearanceItem::Stroke(st) => st.opacity = o,
        }
        i
    };
    let mut s = session();
    let g = live(&mut s);
    // A drawn lower fill and a half-opaque stroke under an unpainted top fill.
    let lower = set_live_part(&mut s, g, Some((25.0, 50.0)), None, vec![fill(red()), faded(stroke(blue(), 2.0), 0.5), fill(Paint::None)], 1.0);
    let quiet = set_live_part(&mut s, g, Some((125.0, 50.0)), None, vec![hidden(fill(red())), faded(fill(red()), 0.0)], 1.0);
    let bare = set_live_part(&mut s, g, Some((75.0, 50.0)), None, vec![], 1.0);
    // Two strokes with a fill between them.
    let two = set_live_part(&mut s, g, None, Some((0.0, 50.0)), vec![stroke(red(), 4.0), fill(green()), stroke(blue(), 2.0)], 1.0);
    let thin = set_live_part(&mut s, g, None, Some((100.0, 50.0)), vec![stroke(red(), 0.0)], 1.0);
    let clear_edge = set_live_part(&mut s, g, None, Some((75.0, 0.0)), vec![stroke(red(), 2.0)], 0.0);
    let hidden_edge = set_live_part(&mut s, g, None, Some((150.0, 50.0)), vec![stroke(red(), 2.0)], 1.0);
    s.edit("t", |d, _| {
        d.node_mut(hidden_edge).unwrap().visible = false;
        Ok(())
    })
    .unwrap();
    let parts = [lower, quiet, bare, two, thin, clear_edge, hidden_edge];
    assert_eq!(parts.iter().collect::<std::collections::HashSet<_>>().len(), parts.len());

    let info = s.execute("livePaint.info", &json!({"group": g.0})).unwrap();
    let part = |key: &str, id: NodeId| info[key].as_array().unwrap().iter().find(|p| p["id"] == id.0).unwrap().clone();
    let fill_row = |i: usize, p: Paint, o: f64, v: bool, drawn: bool| json!({"index": i, "kind": "fill", "paint": p.label(), "opacity": o, "visible": v, "drawn": drawn});
    let stroke_row = |i: usize, p: Paint, o: f64, v: bool, w: f64, drawn: bool| json!({"index": i, "kind": "stroke", "paint": p.label(), "opacity": o, "visible": v, "width": w, "drawn": drawn});

    let f = part("faces", lower);
    assert_eq!(f["fill"], json!(Paint::None.label()), "fill stays the top fill");
    assert!(f["bounds"].is_array());
    assert_eq!(
        f["appearance"],
        json!([fill_row(0, red(), 1.0, true, true), stroke_row(1, blue(), 0.5, true, 2.0, true), fill_row(2, Paint::None, 1.0, true, false)])
    );
    assert_eq!(part("faces", quiet)["appearance"], json!([fill_row(0, red(), 1.0, false, false), fill_row(1, red(), 0.0, true, false)]));
    assert_eq!(part("faces", bare)["appearance"], json!([]));

    let e = part("edges", two);
    assert_eq!((e["stroke"].clone(), e["width"].clone()), (json!(blue().label()), json!(2.0)), "stroke and width stay the top stroke's");
    assert_eq!(
        e["appearance"],
        json!([stroke_row(0, red(), 1.0, true, 4.0, true), fill_row(1, green(), 1.0, true, true), stroke_row(2, blue(), 1.0, true, 2.0, true)])
    );
    assert_eq!(part("edges", thin)["appearance"], json!([stroke_row(0, red(), 1.0, true, 0.0, false)]));
    for id in [clear_edge, hidden_edge] {
        assert_eq!(part("edges", id)["appearance"], json!([stroke_row(0, red(), 1.0, true, 2.0, false)]), "a part that is not shown draws no row");
    }
}

/// Merge carries a face's whole appearance when one of its rows draws, and carries nothing from a
/// face that draws nothing.
#[test]
fn live_paint_merge_carries_the_faces_that_draw() {
    use vectorcraft_doc::FillLayer;
    use vectorcraft_tools::builder::face_at;
    let fill = |p: Paint| AppearanceItem::Fill(FillLayer::new(p));
    let green = || Paint::solid(Color::rgb(0.0, 1.0, 0.0));
    let mut s = session();
    let g = live(&mut s);
    let rows = vec![fill(green()), fill(Paint::None)];
    set_live_part(&mut s, g, Some((25.0, 50.0)), None, rows.clone(), 1.0);
    let mut only_hidden = fill(green());
    if let AppearanceItem::Fill(f) = &mut only_hidden {
        f.visible = false;
    }
    set_live_part(&mut s, g, Some((125.0, 50.0)), None, vec![only_hidden], 1.0);
    let c = rect(&mut s, 120.0, 120.0, 100.0, 20.0);
    s.execute("livePaint.merge", &json!({"ids": [g.0, c.0]})).unwrap();
    let n = node(&s, g);
    assert_eq!(face_at(&n, Point::new(25.0, 50.0)).unwrap().appearance.items, rows, "the drawn lower fill is carried");
    let b = face_at(&n, Point::new(125.0, 50.0)).unwrap();
    assert_eq!(b.appearance.fill_paint(), Paint::solid(Color::rgb(0.0, 0.0, 1.0)), "the face takes its source's fill again");
    assert!(b.appearance.has_drawn_fill());
}

/// Merge rebuilds the faces and edges as other paths, so the appearance it carries to them leaves
/// the strokes' pen pressure behind and keeps the rest.
#[test]
fn live_paint_merge_leaves_pen_pressure_behind() {
    use vectorcraft_doc::{FillLayer, PressureProfile, StrokeLayer};
    use vectorcraft_tools::builder::{edge_near, face_at};
    let pressed = || {
        let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), 3.0);
        st.pressure = PressureProfile::from_samples([(0.0, 0.2), (0.5, 1.0), (1.0, 0.2)]);
        assert!(st.pressure.is_some());
        st
    };
    let green = AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::rgb(0.0, 1.0, 0.0))));
    let mut s = session();
    let g = live(&mut s);
    set_live_part(&mut s, g, Some((75.0, 50.0)), None, vec![green.clone(), AppearanceItem::Stroke(pressed())], 1.0);
    set_live_part(&mut s, g, None, Some((0.0, 50.0)), vec![AppearanceItem::Stroke(pressed())], 1.0);
    let c = rect(&mut s, 120.0, 120.0, 100.0, 20.0);
    s.execute("livePaint.merge", &json!({"ids": [g.0, c.0]})).unwrap();
    let n = node(&s, g);
    let mut plain = pressed();
    plain.pressure = None;
    let face = &face_at(&n, Point::new(75.0, 50.0)).unwrap().appearance.items;
    assert_eq!(face, &vec![green, AppearanceItem::Stroke(plain.clone())]);
    assert_eq!(edge_near(&n, Point::new(0.0, 50.0), 0.5).unwrap().appearance.items, vec![AppearanceItem::Stroke(plain)]);
}

/// The gradients Merge carries to the rebuilt faces and edges keep their fit on the old outline,
/// lower rows included.
#[test]
fn live_paint_merge_keeps_the_fit_of_carried_gradients() {
    use vectorcraft_color::{Gradient, GradientKind, GradientPaint, GradientStop};
    use vectorcraft_doc::{FillLayer, StrokeLayer};
    use vectorcraft_tools::builder::{edge_near, face_at};
    let stops = vec![GradientStop::new(0.0, Color::BLACK), GradientStop::new(1.0, Color::WHITE)];
    let unplaced = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let gradient = || Paint::Gradient(Box::new(unplaced.clone()));
    let mut s = session();
    let g = live(&mut s);
    let rows = vec![AppearanceItem::Fill(FillLayer::new(gradient())), AppearanceItem::Fill(FillLayer::new(Paint::None))];
    let face = set_live_part(&mut s, g, Some((25.0, 50.0)), None, rows, 1.0);
    let edge = set_live_part(&mut s, g, None, Some((0.0, 50.0)), vec![AppearanceItem::Stroke(StrokeLayer::new(gradient(), 2.0))], 1.0);
    let before = node(&s, g);
    let old = |id: NodeId| before.children().unwrap().iter().find(|c| c.id == id).unwrap().clone();
    let (old_face, old_edge) = (old(face), old(edge));
    let c = rect(&mut s, 120.0, 120.0, 100.0, 20.0);
    s.execute("livePaint.merge", &json!({"ids": [g.0, c.0]})).unwrap();
    let n = node(&s, g);
    let geom = |items: &[AppearanceItem]| match items[0].paint() {
        Paint::Gradient(gp) => gp.geom,
        p => panic!("a gradient, not {p:?}"),
    };
    let face_bounds = old_face.geometric_bounds().unwrap();
    let new_face = face_at(&n, Point::new(25.0, 50.0)).unwrap();
    assert_eq!(geom(&new_face.appearance.items), Some(unplaced.resolve(face_bounds)), "the lower-row fill keeps its fit");
    let AppearanceItem::Stroke(st) = &old_edge.appearance.items[0] else { panic!("a stroke") };
    let edge_box = st.paint_bounds(old_edge.geometric_bounds().unwrap());
    let new_edge = edge_near(&n, Point::new(0.0, 50.0), 0.5).unwrap();
    assert_eq!(geom(&new_edge.appearance.items), Some(unplaced.resolve(edge_box)), "the stroke keeps its fit");
}

/// Merge carries a face's or edge's visibility, opacity and blending mode to the rebuilt part with
/// its appearance: a half-transparent face drawn by a stroke alone stays half transparent, and a
/// hidden edge stays hidden.
#[test]
fn live_paint_merge_keeps_the_visibility_opacity_and_blending_of_carried_parts() {
    use vectorcraft_color::BlendMode;
    use vectorcraft_doc::StrokeLayer;
    use vectorcraft_tools::builder::{edge_near, face_at};
    let stroke = || AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::BLACK), 2.0));
    let mut s = session();
    let g = live(&mut s);
    let face = set_live_part(&mut s, g, Some((25.0, 50.0)), None, vec![stroke()], 0.5);
    let edge = set_live_part(&mut s, g, None, Some((0.0, 50.0)), vec![stroke()], 1.0);
    s.edit("t", |d, _| {
        d.node_mut(face).unwrap().blend = BlendMode::Multiply;
        d.node_mut(edge).unwrap().visible = false;
        Ok(())
    })
    .unwrap();
    let c = rect(&mut s, 120.0, 120.0, 100.0, 20.0);
    s.execute("livePaint.merge", &json!({"ids": [g.0, c.0]})).unwrap();
    let n = node(&s, g);
    let f = face_at(&n, Point::new(25.0, 50.0)).unwrap();
    assert_eq!((f.appearance.items.clone(), f.opacity, f.blend, f.visible), (vec![stroke()], 0.5, BlendMode::Multiply, true));
    let e = edge_near(&n, Point::new(0.0, 50.0), 0.5).unwrap();
    assert_eq!((e.appearance.items.clone(), e.visible), (vec![stroke()], false), "the hidden edge stays hidden");
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

/// A horizontal open path across [`live`]'s group at `y`, drawn with the Pen's defaults.
fn line_across(s: &mut Session, y: f64) -> NodeId {
    let r = s.execute("path.create", &json!({"anchors": [{"x": -10, "y": y}, {"x": 160, "y": y}]})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

/// Paths drawn in an isolated Live Paint group join it when isolation mode ends: they go among its
/// sources and cut the faces they cross, which take their sources' fill, a painted face they do not
/// cross keeps its fill, and the whole addition is one undo step, which Undo takes back to the
/// drawn paths in the group and Redo makes again.
#[test]
fn live_paint_isolation_adds_drawn_paths_on_exit() {
    use vectorcraft_tools::builder::{face_at, faces, sources};
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let mut s = session();
    let g = live(&mut s);
    s.execute("livePaint.fill", &json!({"group": g.0, "point": [125, 50], "color": "#00ff00"})).unwrap();
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    // Two lines from left of the group that end in its right face, so they cut only the other two.
    let line = |s: &mut Session, y: f64| {
        let r = s.execute("path.create", &json!({"anchors": [{"x": -10, "y": y}, {"x": 105, "y": y}]})).unwrap();
        NodeId(r["id"].as_u64().unwrap())
    };
    let (a, b) = (line(&mut s, 30.0), line(&mut s, 70.0));
    assert_eq!(s.doc().unwrap().doc.parent_of(a), Some(g), "a path drawn in isolation goes into the group");
    let steps = s.doc().unwrap().history.undo.len();
    s.execute("object.exitIsolation", &json!({})).unwrap();
    let d = s.doc().unwrap();
    assert_eq!(d.isolation, None);
    assert_eq!(d.history.undo.len(), steps + 1, "both paths join in one undo step");
    assert_eq!(d.selection.objects, vec![g]);
    let n = node(&s, g);
    let src = sources(&n).unwrap();
    assert_eq!(src.children().unwrap().len(), 4);
    for id in [a, b] {
        assert_eq!(s.doc().unwrap().doc.parent_of(id), Some(src.id), "{id} joined the sources");
    }
    assert_eq!(faces(&n).len(), 7, "two lines cut each of the two faces they cross in three");
    assert_eq!(face_at(&n, Point::new(125.0, 50.0)).unwrap().appearance.fill_paint(), green, "the face the lines do not cross keeps its fill");
    let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    for y in [15.0, 85.0] {
        assert_eq!(face_at(&n, Point::new(25.0, y)).unwrap().appearance.fill_paint(), red, "a new face takes its source's fill");
    }
    s.execute("edit.undo", &json!({})).unwrap();
    let n = node(&s, g);
    assert_eq!(faces(&n).len(), 3);
    assert_eq!(sources(&n).unwrap().children().unwrap().len(), 2);
    for id in [a, b] {
        assert_eq!(s.doc().unwrap().doc.parent_of(id), Some(g), "Undo puts {id} back in the group as drawn");
    }
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(faces(&node(&s, g)).len(), 7);
}

/// Leaving isolation mode in a Live Paint group with nothing drawn in it records no undo step and
/// leaves the document as it is. Type, and a path that is hidden, stay in the group as they are,
/// while a group of paths drawn beside them joins it.
#[test]
fn live_paint_isolation_adds_only_paths_it_takes() {
    use vectorcraft_tools::builder::{faces, sources};
    let mut s = session();
    let g = live(&mut s);
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    let before = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    s.execute("object.exitIsolation", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc == before.0, "an exit with nothing drawn changes nothing");
    assert_eq!(s.doc().unwrap().history.undo.len(), before.1, "and records no undo step");
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    let text = NodeId(s.execute("text.create", &json!({"x": 20, "y": 20, "text": "Hi"})).unwrap()["id"].as_u64().unwrap());
    let hidden = line_across(&mut s, 50.0);
    s.edit("t", |d, _| {
        d.node_mut(hidden).unwrap().visible = false;
        Ok(())
    })
    .unwrap();
    let steps = s.doc().unwrap().history.undo.len();
    s.execute("object.exitIsolation", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), steps, "type and a hidden path add nothing");
    assert_eq!(faces(&node(&s, g)).len(), 3);
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    let (a, b) = (line_across(&mut s, 30.0), line_across(&mut s, 70.0));
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let paths = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    assert_eq!(s.doc().unwrap().doc.parent_of(paths), Some(g));
    s.execute("object.exitIsolation", &json!({})).unwrap();
    let n = node(&s, g);
    assert_eq!(faces(&n).len(), 9, "the group of paths joined");
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.parent_of(paths), Some(sources(&n).unwrap().id));
    for id in [text, hidden] {
        assert_eq!(d.parent_of(id), Some(g), "{id} stays in the group");
        assert!(d.node(id).is_some());
    }
}

/// A group drawn in an isolated Live Paint group that holds a locked path stays in it as it is when
/// isolation mode ends, as a locked path drawn there does.
#[test]
fn live_paint_isolation_leaves_a_group_holding_a_locked_path() {
    use vectorcraft_tools::builder::faces;
    let mut s = session();
    let g = live(&mut s);
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    let (a, b) = (line_across(&mut s, 30.0), line_across(&mut s, 70.0));
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let paths = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.edit("t", |d, _| {
        d.node_mut(b).unwrap().locked = true;
        Ok(())
    })
    .unwrap();
    let steps = s.doc().unwrap().history.undo.len();
    s.execute("object.exitIsolation", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), steps);
    assert_eq!(s.doc().unwrap().doc.parent_of(paths), Some(g));
    assert_eq!(faces(&node(&s, g)).len(), 3);
}

/// Type drawn in an isolated Live Paint group stays in it when Gap Options or Merge make the group's
/// faces and edges again, including type in a Live Paint group merged into another, and Release
/// puts it beside the released paths.
#[test]
fn live_paint_keeps_type_drawn_in_it_through_gap_options_merge_and_release() {
    let mut s = session();
    let g = live(&mut s);
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    let text = NodeId(s.execute("text.create", &json!({"x": 20, "y": 20, "text": "Hi"})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.exitIsolation", &json!({})).unwrap();
    s.execute("livePaint.gapOptions", &json!({"ids": [g.0], "detect": true})).unwrap();
    assert_eq!(s.doc().unwrap().doc.parent_of(text), Some(g), "Gap Options keeps the type");
    let c = rect(&mut s, 120.0, 20.0, 100.0, 20.0);
    s.execute("livePaint.merge", &json!({"ids": [g.0, c.0]})).unwrap();
    assert_eq!(s.doc().unwrap().doc.parent_of(text), Some(g), "Merge keeps the type");
    let other = rect(&mut s, 300.0, 300.0, 50.0, 50.0);
    let h = NodeId(s.execute("livePaint.make", &json!({"ids": [other.0]})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.isolate", &json!({"id": h.0})).unwrap();
    let text2 = NodeId(s.execute("text.create", &json!({"x": 310, "y": 320, "text": "Yo"})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.exitIsolation", &json!({})).unwrap();
    s.execute("livePaint.merge", &json!({"ids": [g.0, h.0]})).unwrap();
    assert_eq!(s.doc().unwrap().doc.parent_of(text2), Some(g), "the type of the merged group moves with its paths");
    s.execute("livePaint.release", &json!({"ids": [g.0]})).unwrap();
    let d = &s.doc().unwrap().doc;
    for id in [text, text2] {
        assert_eq!(d.parent_of(id), d.default_layer(), "Release keeps {id}");
    }
}

/// Art drawn in an isolated Live Paint group and named like one of the group's parts is not taken
/// for one: type named "Live Paint Sources" stays in the group through Gap Options, and a group of
/// paths named "Face" joins the group when isolation mode ends.
#[test]
fn live_paint_isolation_keeps_art_named_like_its_parts() {
    use vectorcraft_tools::builder::{SOURCES_NAME, faces, sources};
    let mut s = session();
    let g = live(&mut s);
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    let text = NodeId(s.execute("text.create", &json!({"x": 20, "y": 20, "text": "Hi"})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.setProps", &json!({"id": text.0, "name": SOURCES_NAME})).unwrap();
    let (a, b) = (line_across(&mut s, 30.0), line_across(&mut s, 70.0));
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let paths = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.setProps", &json!({"id": paths.0, "name": "Face"})).unwrap();
    s.execute("object.exitIsolation", &json!({})).unwrap();
    let n = node(&s, g);
    assert_eq!(faces(&n).len(), 9, "the group named Face joined");
    assert_eq!(s.doc().unwrap().doc.parent_of(paths), Some(sources(&n).unwrap().id));
    s.execute("livePaint.gapOptions", &json!({"ids": [g.0], "detect": true})).unwrap();
    assert_eq!(s.doc().unwrap().doc.parent_of(text), Some(g), "Gap Options keeps the type named like the sources");
}

/// Type drawn in an isolated Live Paint group under the name of a face, an edge or the sources
/// group is not one of the group's parts: Gap Options, Info and Make count only the faces and edges
/// Live Paint made, and Expand keeps the type.
#[test]
fn live_paint_does_not_count_art_named_like_its_parts() {
    use vectorcraft_tools::builder::{EDGE_NAME, FACE_NAME, SOURCES_NAME};
    let mut s = session();
    let g = live(&mut s);
    let edges = s.execute("livePaint.info", &json!({"group": g.0})).unwrap()["edges"].as_array().unwrap().len();
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    let mut texts = vec![];
    for (i, name) in [FACE_NAME, EDGE_NAME, SOURCES_NAME].into_iter().enumerate() {
        let r = s.execute("text.create", &json!({"x": 20 + 20 * i, "y": 20, "text": "Hi"})).unwrap();
        let t = NodeId(r["id"].as_u64().unwrap());
        s.execute("object.setProps", &json!({"id": t.0, "name": name})).unwrap();
        texts.push(t);
    }
    s.execute("object.exitIsolation", &json!({})).unwrap();
    let r = s.execute("livePaint.gapOptions", &json!({"ids": [g.0], "detect": true})).unwrap();
    assert_eq!(r["faces"], 3, "Gap Options counts the faces Live Paint made");
    let info = s.execute("livePaint.info", &json!({"group": g.0})).unwrap();
    assert_eq!(info["faces"].as_array().unwrap().len(), 3);
    assert_eq!(info["edges"].as_array().unwrap().len(), edges);
    let r = s.execute("livePaint.make", &json!({"ids": [g.0]})).unwrap();
    assert_eq!((r["faces"].as_u64(), r["edges"].as_u64()), (Some(3), Some(edges as u64)));
    s.execute("livePaint.expand", &json!({"ids": [g.0]})).unwrap();
    let d = &s.doc().unwrap().doc;
    for t in texts {
        assert_eq!(d.parent_of(t), Some(g), "Expand keeps {t}");
    }
}

/// With Real-time Drawing and Editing off, a rectangle still being drawn in a Live Paint group
/// isolated in a pattern tile is drawn first when Exit Isolation Mode leaves pattern editing, and
/// joins the group with the tile saved.
#[test]
fn live_paint_in_a_pattern_tile_adds_a_shape_still_being_drawn_on_exit() {
    use vectorcraft_tools::builder::{is_live_paint, sources};
    let mut s = session();
    two(&mut s);
    s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
    let g = NodeId(s.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    s.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": false})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("rectangle", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 10.0, 10.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 40.0, 40.0), v).unwrap();
    assert!(s.interaction_unapplied());
    s.execute("object.exitIsolation", &json!({})).unwrap();
    let st = s.doc().unwrap();
    assert!(st.doc.pattern_edit.is_none());
    let art = &st.doc.pattern("P").unwrap().art;
    let n = art.iter().find(|n| is_live_paint(n)).expect("the tile holds the Live Paint group");
    assert_eq!(sources(n).unwrap().children().unwrap().len(), 3, "the rectangle joined the sources");
}

/// Isolating a level outside an isolated Live Paint group, as the isolation bar's back arrow does,
/// leaves the group and adds the paths drawn in it; isolating a path drawn in it does not.
#[test]
fn live_paint_isolation_adds_drawn_paths_when_isolating_a_level_outside() {
    use vectorcraft_tools::builder::{faces, sources};
    let mut s = session();
    let g = live(&mut s);
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    let a = line_across(&mut s, 50.0);
    s.execute("object.isolate", &json!({"id": a.0})).unwrap();
    assert_eq!(s.doc().unwrap().doc.parent_of(a), Some(g), "the path is still being drawn in the group");
    assert_eq!(faces(&node(&s, g)).len(), 3);
    let layer = s.doc().unwrap().doc.parent_of(g).unwrap();
    s.execute("object.isolate", &json!({"id": layer.0})).unwrap();
    assert_eq!(s.doc().unwrap().isolation, Some(layer));
    let n = node(&s, g);
    assert_eq!(faces(&n).len(), 6);
    assert_eq!(s.doc().unwrap().doc.parent_of(a), Some(sources(&n).unwrap().id));
}

/// A group of paths drawn in an isolated Live Paint group and then isolated itself joins the Live
/// Paint group when isolation mode ends, or when a level outside the Live Paint group is isolated.
#[test]
fn live_paint_isolation_adds_a_drawn_group_isolated_inside_it() {
    use vectorcraft_tools::builder::{faces, sources};
    for to_layer in [false, true] {
        let mut s = session();
        let g = live(&mut s);
        s.execute("object.isolate", &json!({"id": g.0})).unwrap();
        let (a, b) = (line_across(&mut s, 30.0), line_across(&mut s, 70.0));
        s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
        let paths = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
        s.execute("object.isolate", &json!({"id": paths.0})).unwrap();
        assert_eq!(faces(&node(&s, g)).len(), 3, "isolating the drawn group stays in the Live Paint group");
        if to_layer {
            let layer = s.doc().unwrap().doc.parent_of(g).unwrap();
            s.execute("object.isolate", &json!({"id": layer.0})).unwrap();
        } else {
            s.execute("object.exitIsolation", &json!({})).unwrap();
        }
        let n = node(&s, g);
        assert_eq!(faces(&n).len(), 9, "the drawn group joined (to the layer: {to_layer})");
        assert_eq!(s.doc().unwrap().doc.parent_of(paths), Some(sources(&n).unwrap().id));
    }
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

/// A Gap Preview Color with an alpha part (`#rrggbbaa` or `#rgba`) is refused and changes nothing;
/// `#rrggbb` and a layer color's name still set it.
#[test]
fn a_gap_color_with_an_alpha_part_is_refused() {
    let mut s = session();
    let g = live_square_with_a_gap(&mut s);
    let depth = s.doc().unwrap().history.undo.len();
    for bad in ["#00ff0080", "00ff0080", "#0f08"] {
        let err = s.execute("livePaint.gapOptions", &json!({"id": g.0, "color": bad})).unwrap_err().to_string();
        assert!(err.contains(&format!("`{bad}` is not a colour (#rrggbb or a preset name)")), "{bad}: {err}");
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), depth);
    assert_eq!(gap_options(&s, g).color, [0xff, 0x4f, 0x4f]);
    s.execute("livePaint.gapOptions", &json!({"id": g.0, "color": "#00ff00"})).unwrap();
    assert_eq!(gap_options(&s, g).color, [0, 0xff, 0]);
    s.execute("livePaint.gapOptions", &json!({"id": g.0, "color": "Green"})).unwrap();
    assert_eq!(gap_options(&s, g).color, [0x4f, 0xff, 0x4f]);
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
fn live_paint_bucket_drag_fills_multiple_faces() {
    let mut s = session();
    let g = live(&mut s);
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.fill = Paint::solid(Color::rgb(0.0, 1.0, 1.0));
    let before = s.doc().unwrap().doc.clone();
    s.pointer(&PointerEvent::new(PointerKind::Down, 25.0, 50.0), v).unwrap();
    // Skip the middle face between pointer events, as can happen in a fast drag.
    s.pointer(&PointerEvent::new(PointerKind::Drag, 125.0, 50.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 125.0, 50.0), v).unwrap();
    let gn = node(&s, g);
    for x in [25.0, 75.0, 125.0] {
        let f = vectorcraft_tools::builder::face_at(&gn, Point::new(x, 50.0)).unwrap();
        assert_eq!(f.appearance.fill_paint(), s.paint.fill, "dragged face at {x}");
    }
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, before, "one Undo restores every face");
    s.pointer(&PointerEvent::new(PointerKind::Move, 125.0, 50.0), v).unwrap();
    assert!(!s.overlays(v).is_empty(), "hover still highlights a face");
    assert_eq!(s.doc().unwrap().doc, before, "hover does not paint");
}

#[test]
fn live_paint_bucket_drag_crosses_thin_grid_cells_and_can_cancel() {
    for (deferred, cancel) in [(false, false), (false, true), (true, false)] {
        let mut s = session();
        let ids = [rect(&mut s, 0.0, 0.0, 150.0, 100.0), line(&mut s, (50.0, 0.0), (50.0, 100.0)), line(&mut s, (50.25, 0.0), (50.25, 100.0))];
        s.execute("select.set", &json!({"ids": ids.map(|i| i.0)})).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let v = ViewInfo::default();
        s.select_tool("livePaintBucket", v).unwrap();
        s.prefs.real_time_drawing = !deferred;
        s.paint.fill = Paint::solid(Color::rgb(0.0, 1.0, 1.0));
        s.pointer(&PointerEvent::new(PointerKind::Down, 25.0, 50.0), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Drag, 125.0, 50.0), v).unwrap();
        if cancel {
            assert!(s.tool_busy());
            assert!(s.tool_claims_key(ToolKey::Escape, v));
            s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
            assert!(!s.tool_busy());
            assert!(!s.tool_claims_key(ToolKey::Escape, v));
        }
        s.pointer(&PointerEvent::new(PointerKind::Up, 125.0, 50.0), v).unwrap();
        if !cancel {
            let g = s.doc().unwrap().selection.objects[0];
            let gn = node(&s, g);
            for x in [25.0, 50.125, 125.0] {
                let f = vectorcraft_tools::builder::face_at(&gn, Point::new(x, 50.0)).unwrap();
                assert_eq!(f.appearance.fill_paint(), s.paint.fill, "deferred={deferred}, x={x}");
            }
            s.execute("edit.undo", &json!({})).unwrap();
        }
        assert!(s.doc().unwrap().doc == before, "deferred={deferred}, cancel={cancel}: original paths and selection art must be restored");
    }
}

#[test]
fn live_paint_bucket_long_drag_keeps_late_turns_and_release_as_one_undo() {
    for cancel in [false, true] {
        let mut s = session();
        let ids = [rect(&mut s, 0.0, 0.0, 150.0, 100.0), line(&mut s, (50.0, 0.0), (50.0, 100.0)), line(&mut s, (100.0, 0.0), (100.0, 100.0))];
        s.execute("select.set", &json!({"ids": ids.map(|id| id.0)})).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let v = ViewInfo::default();
        s.select_tool("livePaintBucket", v).unwrap();
        s.prefs.real_time_drawing = false;
        s.paint.fill = Paint::solid(Color::rgb(0.0, 1.0, 1.0));
        s.pointer(&PointerEvent::new(PointerKind::Down, 25.0, 50.0), v).unwrap();
        for i in 1..4096 {
            s.pointer(&PointerEvent::new(PointerKind::Drag, 25.0 + f64::from(i) / 10000.0, 50.0), v).unwrap();
        }
        // Turn around the middle face after the chunk limit, then enter the right face on Up.
        for (x, y) in [(25.0, 150.0), (125.0, 150.0)] {
            s.pointer(&PointerEvent::new(PointerKind::Drag, x, y), v).unwrap();
        }
        if cancel {
            s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
        }
        s.pointer(&PointerEvent::new(PointerKind::Up, 125.0, 50.0), v).unwrap();
        if !cancel {
            let g = s.doc().unwrap().selection.objects[0];
            let gn = node(&s, g);
            for x in [25.0, 125.0] {
                assert_eq!(vectorcraft_tools::builder::face_at(&gn, Point::new(x, 50.0)).unwrap().appearance.fill_paint(), s.paint.fill);
            }
            assert_eq!(vectorcraft_tools::builder::face_at(&gn, Point::new(75.0, 50.0)).unwrap().appearance.fill_paint(), Paint::solid(Color::WHITE));
            s.execute("edit.undo", &json!({})).unwrap();
        }
        assert!(s.doc().unwrap().doc == before, "long drag is one cancellable undo step");
    }
}

#[test]
fn live_paint_bucket_drag_rejects_invalid_points_atomically() {
    let mut s = session();
    let g = live(&mut s);
    let before = s.doc().unwrap().doc.clone();
    for points in
        [json!([]), json!([[26.0, 50.0]]), json!([[25.0, 50.0], [null, 0]]), json!([[25.0, 50.0], [1e30, 0]]), json!(vec![[25.0, 50.0]; 4097])]
    {
        assert!(s.execute("livePaint.fill", &json!({"group": g.0, "point": [25.0, 50.0], "points": points})).is_err());
        assert_eq!(s.doc().unwrap().doc, before);
    }
    for continuation in [json!(null), json!([[[26.0, 50.0], [125.0, 50.0]]]), json!([vec![[25.0, 50.0]; 4097]])] {
        assert!(
            s.execute("livePaint.fill", &json!({"group": g.0, "point": [25.0, 50.0], "points": [[25.0, 50.0]], "continuation": continuation}))
                .is_err()
        );
        assert_eq!(s.doc().unwrap().doc, before);
    }
}

/// The edge of `g` through (`x`, 25), halfway up the squares of [`live_squares`].
fn vertical_edge(s: &Session, g: NodeId, x: f64) -> NodeId {
    vectorcraft_tools::builder::edge_near(&node(s, g), Point::new(x, 25.0), 0.5).map(|e| e.id).unwrap_or_else(|| panic!("an edge at x {x}"))
}

/// A Shift-drag from an edge strokes every visible edge of that group the drag crosses, between
/// pointer events and on the release too, as one undo step that Escape cancels; hidden edges and
/// another group's edges keep their strokes.
#[test]
fn live_paint_bucket_drag_strokes_crossed_edges_as_one_undo() {
    for cancel in [false, true] {
        let mut s = session();
        let g = live_squares(&mut s, &[0.0, 50.0, 100.0, 150.0, 200.0]);
        let other = live_squares(&mut s, &[300.0]);
        let black = Paint::solid(Color::rgb(0.0, 0.0, 0.0));
        let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
        let all: Vec<NodeId> = [g, other].iter().flat_map(|g| edges_in(&s, *g, Rect::new(-1.0, -1.0, 351.0, 51.0))).collect();
        paint_parts(&mut s, &all, &black, &[]);
        let hidden = vertical_edge(&s, g, 100.0);
        s.edit("t", |d, _| {
            d.node_mut(hidden).unwrap().visible = false;
            Ok(())
        })
        .unwrap();
        let before = s.doc().unwrap().doc.clone();
        let depth = s.doc().unwrap().history.undo.len();
        let v = ViewInfo::default();
        s.select_tool("livePaintBucket", v).unwrap();
        s.paint.stroke = green.clone();
        s.paint.stroke_width = 3.0;
        let shift = Mods { shift: true, ..Default::default() };
        s.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 25.0).with_mods(shift), v).unwrap();
        // Past the hidden edge and the edge at 150, then back across it, between pointer events.
        s.pointer(&PointerEvent::new(PointerKind::Drag, 160.0, 25.0).with_mods(shift), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Drag, 140.0, 30.0).with_mods(shift), v).unwrap();
        if cancel {
            assert!(s.tool_busy());
            s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
            assert!(!s.tool_busy());
        }
        // The release crosses the edges at 200 and 250 and goes on into the other group.
        s.pointer(&PointerEvent::new(PointerKind::Up, 325.0, 25.0).with_mods(shift), v).unwrap();
        if cancel {
            assert!(s.doc().unwrap().doc == before, "Escape cancels the whole drag");
            assert_eq!(s.doc().unwrap().history.undo.len(), depth);
            continue;
        }
        let stroke = |s: &Session, id: NodeId| {
            let n = node(s, id);
            (n.appearance.stroke_paint(), n.appearance.stroke().map(|st| st.width))
        };
        for x in [50.0, 150.0, 200.0, 250.0] {
            assert_eq!(stroke(&s, vertical_edge(&s, g, x)), (green.clone(), Some(3.0)), "edge at {x}");
        }
        for (group, x) in [(g, 0.0), (g, 100.0), (other, 300.0), (other, 350.0)] {
            assert_eq!(stroke(&s, vertical_edge(&s, group, x)), (black.clone(), Some(1.0)), "edge at {x}");
        }
        for e in edges_in(&s, g, Rect::new(-1.0, -1.0, 201.0, 1.0)) {
            assert_eq!(stroke(&s, e).0, black, "a top edge is not crossed");
        }
        assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1, "one undo step");
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(s.doc().unwrap().doc == before, "one Undo restores every edge");
    }
}

/// A stroke drag over edges that already have the stroke records no undo step and keeps the redo
/// steps.
#[test]
fn live_paint_bucket_stroke_drag_no_op_keeps_history() {
    let mut s = session();
    let g = live_squares(&mut s, &[0.0, 50.0]);
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    let all = edges_in(&s, g, Rect::new(-1.0, -1.0, 101.0, 51.0));
    paint_parts(&mut s, &all, &green, &[]);
    paint_parts(&mut s, &all, &green, &[]);
    s.execute("edit.undo", &json!({})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let (undo, redo) = (s.doc().unwrap().history.undo.len(), s.doc().unwrap().history.redo.len());
    assert_eq!(redo, 1);
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.paint.stroke = green;
    s.paint.stroke_width = 1.0;
    let shift = Mods { shift: true, ..Default::default() };
    s.pointer(&PointerEvent::new(PointerKind::Down, 0.0, 25.0).with_mods(shift), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 60.0, 25.0).with_mods(shift), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 110.0, 25.0).with_mods(shift), v).unwrap();
    assert!(s.doc().unwrap().doc == before);
    assert_eq!((s.doc().unwrap().history.undo.len(), s.doc().unwrap().history.redo.len()), (undo, redo));
}

/// `livePaint.strokeEdge` with `points` paints the crossed edges, counting the edge at the point;
/// a malformed path or a scope other than "one" with `points` is an error that changes nothing.
#[test]
fn live_paint_stroke_drag_rejects_invalid_points_atomically() {
    let mut s = session();
    let g = live_squares(&mut s, &[0.0, 50.0, 100.0]);
    let before = s.doc().unwrap().doc.clone();
    let stroke = |s: &mut Session, extra: Value| {
        let mut p = json!({"group": g.0, "point": [50.0, 25.0], "color": "#00ff00"});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        s.execute("livePaint.strokeEdge", &p)
    };
    for points in
        [json!([]), json!([[51.0, 25.0]]), json!([[50.0, 25.0], [null, 0]]), json!([[50.0, 25.0], [1e30, 0]]), json!(vec![[50.0, 25.0]; 4097])]
    {
        assert!(stroke(&mut s, json!({ "points": points })).is_err(), "{points}");
        assert_eq!(s.doc().unwrap().doc, before);
    }
    for continuation in [json!(null), json!([[[51.0, 25.0], [125.0, 25.0]]]), json!([vec![[50.0, 25.0]; 4097]])] {
        assert!(stroke(&mut s, json!({ "points": [[50.0, 25.0]], "continuation": continuation })).is_err(), "{continuation}");
        assert_eq!(s.doc().unwrap().doc, before);
    }
    assert!(stroke(&mut s, json!({ "continuation": [[[50.0, 25.0]]] })).is_err());
    for scope in ["connected", "samePaint"] {
        assert!(stroke(&mut s, json!({ "points": [[50.0, 25.0], [125.0, 25.0]], "scope": scope })).is_err(), "{scope}");
    }
    assert_eq!(s.doc().unwrap().doc, before);
    // A path in two chunks reaches the edges at 50, 100 and 150, not the one at 0.
    let r = stroke(&mut s, json!({ "points": [[50.0, 25.0], [90.0, 25.0]], "continuation": [[[90.0, 25.0], [160.0, 25.0]]] })).unwrap();
    assert_eq!(r["count"], 3);
    let green = Paint::solid(Color::from_hex("#00ff00").unwrap());
    for x in [50.0, 100.0, 150.0] {
        assert_eq!(node(&s, vertical_edge(&s, g, x)).appearance.stroke_paint(), green, "edge at {x}");
    }
    assert_ne!(node(&s, vertical_edge(&s, g, 0.0)).appearance.stroke_paint(), green);
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
    // A Live Paint Selection click selects the face.
    s.select_tool("livePaintSelection", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 125.0, 50.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 125.0, 50.0), v).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, vec![f.id]);
}

/// `tool.setOption` refuses an invalid value for the Live Paint tools, naming its key, whether the
/// tool is active or not and in the `key`/`value` and `values` forms: a switch that is not true or
/// false, a color that is not an RGB color (an alpha part included), a width that is not a number
/// and an option the tool does not have. Nothing is set, also from the valid values sent with it,
/// and the saved settings stay as they were.
#[test]
fn live_paint_tool_options_refuse_invalid_values_and_change_nothing() {
    let colors = [json!(3), json!("nope"), json!("#12345"), json!("#00ff0080"), json!("00ff0080"), json!("#0f08"), json!(null)];
    let widths = [json!("wide"), json!(null), json!(true)];
    let switches = [json!("true"), json!(null), json!(1)];
    for (tool, keys) in [
        ("livePaintBucket", &["paintFills", "paintStrokes", "cursorSwatchPreview", "highlight"][..]),
        ("livePaintSelection", &["selectFills", "selectStrokes"][..]),
    ] {
        let mut bad: Vec<(&str, Value)> = keys.iter().flat_map(|k| switches.iter().map(move |v| (*k, v.clone()))).collect();
        bad.extend(colors.iter().map(|v| ("color", v.clone())));
        bad.extend(widths.iter().map(|v| ("width", v.clone())));
        bad.push(("colour", json!("#00ff00")));
        for active in [false, true] {
            let mut s = session();
            if active {
                s.select_tool(tool, ViewInfo::default()).unwrap();
            }
            let (options, prefs) = (s.tool_options_of(tool), s.prefs.clone());
            for (key, v) in &bad {
                let mut values = serde_json::Map::new();
                values.insert("width".into(), json!(9));
                values.insert(key.to_string(), v.clone());
                for p in [json!({"tool": tool, "values": values}), json!({"tool": tool, "key": key, "value": v})] {
                    let e = s.set_tool_option_cmd(&p).unwrap_err();
                    assert!(e.contains(&format!("`{key}`")), "{tool} {p}: {e}");
                    assert_eq!((s.tool_options_of(tool), &s.prefs), (options.clone(), &prefs), "{tool} {p}");
                }
            }
            // A `key` left without a `value` is an invalid value too.
            assert!(s.set_tool_option_cmd(&json!({"tool": tool, "key": "color"})).is_err());
            s.select_tool(tool, ViewInfo::default()).unwrap();
            assert_eq!(s.tool_options_of(tool), options, "{tool}");
        }
    }
}

/// The Live Paint tools' options take each color form they took before, and a width out of range
/// is clamped to 1–20 rather than refused; the values are saved with the preferences.
#[test]
fn live_paint_tool_options_take_rgb_colors_and_clamp_the_width() {
    let mut s = session();
    for tool in ["livePaintBucket", "livePaintSelection"] {
        for (color, hex) in [("#00FF00", "#00ff00"), ("#0f0", "#00ff00"), ("0000ff", "#0000ff"), ("light blue", "#4f80ff"), ("white", "#ffffff")] {
            let o = s.set_tool_option_cmd(&json!({"tool": tool, "key": "color", "value": color})).unwrap();
            assert_eq!(o["color"], hex, "{tool} {color}");
        }
        for (width, set) in [(100.0, 20.0), (0.0, 1.0), (-3.0, 1.0), (7.5, 7.5)] {
            let o = s.set_tool_option_cmd(&json!({"tool": tool, "values": {"width": width}})).unwrap();
            assert_eq!(o["width"], set, "{tool} {width}");
        }
        assert_eq!(s.prefs.tool_settings[tool]["width"], 7.5);
        assert_eq!(s.prefs.tool_settings[tool]["color"], "#ffffff");
    }
    let o = s.set_tool_option_cmd(&json!({"tool": "livePaintBucket", "values": {"paintFills": false, "highlight": false}})).unwrap();
    assert_eq!((&o["paintFills"], &o["highlight"]), (&json!(false), &json!(false)));
}

/// Saved Live Paint settings that hold an invalid value still load: the tool keeps its default for
/// that option and takes the valid ones.
#[test]
fn live_paint_tools_load_saved_settings_with_invalid_values() {
    let mut s = session();
    let saved = json!({"paintFills": "yes", "highlight": false, "width": "wide", "color": "nope", "paintStrokes": true});
    s.prefs.tool_settings.insert("livePaintBucket".into(), saved.as_object().unwrap().clone());
    s.select_tool("livePaintBucket", ViewInfo::default()).unwrap();
    let o = s.tool_options_of("livePaintBucket");
    assert_eq!(
        (&o["paintFills"], &o["paintStrokes"], &o["highlight"], &o["width"], &o["color"]),
        (&json!(true), &json!(true), &json!(false), &json!(4.0), &json!("#ff2a2a"))
    );
}

/// `tool.setOption` refuses an invalid value for the Shape Builder, naming its key, whether the
/// tool is active or not and in the `key`/`value` and `values` forms: a switch that is not true or
/// false, a `pickColorFrom` or `selection` that is not one of its names, a `gapLength` that is not
/// a number, a `highlightColor` that is not an RGB color (an alpha part included) and an option the
/// tool does not have. Nothing is set, also from the valid values sent with it, and the saved
/// settings stay as they were.
#[test]
fn shape_builder_tool_options_refuse_invalid_values_and_change_nothing() {
    let tool = "shapeBuilder";
    let switches = ["openFilledAsClosed", "cursorSwatchPreview", "highlightFill", "gapDetection", "clickStrokeSplitsPath", "highlightStroke"];
    let mut bad: Vec<(&str, Value)> = switches.iter().flat_map(|k| [json!("true"), json!(null), json!(1)].map(move |v| (*k, v))).collect();
    bad.extend([json!("Swatches"), json!(true), json!(null), json!("")].map(|v| ("pickColorFrom", v)));
    bad.extend([json!("straight"), json!(1), json!(null)].map(|v| ("selection", v)));
    bad.extend([json!("wide"), json!(null), json!(true)].map(|v| ("gapLength", v)));
    let colors = [json!(3), json!("nope"), json!("#12345"), json!("#00ff0080"), json!("00ff0080"), json!("#0f08"), json!(null)];
    bad.extend(colors.map(|v| ("highlightColor", v)));
    bad.push(("color", json!("#00ff00")));
    for active in [false, true] {
        let mut s = session();
        if active {
            s.select_tool(tool, ViewInfo::default()).unwrap();
        }
        let (options, prefs) = (s.tool_options_of(tool), s.prefs.clone());
        for (key, v) in &bad {
            let mut values = serde_json::Map::new();
            values.insert("gapLength".into(), json!(9));
            values.insert(key.to_string(), v.clone());
            for p in [json!({"tool": tool, "values": values}), json!({"tool": tool, "key": key, "value": v})] {
                let e = s.set_tool_option_cmd(&p).unwrap_err();
                assert!(e.contains(&format!("`{key}`")), "{p}: {e}");
                assert_eq!((s.tool_options_of(tool), &s.prefs), (options.clone(), &prefs), "{p}");
            }
        }
        s.select_tool(tool, ViewInfo::default()).unwrap();
        assert_eq!(s.tool_options_of(tool), options);
    }
}

/// The Shape Builder's options take each value they took before: the names of `pickColorFrom` and
/// `selection`, each RGB color form, and a gap length out of range clamped rather than refused.
#[test]
fn shape_builder_tool_options_take_valid_values_and_clamp_the_gap() {
    let mut s = session();
    let tool = "shapeBuilder";
    for (color, hex) in [("#00FF00", "#00ff00"), ("#0f0", "#00ff00"), ("0000ff", "#0000ff"), ("light blue", "#4f80ff"), ("white", "#ffffff")] {
        let o = s.set_tool_option_cmd(&json!({"tool": tool, "key": "highlightColor", "value": color})).unwrap();
        assert_eq!(o["highlightColor"], hex, "{color}");
    }
    for (gap, set) in [(1000.0, vectorcraft_tools::builder::SHAPE_BUILDER_MAX_GAP), (-3.0, 0.0), (7.5, 7.5)] {
        let o = s.set_tool_option_cmd(&json!({"tool": tool, "values": {"gapLength": gap}})).unwrap();
        assert_eq!(o["gapLength"], set, "{gap}");
    }
    let o = s
        .set_tool_option_cmd(&json!({"tool": tool, "values": {"pickColorFrom": "swatches", "selection": "straightLine", "gapDetection": true}}))
        .unwrap();
    assert_eq!((&o["pickColorFrom"], &o["selection"], &o["gapDetection"]), (&json!("swatches"), &json!("straightLine"), &json!(true)));
    let o = s.set_tool_option_cmd(&json!({"tool": tool, "values": {"pickColorFrom": "artwork", "selection": "freeform"}})).unwrap();
    assert_eq!((&o["pickColorFrom"], &o["selection"]), (&json!("artwork"), &json!("freeform")));
    assert_eq!(s.prefs.tool_settings[tool]["gapLength"], 7.5);
}

#[test]
fn live_paint_bucket_alt_sample_paints_next_face_without_changing_selection() {
    let mut s = session();
    let g = live(&mut s);
    let src = rect(&mut s, 300.0, 0.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#0000ff"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 12})).unwrap();
    let dst = rect(&mut s, 500.0, 0.0, 50.0, 50.0);
    s.execute("paint.default", &json!({})).unwrap();
    let before = (s.doc().unwrap().doc.clone(), s.doc().unwrap().selection.clone());
    let face_before = vectorcraft_tools::builder::face_at(&node(&s, g), Point::new(125.0, 50.0)).unwrap().appearance.fill_paint();
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 350.0, 50.0).with_mods(Mods { alt: true, ..Default::default() }), v).unwrap();
    assert_eq!(s.paint.fill.color(), Some(Color::rgb8(255, 0, 0)));
    assert_eq!(s.paint.stroke.color(), Some(Color::rgb8(0, 0, 255)));
    assert_eq!(s.paint.stroke_width, 12.0);
    assert_eq!((s.doc().unwrap().doc.clone(), s.doc().unwrap().selection.clone()), before);
    assert_eq!(node(&s, dst).appearance.fill_paint().color(), Some(Color::WHITE));
    assert_eq!(node(&s, src).appearance.fill_paint().color(), Some(Color::rgb8(255, 0, 0)));
    assert_eq!(vectorcraft_tools::builder::face_at(&node(&s, g), Point::new(125.0, 50.0)).unwrap().appearance.fill_paint(), face_before);
    s.pointer(&PointerEvent::new(PointerKind::Down, 125.0, 50.0), v).unwrap();
    assert_eq!(
        vectorcraft_tools::builder::face_at(&node(&s, g), Point::new(125.0, 50.0)).unwrap().appearance.fill_paint().color(),
        Some(Color::rgb8(255, 0, 0))
    );
}

#[test]
fn live_paint_bucket_alt_image_samples_fill_and_clears_stroke() {
    let mut s = session();
    let img = image::RgbaImage::from_raw(1, 1, vec![0, 255, 0, 255]).unwrap();
    let mut png = vec![];
    {
        let __dyn: image::DynamicImage = img.into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
    s.edit("Place", |d, _| {
        d.images.insert("px".into(), ImageBlob::png(png));
        let id = d.alloc_id();
        let im = ImageObject {
            key: "px".into(),
            width: 1,
            height: 1,
            xf: Affine::translate((300.0, 0.0)) * Affine::scale(100.0),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        d.insert(d.default_layer(), 0, Node::new(id, NodeKind::Image(Box::new(im)))).map_err(|e| EngineError::Other(e.to_string()))
    })
    .unwrap();
    let dst = rect(&mut s, 500.0, 0.0, 50.0, 50.0);
    s.execute("paint.setStroke", &json!({"color": "#0000ff"})).unwrap();
    let before = (s.doc().unwrap().doc.clone(), s.doc().unwrap().selection.clone());
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 350.0, 50.0).with_mods(Mods { alt: true, ..Default::default() }), v).unwrap();
    assert_eq!(s.paint.fill.color(), Some(Color::rgb8(0, 255, 0)));
    assert!(s.paint.stroke.is_none());
    assert_eq!((s.doc().unwrap().doc.clone(), s.doc().unwrap().selection.clone()), before);
    assert_eq!(node(&s, dst).appearance.stroke_paint().color(), Some(Color::rgb8(0, 0, 255)));
}

#[test]
fn live_paint_bucket_alt_shift_gradient_samples_active_stroke_only() {
    let mut s = session();
    let src = rect(&mut s, 300.0, 0.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]}})).unwrap();
    let dst = rect(&mut s, 500.0, 0.0, 50.0, 50.0);
    s.execute("paint.setStroke", &json!({"color": "#ffffff"})).unwrap();
    let fill = s.paint.fill.clone();
    let before = (s.doc().unwrap().doc.clone(), s.doc().unwrap().selection.clone());
    let v = ViewInfo::default();
    s.select_tool("livePaintBucket", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 350.0, 50.0).with_mods(Mods { alt: true, shift: true, ..Default::default() }), v).unwrap();
    let sampled = s.paint.stroke.color().unwrap().to_rgb();
    assert!(sampled[0] > 0.0 && sampled[2] > 0.0 && sampled[1] < 0.01, "{sampled:?}");
    assert_eq!(s.paint.fill, fill);
    assert_eq!((s.doc().unwrap().doc.clone(), s.doc().unwrap().selection.clone()), before);
    assert_eq!(node(&s, dst).appearance.stroke_paint().color(), Some(Color::WHITE));
    assert!(matches!(node(&s, src).appearance.fill_paint(), Paint::Gradient(_)));
}

#[test]
fn live_paint_selection_double_click_selects_the_faces_no_stroked_edge_parts() {
    let mut s = session();
    let g = live(&mut s);
    // The edge between the middle and right faces draws no stroke.
    s.execute("livePaint.strokeEdge", &json!({"group": g.0, "point": [100.0, 50.0], "none": true})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("livePaintSelection", v).unwrap();
    let gn = node(&s, g);
    let face = |x: f64| vectorcraft_tools::builder::face_at(&gn, Point::new(x, 50.0)).unwrap().id;
    let before = s.doc().unwrap().doc.clone();
    let double_click = |s: &mut Session, x: f64, m: Mods| {
        for kind in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick] {
            s.pointer(&PointerEvent::new(kind, x, 50.0).with_mods(m), v).unwrap();
        }
        let mut ids = s.doc().unwrap().selection.objects.clone();
        ids.sort();
        ids
    };
    let sorted = |mut ids: Vec<NodeId>| {
        ids.sort();
        ids
    };
    assert_eq!(double_click(&mut s, 125.0, Mods::default()), sorted(vec![face(75.0), face(125.0)]));
    // The stroked edge at x = 50 parts the left face from the others.
    assert_eq!(double_click(&mut s, 25.0, Mods::default()), vec![face(25.0)]);
    // Shift-double-click adds the faces reached to the selection.
    assert_eq!(double_click(&mut s, 75.0, Mods { shift: true, ..Default::default() }), sorted(vec![face(25.0), face(75.0), face(125.0)]));
    assert_eq!(s.doc().unwrap().doc, before, "selecting changes no paint");
}

#[test]
fn live_paint_selection_marquee_selects_what_it_touches() {
    let mut s = session();
    let g = live(&mut s);
    let v = ViewInfo::default();
    s.select_tool("livePaintSelection", v).unwrap();
    let drag = |s: &mut Session, from: (f64, f64), to: (f64, f64), m: Mods, escape: bool| {
        s.pointer(&PointerEvent::new(PointerKind::Down, from.0, from.1).with_mods(m), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Drag, to.0, to.1).with_mods(m), v).unwrap();
        if escape {
            s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
        }
        s.pointer(&PointerEvent::new(PointerKind::Up, to.0, to.1).with_mods(m), v).unwrap();
        s.doc().unwrap().selection.objects.clone()
    };
    let gn = node(&s, g);
    let face = |x: f64| vectorcraft_tools::builder::face_at(&gn, Point::new(x, 50.0)).unwrap().id;
    let edge = |x: f64| vectorcraft_tools::builder::edge_near(&gn, Point::new(x, 50.0), 0.5).unwrap().id;
    let shift = Mods { shift: true, ..Default::default() };
    // Across the left side: the left face and that side.
    assert_eq!(drag(&mut s, (-10.0, 40.0), (40.0, 60.0), Mods::default(), false), vec![face(25.0), edge(0.0)]);
    // Shift-drag across the right side adds the right face and that side.
    assert_eq!(drag(&mut s, (160.0, 40.0), (140.0, 60.0), shift, false), vec![face(25.0), edge(0.0), face(125.0), edge(150.0)]);
    // Shift-drag across the left side again takes those out.
    assert_eq!(drag(&mut s, (-10.0, 40.0), (40.0, 60.0), shift, false), vec![face(125.0), edge(150.0)]);
    // Escape during the drag, with or without Shift, from a press on a face or outside the group:
    // the selection stays as it was.
    for (from, m) in [((-10.0, 40.0), shift), ((25.0, 50.0), Mods::default()), ((25.0, 50.0), shift)] {
        assert_eq!(drag(&mut s, from, (40.0, 60.0), m, true), vec![face(125.0), edge(150.0)], "{from:?} {m:?}");
    }
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

/// The Shape Builder takes a Live Paint face or edge when any of its fill or stroke rows draws,
/// whichever row is on top: a painted lower row under an unpainted top row counts, and a part
/// whose only painted row is hidden, transparent or (for an edge) 0 pt wide, or whose own opacity
/// is 0, does not.
#[test]
fn shape_builder_takes_the_live_paint_faces_and_edges_that_draw() {
    use vectorcraft_doc::{FillLayer, StrokeLayer};
    let fill = |p: Paint| AppearanceItem::Fill(FillLayer::new(p));
    let stroke = |p: Paint, w: f64| AppearanceItem::Stroke(StrokeLayer::new(p, w));
    let red = || Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let hidden = |mut i: AppearanceItem| {
        match &mut i {
            AppearanceItem::Fill(f) => f.visible = false,
            AppearanceItem::Stroke(st) => st.visible = false,
        }
        i
    };
    let clear = |mut i: AppearanceItem| {
        match &mut i {
            AppearanceItem::Fill(f) => f.opacity = 0.0,
            AppearanceItem::Stroke(st) => st.opacity = 0.0,
        }
        i
    };
    let mut s = session();
    let g = live(&mut s);
    let lower_fill = set_live_part(&mut s, g, Some((25.0, 50.0)), None, vec![fill(red()), fill(Paint::None)], 1.0);
    let hidden_fill = set_live_part(&mut s, g, Some((125.0, 50.0)), None, vec![hidden(fill(red()))], 1.0);
    let clear_fill = set_live_part(&mut s, g, Some((75.0, 50.0)), None, vec![clear(fill(red()))], 1.0);
    let lower_stroke = set_live_part(&mut s, g, None, Some((0.0, 50.0)), vec![stroke(red(), 2.0), stroke(Paint::None, 1.0)], 1.0);
    let hidden_stroke = set_live_part(&mut s, g, None, Some((150.0, 50.0)), vec![hidden(stroke(red(), 2.0))], 1.0);
    let thin_stroke = set_live_part(&mut s, g, None, Some((100.0, 50.0)), vec![stroke(red(), 0.0)], 1.0);
    let clear_stroke = set_live_part(&mut s, g, None, Some((50.0, 50.0)), vec![clear(stroke(red(), 2.0))], 1.0);
    let clear_edge = set_live_part(&mut s, g, None, Some((75.0, 0.0)), vec![stroke(red(), 2.0)], 0.0);
    let parts = [lower_fill, hidden_fill, clear_fill, lower_stroke, hidden_stroke, thin_stroke, clear_stroke, clear_edge];
    assert_eq!(parts.iter().collect::<std::collections::HashSet<_>>().len(), parts.len());
    let d = s.doc().unwrap();
    let (_, leaves) = vectorcraft_tools::builder::shape_builder_shapes(&d.doc, &[g], true);
    let taken: Vec<NodeId> = leaves.iter().map(|l| l.id).collect();
    assert!(taken.contains(&lower_fill) && taken.contains(&lower_stroke), "a painted lower row draws: {taken:?}");
    for id in [hidden_fill, clear_fill, hidden_stroke, thin_stroke, clear_stroke, clear_edge] {
        assert!(!taken.contains(&id), "{id:?} draws nothing: {taken:?}");
    }
    // A face with a painted fill whose own opacity is 0 is left out too.
    let mut s = session();
    let g = live(&mut s);
    let clear_face = set_live_part(&mut s, g, Some((75.0, 50.0)), None, vec![fill(red())], 0.0);
    let d = s.doc().unwrap();
    let (_, leaves) = vectorcraft_tools::builder::shape_builder_shapes(&d.doc, &[g], true);
    assert!(!leaves.iter().any(|l| l.id == clear_face), "a face with an opacity of 0 draws nothing");
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
    let before = node(&s, g);
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[200, 30]]})).unwrap();
    // The merge does not use the group, so it stays as it was, with every edge: the outline of
    // 0..150 × 0..100 and the edges at x = 50 and 100.
    assert_eq!(s.doc().unwrap().doc.node(g), Some(&before));
    assert!(ids(&r).contains(&g), "{r}");
    let edges: Vec<NodeId> = vectorcraft_tools::builder::edges(&before).iter().map(|e| e.id).collect();
    assert!((stroked_length(&s, &edges) - 700.0).abs() < 1.0, "{}", stroked_length(&s, &edges));
}

/// A locked face of a Live Paint group the merge replaces is not a region, and a copy of it stays,
/// still locked.
#[test]
fn shape_builder_merge_keeps_a_locked_live_paint_face() {
    let mut s = session();
    let g = live(&mut s);
    let locked = vectorcraft_tools::builder::face_at(&node(&s, g), vectorcraft_geom::Point::new(125.0, 50.0)).unwrap().id;
    s.execute("object.lock", &json!({"ids": [locked.0]})).unwrap();
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    let map = s.execute("shapeBuilder.regions", &json!({})).unwrap();
    assert!(map["regions"].as_array().unwrap().iter().all(|r| !r["sources"].as_array().unwrap().iter().any(|i| *i == locked.0)), "{map}");
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[25, 50], [75, 50]]})).unwrap();
    assert_eq!(r["regions"], 2, "{r}");
    assert!(s.doc().unwrap().doc.node(g).is_none());
    let kept: Vec<Node> = ids(&r).into_iter().map(|i| node(&s, i)).filter(|n| n.locked).collect();
    assert_eq!(kept.len(), 1, "{r}");
    assert!((area(&s, kept[0].id) - 5000.0).abs() < 1.0, "{}", area(&s, kept[0].id));
}

/// A hidden face of a Live Paint group the merge replaces is not a region, and a copy of it stays,
/// still hidden.
#[test]
fn shape_builder_merge_keeps_a_hidden_live_paint_face() {
    let mut s = session();
    let g = live(&mut s);
    let hidden = vectorcraft_tools::builder::face_at(&node(&s, g), vectorcraft_geom::Point::new(125.0, 50.0)).unwrap().id;
    s.execute("object.hide", &json!({"ids": [hidden.0]})).unwrap();
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[25, 50], [75, 50]]})).unwrap();
    assert_eq!(r["regions"], 2, "{r}");
    assert!(s.doc().unwrap().doc.node(g).is_none());
    let kept: Vec<Node> = ids(&r).into_iter().map(|i| node(&s, i)).filter(|n| !n.visible).collect();
    assert_eq!(kept.len(), 1, "{r}");
    assert!((area(&s, kept[0].id) - 5000.0).abs() < 1.0, "{}", area(&s, kept[0].id));
}

/// A locked face without a fill and a locked edge without a stroke, in a Live Paint group the
/// merge replaces, each keep a copy, still locked.
#[test]
fn shape_builder_merge_keeps_unpainted_locked_live_paint_members() {
    let mut s = session();
    let g = live(&mut s);
    s.execute("livePaint.fill", &json!({"group": g.0, "point": [125, 50], "none": true})).unwrap();
    let n = node(&s, g);
    let face = vectorcraft_tools::builder::face_at(&n, Point::new(125.0, 50.0)).unwrap().id;
    let edge = vectorcraft_tools::builder::visible_edge_near(&n, Point::new(150.0, 50.0), 4.0).unwrap().id;
    let names = (node(&s, face).name, node(&s, edge).name);
    assert_ne!(names.0, names.1);
    s.edit("t", |d, _| {
        d.node_mut(edge).unwrap().appearance.set_stroke(Paint::None);
        Ok(())
    })
    .unwrap();
    s.execute("object.lock", &json!({"ids": [face.0, edge.0]})).unwrap();
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[25, 50], [75, 50]]})).unwrap();
    assert!(s.doc().unwrap().doc.node(g).is_none());
    let kept: Vec<Node> = ids(&r).into_iter().map(|i| node(&s, i)).filter(|n| n.locked).collect();
    let faces = kept.iter().filter(|n| n.name == names.0 && n.appearance.fill_paint().is_none() && (area(&s, n.id) - 5000.0).abs() < 1.0).count();
    let edges = kept.iter().filter(|n| n.name == names.1 && n.appearance.stroke_paint().is_none()).count();
    assert_eq!((kept.len(), faces, edges), (2, 1, 1), "{r}");
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
    // The merge keeps the unused group whole, so its rectangle is inside an id of the reply.
    let kept_area = |n: &Node| n.children().map_or_else(|| area(&s, n.id), |c| c.iter().map(|c| area(&s, c.id)).sum());
    assert!(out.iter().any(|i| (kept_area(&node(&s, *i)) - 10000.0).abs() < 1.0), "the group's rectangle is kept: {r}");
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

/// The traced paths of an Image Trace object: its children but the image and the seam fills'
/// clip group (a trace makes no other clip group).
fn traced_paths(n: &Node) -> Vec<Arc<Node>> {
    n.children().unwrap().iter().filter(|c| !matches!(c.kind, NodeKind::Image(_) | NodeKind::Group { clip: true, .. })).cloned().collect()
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

#[test]
fn image_trace_keep_colors_traces_only_the_colors_listed() {
    // A light field with a black ring, a red square and a blue bar on it.
    let art = vectorcraft_trace::Raster::from_fn(200, 150, |x, y| {
        let r = (f64::from(x) + 0.5 - 70.0).hypot(f64::from(y) + 0.5 - 60.0);
        if (34.0..=40.0).contains(&r) {
            [0, 0, 0, 255]
        } else if (130..170).contains(&x) && (30..70).contains(&y) {
            [200, 40, 50, 255]
        } else if (30..170).contains(&x) && (110..130).contains(&y) {
            [40, 80, 200, 255]
        } else {
            [240, 240, 220, 255]
        }
    });
    let traced = |keep: Value| {
        let mut s = session();
        add_image(&mut s, &art, Affine::IDENTITY);
        let params = json!({"params": {"mode": "color", "palette": "limited", "colors": 4, "keepColors": keep}});
        let r = s.execute("imageTrace.makeAndExpand", &params)?;
        let g = node(&s, NodeId(r["id"].as_u64().unwrap()));
        let fills: Vec<Paint> = g.children().unwrap().iter().map(|n| n.appearance.fill_paint()).collect();
        Ok::<_, EngineError>((r["colors"].clone(), fills))
    };
    let (black, red, blue, light) = (Color::rgb8(0, 0, 0), Color::rgb8(200, 40, 50), Color::rgb8(40, 80, 200), Color::rgb8(240, 240, 220));
    let has = |fills: &[Paint], c: Color| fills.contains(&Paint::solid(c));
    // Nothing listed: every color.
    let (colors, fills) = traced(json!([])).unwrap();
    assert_eq!(colors, 4);
    assert!([black, red, blue, light].into_iter().all(|c| has(&fills, c)), "{fills:?}");
    // The black and the red only.
    let (colors, fills) = traced(json!(["#000000", "#c8283c"])).unwrap();
    assert_eq!(colors, 2);
    assert!(has(&fills, black) && has(&fills, red), "{fills:?}");
    assert!(fills.iter().all(|f| *f == Paint::solid(black) || *f == Paint::solid(red)), "{fills:?}");
    // A color that is none is refused.
    let e = traced(json!(["#000000", "black"])).unwrap_err().to_string();
    assert!(e.contains("\"black\"") && e.contains("#rrggbb"), "{e}");
    assert!(find_command("imageTrace.make").unwrap().params.contains("keepColors?: [\"#rrggbb\"…]"));
    assert!(find_command("imageTrace.makeAndExpand").unwrap().params.contains("warnings: [text…] (as imageTrace.make)"));
}

#[test]
fn image_trace_keep_colors_warns_about_colors_that_match_nothing() {
    // A light field with a black ring and a red square on it.
    let art = vectorcraft_trace::Raster::from_fn(200, 150, |x, y| {
        let r = (f64::from(x) + 0.5 - 70.0).hypot(f64::from(y) + 0.5 - 60.0);
        if (34.0..=40.0).contains(&r) {
            [0, 0, 0, 255]
        } else if (130..170).contains(&x) && (30..70).contains(&y) {
            [200, 40, 50, 255]
        } else {
            [240, 240, 220, 255]
        }
    });
    let traced = |params: Value| {
        let mut s = session();
        add_image(&mut s, &art, Affine::IDENTITY);
        let r = s.execute("imageTrace.makeAndExpand", &json!({ "params": params })).unwrap();
        (r["paths"].as_u64().unwrap(), r["warnings"].clone())
    };
    let color = |keep: Value| traced(json!({"mode": "color", "palette": "limited", "colors": 3, "keepColors": keep}));
    // Every color listed is in the image: no warning.
    let (paths, warnings) = color(json!(["#000000", "#c8283c"]));
    assert!(paths > 0);
    assert_eq!(warnings, json!([]));
    // No color matches: nothing is traced, and the reply says why.
    assert_eq!(
        color(json!(["#ff00ff", "#00ffff"])),
        (0, json!(["Nothing was traced: none of the colors the trace found matches keepColors (#ff00ff, #00ffff)."]))
    );
    // One color matches nothing: the others are traced, and that one is named.
    let (paths, warnings) = color(json!(["#000000", "#00ffff"]));
    assert!(paths > 0);
    assert_eq!(warnings, json!(["keepColors: none of the colors the trace found matches #00ffff, so nothing of it is traced."]));
    // Ignore Color leaves out the only color listed.
    let ignored =
        traced(json!({"mode": "color", "palette": "limited", "colors": 3, "keepColors": ["#000000"], "ignoreWhite": true, "ignoreColor": "#000000"}));
    assert_eq!(ignored, (0, json!(["Nothing was traced: Ignore Color leaves out every area of the colors keepColors lists."])));
    // Flat Logo mode does not use the list.
    let (paths, warnings) = traced(json!({"mode": "logo", "keepColors": ["#ff00ff"]}));
    assert!(paths > 0);
    assert_eq!(warnings, json!(["Flat Logo mode does not use keepColors: it traces the colors named in logoColors, or the colors it finds."]));
}

/// #3098: a trace made away from the command (off the UI thread) and handed in makes the command's
/// own edit, as one undo step; Cancel stops it; once its image has changed it no longer applies.
#[test]
fn a_trace_run_hands_its_trace_to_the_command() {
    let p = json!({"params": {"ignoreWhite": true}});
    let mut direct = session();
    image_doc(&mut direct);
    let want = direct.execute("imageTrace.make", &p).unwrap();
    let mut s = session();
    let img = image_doc(&mut s);
    let run = cmd::TraceRun::prepare(&s, "imageTrace.make", &p).unwrap();
    assert_eq!((run.command(), run.params()["id"].as_u64(), run.image_size()), ("imageTrace.make", Some(img.0), (100, 100)));
    let total = std::sync::atomic::AtomicU64::new(0);
    let traced = run
        .run(&|_, t| {
            total.fetch_max(t, std::sync::atomic::Ordering::Relaxed);
            true
        })
        .unwrap();
    assert!(total.into_inner() > 0, "the layers are counted");
    assert!(run.current(&s));
    traced.hand_in(&mut s);
    let got = s.execute(run.command(), run.params()).unwrap();
    assert!(s.trace_hand_off.is_none(), "taken");
    assert_eq!(got, want);
    let g = NodeId(got["id"].as_u64().unwrap());
    assert_eq!(node(&s, g), node(&direct, g));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(matches!(node(&s, img).kind, NodeKind::Image(_)), "one undo step");

    assert!(matches!(run.run(&|_, _| false), Err(EngineError::Cancelled)));
    assert!(matches!(cmd::TraceRun::prepare(&s, "imageTrace.release", &p), Err(EngineError::Other(_))));
    // The image moved meanwhile: the run no longer applies, and the command traces it anew.
    let traced = run.run(&|_, _| true).unwrap();
    s.execute("object.move", &json!({"ids": [img.0], "dx": 10, "dy": 0})).unwrap();
    assert!(!run.current(&s));
    traced.hand_in(&mut s);
    let moved = s.execute(run.command(), run.params()).unwrap();
    assert!(s.trace_hand_off.is_none(), "dropped");
    let bb = outline(&traced_paths(&node(&s, NodeId(moved["id"].as_u64().unwrap())))[0]).0.bounds().unwrap();
    assert!((bb.center().x - 160.0).abs() < 1.5, "traced where the image is now: {bb:?}");
}

/// A trace made in one document applies only there: in another document with the same image,
/// object and settings, the run no longer applies, and both trace commands drop the trace handed
/// in and trace that document's own image, as one undo step.
#[test]
fn a_trace_run_applies_only_in_its_own_document() {
    let p = json!({"params": {"ignoreWhite": true}});
    for command in ["imageTrace.make", "imageTrace.makeAndExpand"] {
        let mut direct = session();
        image_doc(&mut direct);
        let want = direct.execute(command, &p).unwrap();
        let mut s = session();
        let img = image_doc(&mut s);
        let a = s.active_index().unwrap();
        let copy = (*s.doc().unwrap().doc).clone();
        let run = cmd::TraceRun::prepare(&s, command, &p).unwrap();
        let traced = run.run(&|_, _| true).unwrap();
        let b = s.add_document(copy, None);
        assert_ne!(s.documents()[a].uid, s.documents()[b].uid);
        assert!(!run.current(&s), "{command}: another document");
        assert!(s.set_active(a));
        assert!(run.current(&s), "{command}: its own document");
        assert!(s.set_active(b));
        traced.hand_in(&mut s);
        let got = s.execute(run.command(), run.params()).unwrap();
        assert!(s.trace_hand_off.is_none(), "{command}: dropped");
        assert_eq!(got, want, "{command}");
        let g = NodeId(got["id"].as_u64().unwrap());
        assert_eq!(node(&s, g), node(&direct, g), "{command}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(matches!(node(&s, img).kind, NodeKind::Image(_)), "{command}: one undo step");
        assert!(s.set_active(a));
        assert!(matches!(node(&s, img).kind, NodeKind::Image(_)), "{command}: the first document is untouched");
    }
}

/// A trace prepared before File › Revert doesn't apply after it, even when the document keeps its
/// uid and the restored image, object and settings are the same: both trace commands drop the
/// trace handed in and trace the restored image again, as one undo step. Reverting another
/// document, or a revert that fails, leaves the run current, and a trace prepared after the revert
/// applies as usual.
#[test]
fn a_trace_run_prepared_before_a_revert_does_not_apply_after_it() {
    let p = json!({"params": {"ignoreWhite": true}});
    for command in ["imageTrace.make", "imageTrace.makeAndExpand"] {
        let mut direct = session();
        image_doc(&mut direct);
        let want = direct.execute(command, &p).unwrap();
        let mut s = session();
        let img = image_doc(&mut s);
        let a = s.active_index().unwrap();
        let copy = (*s.doc().unwrap().doc).clone();
        let run = cmd::TraceRun::prepare(&s, command, &p).unwrap();
        let traced = run.run(&|_, _| true).unwrap();
        let b = s.add_document(copy.clone(), None);
        assert!(s.replace_document(b, copy.clone()));
        assert!(s.set_active(a));
        assert!(run.current(&s) && !run.reverted(&s), "{command}: another document reverted");
        assert!(!s.replace_document(s.documents().len(), copy.clone()));
        assert!(run.current(&s) && !run.reverted(&s), "{command}: a revert that failed");
        let (uid, reverts) = (s.doc().unwrap().uid, s.doc().unwrap().reverts);
        assert!(s.replace_document(a, copy));
        assert_eq!(s.doc().unwrap().uid, uid, "{command}: the tab keeps its uid");
        assert_eq!(s.doc().unwrap().reverts, reverts + 1, "{command}");
        assert!(!run.current(&s), "{command}: reverted");
        assert!(run.reverted(&s), "{command}");
        traced.hand_in(&mut s);
        let traces = cmd::TraceRun::traces_on_this_thread();
        let got = s.execute(run.command(), run.params()).unwrap();
        assert_eq!(cmd::TraceRun::traces_on_this_thread(), traces + 1, "{command}: traced again");
        assert!(s.trace_hand_off.is_none(), "{command}: dropped");
        assert_eq!(got, want, "{command}");
        let g = NodeId(got["id"].as_u64().unwrap());
        assert_eq!(node(&s, g), node(&direct, g), "{command}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(matches!(node(&s, img).kind, NodeKind::Image(_)), "{command}: one undo step");
        // A trace prepared after the revert is handed in and applied without tracing again.
        let after = cmd::TraceRun::prepare(&s, command, run.params()).unwrap();
        let traced = after.run(&|_, _| true).unwrap();
        assert!(after.current(&s) && !after.reverted(&s), "{command}: prepared after the revert");
        traced.hand_in(&mut s);
        let traces = cmd::TraceRun::traces_on_this_thread();
        s.execute(after.command(), after.params()).unwrap();
        assert_eq!(cmd::TraceRun::traces_on_this_thread(), traces, "{command}: the trace handed in applied");
    }
}

/// A plug-in run that calls Image Trace and then fails leaves a trace handed in for the command in
/// place, as it leaves the document.
#[test]
fn a_failed_plugin_run_keeps_a_handed_in_trace() {
    use vectorcraft_plugins::wat as tpl;
    let (id, cmd, args) = ("test.engine.calltrace", "imageTrace.make", r#"{"params":{"ignoreWhite":true}}"#);
    let mut s = session();
    s.allow_plugin_grants_from_requests();
    image_doc(&mut s);
    let p: Value = serde_json::from_str(args).unwrap();
    let run = cmd::TraceRun::prepare(&s, cmd, &p).unwrap();
    run.run(&|_, _| true).unwrap().hand_in(&mut s);
    let grant = json!(["doc.read", "doc.write", "commands"]);
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":{grant},"commandAllowlist":["{cmd}"]}}"#);
    let imports = format!(
        r#"(import "env" "vc_call" (func $vc_call (param i32 i32 i32 i32) (result i32)))
  (data (i32.const 0x2000) "{}") (data (i32.const 0x2100) "{}")"#,
        tpl::string(cmd),
        tpl::string(args)
    );
    let first =
        format!("(drop (call $vc_call (i32.const 0x2000) (i32.const {}) (i32.const 0x2100) (i32.const {}))) unreachable", cmd.len(), args.len());
    let wasm = wat::parse_str(tpl::with_imports(&manifest, &imports, &first, "{}")).unwrap();
    s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": grant, "replace": true})).unwrap();
    assert!(s.execute("plugin.run", &json!({"id": id, "ids": []})).is_err());
    assert!(s.trace_hand_off.is_some(), "the trace is still handed in");
    assert!(s.execute(cmd, &p).is_ok());
    assert!(s.trace_hand_off.is_none(), "taken");
    s.execute("plugin.remove", &json!({"id": id})).unwrap();
}

#[test]
fn image_trace_ignore_background_keeps_the_color_inside_the_art() {
    // A black field, a light panel on it and a black ring on the panel.
    let art = vectorcraft_trace::Raster::from_fn(200, 150, |x, y| {
        let r = (f64::from(x) + 0.5 - 100.0).hypot(f64::from(y) + 0.5 - 75.0);
        if !((20..180).contains(&x) && (20..130).contains(&y)) || (37.0..=43.0).contains(&r) { [0, 0, 0, 255] } else { [240, 240, 220, 255] }
    });
    let traced = |background: bool| {
        let mut s = session();
        add_image(&mut s, &art, Affine::IDENTITY);
        let params = json!({"params": {"mode": "color", "palette": "limited", "colors": 2, "ignoreWhite": true, "ignoreColor": "#000000", "ignoreBackground": background}});
        let r = s.execute("imageTrace.makeAndExpand", &params).unwrap();
        let g = node(&s, NodeId(r["id"].as_u64().unwrap()));
        let black: Vec<_> = g
            .children()
            .unwrap()
            .iter()
            .filter(|n| n.appearance.fill_paint() == Paint::solid(Color::rgb8(0, 0, 0)))
            .map(|n| outline(n).0.bounds().unwrap())
            .collect();
        (r["colors"].clone(), black)
    };
    // The whole color: the ring goes with the field.
    let (colors, black) = traced(false);
    assert_eq!(colors, 1);
    assert!(black.is_empty(), "{black:?}");
    // The background only: the ring stays, and nothing black reaches the image's edge.
    let (colors, black) = traced(true);
    assert_eq!(colors, 2);
    assert_eq!(black.len(), 1, "{black:?}");
    let bb = black[0];
    assert!((bb.x0 - 57.0).abs() < 1.5 && (bb.x1 - 143.0).abs() < 1.5 && (bb.y0 - 32.0).abs() < 1.5 && (bb.y1 - 118.0).abs() < 1.5, "{bb:?}");
    // The params doc tells agents about the option.
    assert!(find_command("imageTrace.make").unwrap().params.contains("ignoreBackground: false"));
}

/// A Document Library trace made away from its command no longer applies once the library's
/// colours change: the settings' JSON leaves them out (#4605 review).
#[test]
fn a_trace_run_needs_the_library_colours_it_traced_with() {
    let mut s = session();
    image_doc(&mut s);
    let p = json!({"preset": "6 Colors", "params": {"palette": "documentLibrary"}});
    let run = cmd::TraceRun::prepare(&s, "imageTrace.make", &p).unwrap();
    assert!(run.current(&s));
    let swatch = |w: &&vectorcraft_color::Swatch| matches!(w.paint, Paint::Solid { .. }) && !w.name.starts_with('[');
    let name = s.doc().unwrap().doc.swatches_iter().find(swatch).map(|w| w.name.clone()).unwrap();
    s.execute("swatch.edit", &json!({"name": name, "color": "#123456"})).unwrap();
    assert!(!run.current(&s), "{name} changed");
}

/// `image_doc`'s image with its disc 20 px to the left: the same size, other pixels.
fn moved_disc_png() -> Vec<u8> {
    vectorcraft_trace::Raster::from_fn(100, 100, |x, y| {
        let (dx, dy) = (x as f64 + 0.5 - 30.0, y as f64 + 0.5 - 50.0);
        if dx * dx + dy * dy <= 900.0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }
    })
    .encode_png()
}

/// Puts `bytes` under the image key `key`, or takes the key's image data away (`None`).
fn set_image_data(s: &mut Session, key: &str, bytes: Option<Vec<u8>>) {
    s.edit("image data", |d, _| {
        match bytes {
            Some(b) => d.images.insert(key.into(), ImageBlob::new("image/png", b)),
            None => d.images.remove(key),
        };
        Ok(())
    })
    .unwrap();
}

/// The middle of the first traced path of the Image Trace object the reply `r` names, in x.
fn traced_center_x(s: &Session, r: &Value) -> f64 {
    outline(&traced_paths(&node(s, NodeId(r["id"].as_u64().unwrap())))[0]).0.bounds().unwrap().center().x
}

/// A trace made away from its command no longer applies once the pixels under its image's key
/// change, though the object, its image record and the settings stay as they were; both commands
/// then trace the pixels there now. The same bytes in another allocation still apply.
#[test]
fn a_trace_run_needs_the_pixels_it_traced() {
    let p = json!({"params": {"ignoreWhite": true}});
    for command in ["imageTrace.make", "imageTrace.makeAndExpand"] {
        let mut s = session();
        image_doc(&mut s);
        let run = cmd::TraceRun::prepare(&s, command, &p).unwrap();
        let traced = run.run(&|_, _| true).unwrap();
        // The same bytes, allocated anew: still current.
        let same = s.doc().unwrap().doc.images["img1"].bytes.to_vec();
        set_image_data(&mut s, "img1", Some(same));
        assert!(run.current(&s), "{command}: equal bytes");
        // Other pixels under the same key: no longer current, and the hand-off is dropped.
        set_image_data(&mut s, "img1", Some(moved_disc_png()));
        assert!(!run.current(&s), "{command}: other pixels");
        traced.hand_in(&mut s);
        let r = s.execute(command, run.params()).unwrap();
        assert!(s.trace_hand_off.is_none(), "{command}: dropped");
        // The disc is now at image x 30, which the ×2 placement at 50 puts at 110.
        let x = traced_center_x(&s, &r);
        assert!((x - 110.0).abs() < 1.5, "{command}: traced the pixels there now, at {x}");
    }
}

/// Image data that is missing or empty: a run made before it went no longer applies, the run's
/// command gives the error it gives without a run, and nothing panics.
#[test]
fn a_trace_run_of_missing_or_empty_image_data() {
    let p = json!({"params": {"ignoreWhite": true}});
    let mut s = session();
    image_doc(&mut s);
    let run = cmd::TraceRun::prepare(&s, "imageTrace.make", &p).unwrap();
    let traced = run.run(&|_, _| true).unwrap();
    set_image_data(&mut s, "img1", None);
    assert!(!run.current(&s), "missing");
    traced.hand_in(&mut s);
    let e = s.execute(run.command(), run.params()).unwrap_err().to_string();
    assert!(e.contains("image data `img1` is missing"), "{e}");
    set_image_data(&mut s, "img1", Some(vec![]));
    assert!(!run.current(&s), "empty");
    let empty = cmd::TraceRun::prepare(&s, "imageTrace.make", &p).unwrap();
    assert!(empty.current(&s), "two empty blobs are equal");
    assert!(matches!(empty.run(&|_, _| true), Err(EngineError::Other(_))));
    assert!(matches!(s.execute(run.command(), run.params()), Err(EngineError::Other(_))));
}

/// Tracing an Image Trace object again follows the same rule: once the pixels of the image it
/// holds change, a run made before no longer applies, and the command traces the pixels there now.
#[test]
fn a_trace_run_of_an_image_trace_object_needs_its_pixels() {
    let p = json!({"params": {"ignoreWhite": true}});
    let mut s = session();
    image_doc(&mut s);
    let g = s.execute("imageTrace.make", &p).unwrap()["id"].as_u64().unwrap();
    let again = json!({"id": g, "params": {"ignoreWhite": true}});
    let run = cmd::TraceRun::prepare(&s, "imageTrace.make", &again).unwrap();
    assert_eq!(run.target(), NodeId(g));
    let traced = run.run(&|_, _| true).unwrap();
    assert!(run.current(&s));
    set_image_data(&mut s, "img1", Some(moved_disc_png()));
    assert!(!run.current(&s));
    traced.hand_in(&mut s);
    let r = s.execute(run.command(), run.params()).unwrap();
    assert!(s.trace_hand_off.is_none(), "dropped");
    let x = traced_center_x(&s, &r);
    assert!((x - 110.0).abs() < 1.5, "traced the pixels there now, at {x}");
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

/// Abutting with Fills: the strokes along the edges the paths share sit in a "Seam Fills" group
/// under the paths, in the lower path's color, and stay under them through Expand by color.
#[test]
fn image_trace_abutting_seam_fills_sit_under_the_paths() {
    // Red beside blue, the edge between them anti-aliased.
    let art = vectorcraft_trace::Raster::from_fn(40, 20, |x, _| {
        let share = (16.4 - f64::from(x)).clamp(0.0, 1.0);
        let mix = |a: f64, b: f64| (a * share + b * (1.0 - share)).round() as u8;
        [mix(200.0, 30.0), mix(40.0, 90.0), mix(50.0, 200.0), 255]
    });
    let seams = |n: &Node| n.children().unwrap().iter().find(|c| c.name.as_deref() == Some("Seam Fills")).cloned();
    let made = |method: &str, cmd: &str, by_color: bool| {
        let mut s = session();
        let img = add_image(&mut s, &art, Affine::scale(2.0));
        let params = json!({"mode": "color", "palette": "limited", "colors": 2, "method": method});
        let mut p = json!({ "params": params });
        if by_color {
            p["byColor"] = json!(true);
        }
        let r = s.execute(cmd, &p).unwrap();
        (s, img, r)
    };
    let (mut s, _, r) = made("abutting", "imageTrace.make", false);
    assert_eq!(r["paths"], 2, "{r}");
    let g = NodeId(r["id"].as_u64().unwrap());
    let n = node(&s, g);
    let kids = n.children().unwrap();
    assert_eq!(kids.len(), 4, "the image, the seam fills and two paths");
    assert!(matches!(kids[0].kind, NodeKind::Image(_)));
    let group = seams(&n).unwrap();
    assert!(Arc::ptr_eq(&group, &kids[1]), "the seam fills come under the paths");
    assert!(matches!(group.kind, NodeKind::Group { clip: true, .. }), "{group:?}");
    // The two paths cover the image, so the strokes are clipped to its box.
    let (clip, strokes) = group.children().unwrap().split_first().unwrap();
    assert!(matches!(clip.kind, NodeKind::Path { clipping: true, .. }), "{clip:?}");
    let cb = outline(clip).0.bounds().unwrap();
    assert!([cb.x0, cb.y0, cb.x1 - 80.0, cb.y1 - 40.0].iter().all(|v| v.abs() < 1e-9), "{cb:?}");
    assert_eq!(strokes.len(), 1);
    assert_eq!(strokes[0].appearance.fill_paint(), Paint::None);
    let AppearanceItem::Stroke(layer) = &strokes[0].appearance.items[0] else { panic!("{:?}", strokes[0].appearance) };
    assert_eq!((layer.cap, layer.join, layer.width), (LineCap::Butt, LineJoin::Round, 3.0), "1.5 image pixels, the image scaled 2×");
    assert_eq!(layer.paint, kids[2].appearance.fill_paint(), "the lower path's color");
    // The stroke runs along the edge, from the image's top to its bottom.
    let bb = outline(&strokes[0]).0.bounds().unwrap();
    assert!((bb.center().x - 32.8).abs() < 1.0 && bb.y0.abs() < 1e-9 && (bb.y1 - 40.0).abs() < 1e-9, "{bb:?}");
    // Expand by color keeps the seam fills under the color groups, renamed too; Release gives the
    // image back.
    s.execute("layer.setProps", &json!({"id": group.id.0, "name": "Edges"})).unwrap();
    s.execute("imageTrace.expand", &json!({"id": g.0, "byColor": true})).unwrap();
    let n = node(&s, g);
    let names: Vec<_> = n.children().unwrap().iter().map(|c| c.name.clone().unwrap_or_default()).collect();
    assert_eq!(names.len(), 3, "{names:?}");
    assert_eq!(names[0], "Edges", "{names:?}");
    assert_eq!(n.children().unwrap()[0].id, group.id);
    // Above a path, the seam fills' group is refused by name, not as an Auto Grouping group.
    let (mut s, _, r) = made("abutting", "imageTrace.make", false);
    let g = NodeId(r["id"].as_u64().unwrap());
    let kids = node(&s, g).children().unwrap().clone();
    s.execute("object.arrange.bringToFront", &json!({"ids": [kids[1].id.0]})).unwrap();
    assert_eq!(node(&s, g).children().unwrap().last().map(|c| c.id), Some(kids[1].id));
    let e = s.execute("imageTrace.expand", &json!({"id": g.0, "byColor": true})).unwrap_err().to_string();
    assert!(e.contains("seam fills") && !e.contains("Auto Grouping"), "{e}");
    let (mut s, img, _) = made("abutting", "imageTrace.make", false);
    assert_eq!(ids(&s.execute("imageTrace.release", &json!({})).unwrap()), vec![img]);
    // Make and Expand by color too.
    let (s, _, r) = made("abutting", "imageTrace.makeAndExpand", true);
    let n = node(&s, NodeId(r["id"].as_u64().unwrap()));
    assert_eq!(n.children().unwrap()[0].name.as_deref(), Some("Seam Fills"));
    // Overlapping paths are stacked: no seam fills.
    let (s, _, r) = made("overlapping", "imageTrace.make", false);
    assert!(seams(&node(&s, NodeId(r["id"].as_u64().unwrap()))).is_none());
}

/// With white left out, the seam fills are clipped to the image's box within the traced paths'
/// box, so the expanded trace measures as its paths do, also on a turned image.
#[test]
fn image_trace_seam_fills_stay_within_the_traced_paths_box() {
    let seams = |n: &Node| n.children().unwrap().iter().find(|c| c.name.as_deref() == Some("Seam Fills")).cloned();
    let band = vectorcraft_trace::Raster::from_fn(24, 16, |x, y| match ((4..12).contains(&y), x < 12) {
        (false, _) => [255; 4],
        (true, true) => [200, 30, 30, 255],
        (true, false) => [30, 30, 200, 255],
    });
    for xf in [Affine::IDENTITY, Affine::translate((100.0, 100.0)) * Affine::rotate(0.5)] {
        let mut s = session();
        add_image(&mut s, &band, xf);
        let r = s
            .execute(
                "imageTrace.makeAndExpand",
                &json!({"params": {"mode": "color", "palette": "limited", "colors": 3, "method": "abutting", "ignoreWhite": true}}),
            )
            .unwrap();
        assert_eq!(r["paths"], 2, "{r}");
        let n = node(&s, NodeId(r["id"].as_u64().unwrap()));
        let group = seams(&n).unwrap();
        let clip = &group.children().unwrap()[0];
        assert!(matches!(clip.kind, NodeKind::Path { clipping: true, .. }), "{clip:?}");
        let cb = outline(clip).0.bounds().unwrap();
        let paths = n.children().unwrap()[1..].iter().filter_map(|c| c.geometric_bounds()).reduce(|a, b| a.union(b)).unwrap();
        assert_eq!(n.geometric_bounds(), Some(paths), "{xf:?}");
        assert!(cb.x0 >= paths.x0 - 1e-9 && cb.y0 >= paths.y0 - 1e-9 && cb.x1 <= paths.x1 + 1e-9 && cb.y1 <= paths.y1 + 1e-9, "{xf:?}: {cb:?}");
        if xf == Affine::IDENTITY {
            assert!([cb.x0, cb.y0 - 4.0, cb.x1 - 24.0, cb.y1 - 12.0].iter().all(|v| v.abs() < 1e-9), "{cb:?}");
        }
    }
}

/// On an image stretched more one way than another, the seam fills are the strokes' outlines in
/// image pixels, filled: on an image stretched 20× across, the one along a horizontal edge covers
/// 0.75 image pixels on each side of it, as on an image scaled alike both ways, and does not reach
/// across the transparent rows above the edge.
#[test]
fn image_trace_seam_fills_on_a_stretched_image_stay_near_their_edge() {
    // Red beside blue, the edge between them anti-aliased, on an image stretched 3× across and
    // squeezed to half height: the outline is 1.5 pixels (4.5 points) across the vertical edge.
    let art = vectorcraft_trace::Raster::from_fn(40, 20, |x, _| {
        let share = (16.4 - f64::from(x)).clamp(0.0, 1.0);
        let mix = |a: f64, b: f64| (a * share + b * (1.0 - share)).round() as u8;
        [mix(200.0, 30.0), mix(40.0, 90.0), mix(50.0, 200.0), 255]
    });
    let mut s = session();
    add_image(&mut s, &art, Affine::scale_non_uniform(3.0, 0.5));
    let r = s.execute("imageTrace.make", &json!({"params": {"mode": "color", "palette": "limited", "colors": 2, "method": "abutting"}})).unwrap();
    let n = node(&s, NodeId(r["id"].as_u64().unwrap()));
    let kids = n.children().unwrap();
    assert_eq!(kids[1].name.as_deref(), Some("Seam Fills"));
    let fill = &kids[1].children().unwrap()[1];
    assert_eq!(fill.appearance.stroke_paint(), Paint::None, "{:?}", fill.appearance);
    assert_eq!(fill.appearance.fill_paint(), kids[2].appearance.fill_paint(), "the lower path's color");
    let bb = outline(fill).0.bounds().unwrap();
    assert!((bb.width() - 4.5).abs() < 0.05 && bb.y0.abs() < 1e-6 && (bb.y1 - 10.0).abs() < 1e-6, "{bb:?}");
    // Rows 0 to 5 transparent, 5 to 10 red, 10 to 20 blue.
    let art = vectorcraft_trace::Raster::from_fn(40, 20, |_, y| match y {
        0..5 => [0, 0, 0, 0],
        5..10 => [200, 30, 30, 255],
        _ => [30, 30, 200, 255],
    });
    let mut s = session();
    add_image(&mut s, &art, Affine::scale_non_uniform(20.0, 1.0));
    let r = s.execute("imageTrace.make", &json!({"params": {"mode": "color", "palette": "limited", "colors": 2, "method": "abutting"}})).unwrap();
    let n = node(&s, NodeId(r["id"].as_u64().unwrap()));
    let group = n.children().unwrap().iter().find(|c| c.name.as_deref() == Some("Seam Fills")).cloned().unwrap();
    let (_, members) = group.children().unwrap().split_first().unwrap();
    assert!(!members.is_empty(), "{group:?}");
    for m in members {
        // What the member covers: its outline, grown by half its stroke's width.
        let half = m.appearance.items.iter().fold(0.0_f64, |w, it| match it {
            AppearanceItem::Stroke(l) => w.max(l.width / 2.0),
            _ => w,
        });
        let bb = outline(m).0.bounds().unwrap();
        let (y0, y1) = (bb.y0 - half, bb.y1 + half);
        assert!(y0 > 9.25 - 1e-6 && y1 < 10.75 + 1e-6, "covers y {y0} to {y1}: {m:?}");
    }
    // Renamed, the group of filled outlines is still the seam fills: Expand by color keeps it under
    // every color's group.
    s.execute("layer.setProps", &json!({"id": group.id.0, "name": "Edges"})).unwrap();
    s.execute("imageTrace.expand", &json!({"id": n.id.0, "byColor": true})).unwrap();
    assert_eq!(node(&s, n.id).children().unwrap()[0].id, group.id);
}

/// A trace made before the seam fills were clipped holds them in a group without a clip: Expand by
/// color keeps that group under every color's group too, named or renamed.
#[test]
fn image_trace_expand_by_color_keeps_unclipped_seam_fills() {
    let art = vectorcraft_trace::Raster::from_fn(40, 20, |x, _| {
        let share = (16.4 - f64::from(x)).clamp(0.0, 1.0);
        let mix = |a: f64, b: f64| (a * share + b * (1.0 - share)).round() as u8;
        [mix(200.0, 30.0), mix(40.0, 90.0), mix(50.0, 200.0), 255]
    });
    for name in [Some("Seam Fills"), None] {
        let mut s = session();
        add_image(&mut s, &art, Affine::scale(2.0));
        let r = s.execute("imageTrace.make", &json!({"params": {"mode": "color", "palette": "limited", "colors": 2, "method": "abutting"}})).unwrap();
        let g = NodeId(r["id"].as_u64().unwrap());
        // The seam fills as such a trace holds them: the strokes alone, in a plain group.
        let seams = node(&s, g).children().unwrap()[1].clone();
        let NodeKind::Group { clip: true, children } = &seams.kind else { panic!("{seams:?}") };
        let mut legacy = Node::group(seams.id, children[1..].to_vec());
        legacy.name = name.map(Into::into);
        s.edit("legacy", |d, _| {
            let NodeKind::Group { children, .. } = &mut d.node_mut(g).unwrap().kind else { panic!("trace group") };
            children[1] = Arc::new(legacy);
            Ok(())
        })
        .unwrap();
        s.execute("imageTrace.expand", &json!({"id": g.0, "byColor": true})).unwrap();
        let kids = node(&s, g).children().unwrap().clone();
        assert_eq!(kids.len(), 3, "{name:?}: {kids:?}");
        assert_eq!(kids[0].id, seams.id, "{name:?}");
    }
}

/// Expand by color: each color's paths in a group of their own, named by the color, the groups
/// stacked by the lowest path in each and the art drawn as before; Release to Layers then gives a
/// sublayer for each color.
#[test]
#[allow(clippy::cognitive_complexity)] // Exercises several color and stacking cases in one trace fixture.
fn image_trace_expand_by_color_groups_each_colors_paths() {
    // A light field with a black ring and a red square on it.
    let art = vectorcraft_trace::Raster::from_fn(200, 150, |x, y| {
        let r = (f64::from(x) + 0.5 - 70.0).hypot(f64::from(y) + 0.5 - 75.0);
        if (34.0..=40.0).contains(&r) {
            [0, 0, 0, 255]
        } else if (130..170).contains(&x) && (55..95).contains(&y) {
            [200, 40, 50, 255]
        } else {
            [240, 240, 220, 255]
        }
    });
    let names = |n: &Node| n.children().unwrap().iter().map(|c| c.name.clone().unwrap_or_default()).collect::<Vec<_>>();
    let hex = |n: &Node| n.appearance.fill_paint().color().unwrap().to_hex();
    let shot = |s: &Session| {
        let img = render_doc(s, false);
        (0..200).flat_map(|x| (0..150).map(move |y| (x, y))).map(|(x, y)| img.pixel(x, y)).collect::<Vec<_>>()
    };
    for method in ["abutting", "overlapping"] {
        let params = json!({"mode": "color", "palette": "limited", "colors": 3, "method": method});
        let expand = |by_color: bool| {
            let mut s = session();
            add_image(&mut s, &art, Affine::IDENTITY);
            let r = s.execute("imageTrace.makeAndExpand", &json!({"params": params, "byColor": by_color})).unwrap();
            let g = NodeId(r["id"].as_u64().unwrap());
            (s, g)
        };
        let (flat, fg) = expand(false);
        let (mut s, g) = expand(true);
        let kids = node(&flat, fg).children().unwrap().clone();
        // Abutting: the seam fills' group comes first, under every path, with byColor too.
        let seams = usize::from(method == "abutting");
        assert!(kids.iter().take(seams).all(|c| c.name.as_deref() == Some("Seam Fills")), "{method}");
        let paths = kids[seams..].to_vec();
        // Without byColor (the default) the paths sit in the trace's group, with no group of their own.
        assert!(paths.iter().all(|p| !matches!(p.kind, NodeKind::Group { .. })), "{method}");
        let (default, dg) = {
            let mut s = session();
            add_image(&mut s, &art, Affine::IDENTITY);
            let r = s.execute("imageTrace.makeAndExpand", &json!({"params": params})).unwrap();
            (s, NodeId(r["id"].as_u64().unwrap()))
        };
        assert!(*node(&default, dg).children().unwrap() == kids, "{method}: byColor is false when not given");
        // The colors in the order they first come in, bottom to top.
        let mut order: Vec<String> = vec![];
        for p in &paths {
            if !order.contains(&hex(p)) {
                order.push(hex(p));
            }
        }
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(sorted, ["#000000", "#c82832", "#f0f0dc"], "{method}");
        let n = node(&s, g);
        // Overlapping fills the ring whole and draws the field inside it above the ring: that path starts a second
        // group of the field's color, above the black one.
        let want: Vec<String> = if method == "overlapping" { order.iter().chain(order.first()).cloned().collect() } else { order.clone() };
        assert_eq!(names(&n)[seams..], want, "{method}");
        assert!(n.children().unwrap().iter().take(seams).all(|c| *c == kids[0]), "{method}: the seam fills stay as they are");
        // Each group holds its color's paths, and in turn they are the paths of the trace without
        // byColor, in their order.
        let groups = &n.children().unwrap()[seams..];
        for c in groups {
            assert!(matches!(c.kind, NodeKind::Group { .. }), "{method}");
            assert!(c.children().unwrap().iter().all(|m| Some(hex(m)) == c.name), "{method}: {:?}", c.name);
        }
        let members: Vec<Arc<Node>> = groups.iter().flat_map(|c| c.children().unwrap().iter().cloned()).collect();
        assert!(members == paths, "{method}");
        assert!(shot(&s) == shot(&flat), "{method}: the art changed");
        // Release to Layers: a sublayer for each color, in the same order.
        let r = s.execute("layer.releaseToLayers", &json!({"id": g.0})).unwrap();
        let subs = ids(&r);
        assert_eq!(subs.len(), seams + want.len(), "{method}");
        for (sub, name) in subs[seams..].iter().zip(&want) {
            let l = node(&s, *sub);
            assert!(matches!(l.kind, NodeKind::Layer { .. }), "{method}");
            assert_eq!(names(&l), std::slice::from_ref(name), "{method}");
        }
    }

    // imageTrace.expand takes it too, as one undo step.
    let mut s = session();
    add_image(&mut s, &art, Affine::IDENTITY);
    let params = json!({"mode": "color", "palette": "limited", "colors": 3});
    let g = NodeId(s.execute("imageTrace.make", &json!({"params": params})).unwrap()["id"].as_u64().unwrap());
    let steps = s.doc().unwrap().history.undo.len();
    s.execute("imageTrace.expand", &json!({"byColor": true})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1);
    let mut got = names(&node(&s, g));
    got.sort();
    assert_eq!(got, ["#000000", "#c82832", "#f0f0dc", "Seam Fills"]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, g).name.as_deref(), Some("Image Trace"));

    // A trace of one color gives one group holding every path.
    let mut one = session();
    let bar =
        vectorcraft_trace::Raster::from_fn(80, 40, |x, y| if (10..70).contains(&x) && (10..30).contains(&y) { [0, 0, 0, 255] } else { [255; 4] });
    add_image(&mut one, &bar, Affine::IDENTITY);
    let r = one.execute("imageTrace.makeAndExpand", &json!({"params": {"ignoreWhite": true}, "byColor": true})).unwrap();
    let og = node(&one, NodeId(r["id"].as_u64().unwrap()));
    assert_eq!(names(&og), ["#000000"]);
    let only = &og.children().unwrap()[0];
    assert!(!only.children().unwrap().is_empty() && only.children().unwrap().iter().all(|m| hex(m) == "#000000"));

    // Auto Grouping's groups are not split: refused, and the trace stays as it was.
    let mut grouped = params.clone();
    grouped["autoGrouping"] = json!(true);
    let e = s.execute("imageTrace.makeAndExpand", &json!({"id": g.0, "params": grouped, "byColor": true})).unwrap_err().to_string();
    assert!(e.contains("Auto Grouping"), "{e}");
    let g = NodeId(s.execute("imageTrace.make", &json!({"id": g.0, "params": grouped})).unwrap()["id"].as_u64().unwrap());
    let e = s.execute("imageTrace.expand", &json!({"byColor": true})).unwrap_err().to_string();
    assert!(e.contains("Auto Grouping"), "{e}");
    assert_eq!(node(&s, g).name.as_deref(), Some("Image Trace"));
    let e = s.execute("imageTrace.expand", &json!({"byColor": "yes"})).unwrap_err().to_string();
    assert!(e.contains("true or false"), "{e}");
    // Refused too when Auto Grouping grouped nothing, as with side-by-side stripes.
    let mut s = session();
    let stripes = vectorcraft_trace::Raster::from_fn(60, 60, |x, _| match x / 20 {
        0 => [200, 40, 40, 255],
        1 => [40, 160, 60, 255],
        _ => [40, 70, 200, 255],
    });
    add_image(&mut s, &stripes, Affine::IDENTITY);
    let g = NodeId(s.execute("imageTrace.make", &json!({"params": grouped})).unwrap()["id"].as_u64().unwrap());
    assert!(traced_paths(&node(&s, g)).iter().all(|c| !matches!(c.kind, NodeKind::Group { .. })));
    let e = s.execute("imageTrace.expand", &json!({"byColor": true})).unwrap_err().to_string();
    assert!(e.contains("Auto Grouping"), "{e}");
    assert_eq!(node(&s, g).name.as_deref(), Some("Image Trace"));

    // A color linked to a global swatch goes by the swatch's name, a tint with its percentage.
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Ink", "color": "#c82828", "global": true})).unwrap();
    s.execute("swatch.new", &json!({"name": "Sky", "color": "#2846c8", "global": true})).unwrap();
    let r = vectorcraft_trace::Raster::from_fn(60, 60, |x, y| match (x < 30, y < 30) {
        (true, _) => [200, 40, 40, 255],
        (false, true) => [40, 160, 60, 255],
        (false, false) => [40, 70, 200, 255],
    });
    add_image(&mut s, &r, Affine::IDENTITY);
    let limited = json!({"mode": "color", "palette": "limited", "colors": 3});
    let g = NodeId(s.execute("imageTrace.make", &json!({"params": limited})).unwrap()["id"].as_u64().unwrap());
    let paths: Vec<(u64, String)> = traced_paths(&node(&s, g)).iter().map(|c| (c.id.0, hex(c))).collect();
    let mut colors: Vec<&str> = paths.iter().map(|(_, h)| h.as_str()).collect();
    colors.sort();
    colors.dedup();
    assert_eq!(colors, ["#2846c8", "#28a03c", "#c82828"]);
    // The red paths link to Ink and the blue ones to Sky at 50%; green stays a plain color.
    let pick = |want: &str| paths.iter().filter(|(_, h)| h == want).map(|(id, _)| *id).collect::<Vec<_>>();
    s.execute("paint.setFill", &json!({"ids": pick("#c82828"), "swatch": "Ink"})).unwrap();
    s.execute("paint.setFill", &json!({"ids": pick("#2846c8"), "swatch": "Sky", "tint": 50})).unwrap();
    s.execute("imageTrace.expand", &json!({"id": g.0, "byColor": true})).unwrap();
    let mut got = names(&node(&s, g));
    got.sort();
    assert_eq!(got, ["#28a03c", "Ink", "Seam Fills", "Sky 50%"]);
    // A swatch named as another color's #rrggbb still gets a group of its own.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("swatch.edit", &json!({"name": "Ink", "newName": "#28a03c"})).unwrap();
    s.execute("imageTrace.expand", &json!({"id": g.0, "byColor": true})).unwrap();
    let mut got = names(&node(&s, g));
    got.sort();
    assert_eq!(got, ["#28a03c", "#28a03c", "Seam Fills", "Sky 50%"]);

    // Gradient fills share one group, and a stroked line goes by its stroke.
    let ramp = vectorcraft_trace::Raster::from_fn(100, 60, |x, _| {
        let t = (f64::from(x) + 0.5) / 100.0;
        [(20.0 + 200.0 * t) as u8, 40, (220.0 - 180.0 * t) as u8, 255]
    });
    let line =
        vectorcraft_trace::Raster::from_fn(100, 100, |x, y| if (10..90).contains(&x) && (49..52).contains(&y) { [0, 0, 0, 255] } else { [255; 4] });
    for (art, params, want) in [
        (ramp, json!({"preset": "6 Colors", "params": {"gradients": true}, "byColor": true}), "Gradients"),
        (line, json!({"params": {"ignoreWhite": true, "strokes": true, "strokeWidth": 5}, "byColor": true}), "#000000"),
    ] {
        let mut s = session();
        add_image(&mut s, &art, Affine::IDENTITY);
        let res = s.execute("imageTrace.makeAndExpand", &params).unwrap();
        assert_eq!(names(&node(&s, NodeId(res["id"].as_u64().unwrap()))), [want]);
    }

    // Flat Logo stacks its shapes biggest first without cutting holes under white: a black ring's
    // outline, a white disc on it and a black dot on that. Black keeps a group below white and one
    // above it, and the art draws as before, through Make and Expand and through Expand.
    let target = vectorcraft_trace::Raster::from_fn(120, 120, |x, y| {
        let r = (f64::from(x) + 0.5 - 60.0).hypot(f64::from(y) + 0.5 - 60.0);
        if r < 15.0 || (30.0..50.0).contains(&r) {
            [0, 0, 0, 255]
        } else if r < 30.0 {
            [255, 255, 255, 255]
        } else {
            [0, 0, 0, 0]
        }
    });
    let logo = json!({"mode": "logo", "logoColors": ["#000000"]});
    let traced = |by_color: bool, expand_after: bool| {
        let mut s = session();
        add_image(&mut s, &target, Affine::IDENTITY);
        let g = if expand_after {
            let g = s.execute("imageTrace.make", &json!({"params": logo})).unwrap()["id"].as_u64().unwrap();
            s.execute("imageTrace.expand", &json!({"byColor": by_color})).unwrap();
            g
        } else {
            s.execute("imageTrace.makeAndExpand", &json!({"params": logo, "byColor": by_color})).unwrap()["id"].as_u64().unwrap()
        };
        (s, NodeId(g))
    };
    for expand_after in [false, true] {
        let (flat, fg) = traced(false, expand_after);
        let (s, g) = traced(true, expand_after);
        let paths = node(&flat, fg).children().unwrap().clone();
        assert_eq!(paths.iter().map(|p| hex(p)).collect::<Vec<_>>(), ["#000000", "#ffffff", "#000000"]);
        assert_eq!(names(&node(&s, g)), ["#000000", "#ffffff", "#000000"], "expand after: {expand_after}");
        let members: Vec<Arc<Node>> = node(&s, g).children().unwrap().iter().flat_map(|c| c.children().unwrap().iter().cloned()).collect();
        assert!(members == paths, "expand after: {expand_after}");
        assert!(shot(&s) == shot(&flat), "expand after: {expand_after}: the art changed");
    }

    // An Abutting trace's top path, given the bottom path's color and moved onto the middle one (by
    // a move or by a Transform effect), moved clear of it with a drop shadow cast onto it, or moved
    // to share a pixel at its edge, before Expand, still draws on top of it.
    let stripes = vectorcraft_trace::Raster::from_fn(90, 30, |x, _| match x / 30 {
        0 => [200, 40, 40, 255],
        1 => [40, 160, 60, 255],
        _ => [40, 70, 200, 255],
    });
    let edited = |by_color: bool, how: &str| {
        let mut s = session();
        add_image(&mut s, &stripes, Affine::IDENTITY);
        let limited = json!({"mode": "color", "palette": "limited", "colors": 3, "method": "abutting"});
        let g = NodeId(s.execute("imageTrace.make", &json!({"params": limited})).unwrap()["id"].as_u64().unwrap());
        let paths = traced_paths(&node(&s, g));
        assert_eq!(paths.len(), 3);
        let (first, middle, last) = (&paths[0], &paths[1], &paths[2]);
        s.execute("paint.setFill", &json!({"ids": [last.id.0], "color": hex(first)})).unwrap();
        let (to, from) = (middle.visual_bounds().unwrap(), last.visual_bounds().unwrap());
        let (dx, dy) = (to.x0 - from.x0, to.y0 - from.y0);
        match how {
            "beside" => {
                // The middle path and the moved top path end and start inside one pixel.
                s.execute("object.move", &json!({"ids": [middle.id.0], "dx": -0.5, "dy": 0})).unwrap();
                s.execute("object.move", &json!({"ids": [last.id.0], "dx": -0.3, "dy": 0})).unwrap()
            }
            "move" => s.execute("object.move", &json!({"ids": [last.id.0], "dx": dx, "dy": dy})).unwrap(),
            "transform" => {
                s.execute("effect.apply", &json!({"ids": [last.id.0], "effect": "distort.transform", "params": {"moveH": dx, "moveV": dy}})).unwrap()
            }
            _ => {
                s.execute("object.move", &json!({"ids": [last.id.0], "dx": 40, "dy": 0})).unwrap();
                s.execute("effect.apply", &json!({"ids": [last.id.0], "effect": "stylize.dropShadow", "params": {"x": -70, "y": 0, "blur": 0}}))
                    .unwrap()
            }
        };
        s.execute("imageTrace.expand", &json!({"id": g.0, "byColor": by_color})).unwrap();
        (s, g)
    };
    for how in ["move", "transform", "shadow", "beside"] {
        let (flat, _) = edited(false, how);
        let (s, g) = edited(true, how);
        assert_eq!(names(&node(&s, g)).len(), 4, "{how}: the seam fills and three colors");
        assert!(shot(&s) == shot(&flat), "{how}: the edited art changed");
    }

    // Two tints of one swatch that round to the same percentage are still two colors.
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Sky", "color": "#2846c8", "global": true})).unwrap();
    add_image(&mut s, &stripes, Affine::IDENTITY);
    let limited = json!({"mode": "color", "palette": "limited", "colors": 3});
    let g = NodeId(s.execute("imageTrace.make", &json!({"params": limited})).unwrap()["id"].as_u64().unwrap());
    let paths: Vec<u64> = traced_paths(&node(&s, g)).iter().map(|c| c.id.0).collect();
    assert_eq!(paths.len(), 3);
    s.execute("paint.setFill", &json!({"ids": [paths[0]], "swatch": "Sky", "tint": 50.1})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [paths[1], paths[2]], "swatch": "Sky", "tint": 50.4})).unwrap();
    s.execute("imageTrace.expand", &json!({"id": g.0, "byColor": true})).unwrap();
    let groups = node(&s, g).children().unwrap().clone();
    assert_eq!(names(&node(&s, g)), ["Seam Fills", "Sky 50%", "Sky 50%"]);
    assert_eq!(groups.iter().skip(1).map(|c| c.children().unwrap().len()).collect::<Vec<_>>(), [1, 2]);
}

#[test]
fn image_trace_expand_by_color_preserves_live_pathfinder_art() {
    let mut s = session();
    let art = vectorcraft_trace::Raster::from_fn(100, 60, |x, y| {
        if (10..40).contains(&y) && ((10..35).contains(&x) || (65..90).contains(&x)) { [0, 0, 0, 255] } else { [255, 255, 255, 255] }
    });
    add_image(&mut s, &art, Affine::IDENTITY);
    let g = NodeId(s.execute("imageTrace.make", &json!({"params": {"ignoreWhite": true}})).unwrap()["id"].as_u64().unwrap());
    let paths: Vec<NodeId> = node(&s, g).children().unwrap().iter().filter(|c| c.path_data().is_some()).map(|c| c.id).collect();
    assert_eq!(paths.len(), 2);
    s.execute("select.set", &json!({"ids": [paths[1].0]})).unwrap();
    s.execute("object.move", &json!({"dx": -45, "dy": 0})).unwrap();
    s.execute("effect.apply", &json!({"ids": [g.0], "effect": "pathfinder.subtract"})).unwrap();
    let before = render_doc(&s, false);
    let before_pixels: Vec<_> = (0..100).flat_map(|x| (0..60).map(move |y| (x, y))).map(|(x, y)| before.pixel(x, y)).collect();
    let original = node(&s, g);
    let steps = s.doc().unwrap().history.undo.len();
    let result = s.execute("imageTrace.expand", &json!({"id": g.0, "byColor": true}));
    if let Err(e) = &result {
        assert!(e.to_string().contains("Pathfinder"), "{e}");
        assert_eq!(node(&s, g), original);
        assert_eq!(s.doc().unwrap().history.undo.len(), steps);
    }
    let after = render_doc(&s, false);
    let after_pixels: Vec<_> = (0..100).flat_map(|x| (0..60).map(move |y| (x, y))).map(|(x, y)| after.pixel(x, y)).collect();
    let changed = after_pixels.iter().zip(&before_pixels).filter(|(a, b)| a != b).count();
    assert_eq!(changed, 0, "byColor changed {changed} live Pathfinder pixels: {result:?}");
    assert!(result.is_err(), "byColor must refuse a live Pathfinder operand group");
    let error = s.execute("imageTrace.makeAndExpand", &json!({"id": g.0, "params": {"ignoreWhite": true}, "byColor": true})).unwrap_err();
    assert!(error.to_string().contains("Pathfinder"), "{error}");
    assert_eq!(node(&s, g), original);
    assert_eq!(s.doc().unwrap().history.undo.len(), steps);
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
    assert_eq!(traced_paths(&g).len(), 4);
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
fn image_trace_document_library_fills_are_unlinked() {
    let mut s = session();
    // A global color, a tint of another global color and a process color.
    s.execute("swatch.new", &json!({"name": "Ink", "color": "#c82828", "global": true})).unwrap();
    s.execute("swatch.new", &json!({"name": "Sky", "color": "#2846c8", "global": true})).unwrap();
    s.execute("swatch.new", &json!({"swatch": "Sky", "tint": 50})).unwrap();
    s.execute("swatch.new", &json!({"name": "Leaf", "color": "#28a03c"})).unwrap();
    let hex_of = |s: &mut Session, name: &str| {
        let list = s.execute("swatch.list", &json!({})).unwrap();
        list["swatches"].as_array().unwrap().iter().find(|w| w["name"] == name).unwrap()["hex"].as_str().unwrap().to_lowercase()
    };
    let tint = hex_of(&mut s, "Sky 50%");
    let [tr, tg, tb] = [1, 3, 5].map(|i| u8::from_str_radix(&tint[i..i + 2], 16).unwrap());
    let r = vectorcraft_trace::Raster::from_fn(60, 60, |x, y| match (x < 30, y < 30) {
        (true, _) => [200, 40, 40, 255],
        (false, true) => [40, 160, 60, 255],
        (false, false) => [tr, tg, tb, 255],
    });
    let img = add_image(&mut s, &r, Affine::IDENTITY);
    let params = json!({"mode": "color", "palette": "documentLibrary", "library": "document", "colors": 3});
    // Each fill as (its color, its swatch link, its tint), one per color.
    let fills = |s: &Session, id: &Value| {
        let mut v: Vec<(String, Option<String>, f32)> = node(s, NodeId(id.as_u64().unwrap()))
            .children()
            .unwrap()
            .iter()
            .filter_map(|n| match n.appearance.fill_paint() {
                Paint::Solid { color, swatch, tint } => {
                    let [r, g, b, _] = color.to_rgba8(1.0);
                    Some((format!("#{r:02x}{g:02x}{b:02x}"), swatch, tint))
                }
                _ => None,
            })
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v.dedup();
        v
    };
    let fill = |hex: &str, swatch: Option<&str>, tint: f32| (hex.to_string(), swatch.map(String::from), tint);
    let res = s.execute("imageTrace.makeAndExpand", &json!({"params": params})).unwrap();
    let got = fills(&s, &res["id"]);
    assert_eq!(got.len(), 3, "{got:?}");
    assert!(got.contains(&fill("#c82828", None, 1.0)), "{got:?}");
    assert!(got.contains(&fill(&tint, None, 1.0)), "{got:?}");
    assert!(got.contains(&fill("#28a03c", None, 1.0)), "{got:?}");
    // Editing the global swatches leaves the traced colors unchanged.
    s.execute("swatch.edit", &json!({"name": "Ink", "color": "#ff8800"})).unwrap();
    s.execute("swatch.edit", &json!({"name": "Sky", "color": "#000000"})).unwrap();
    let got = fills(&s, &res["id"]);
    assert!(got.contains(&fill("#c82828", None, 1.0)), "{got:?}");
    assert!(got.contains(&fill(&tint, None, 1.0)), "{got:?}");
    assert!(got.contains(&fill("#28a03c", None, 1.0)), "{got:?}");
    for _ in 0..3 {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    // A color group's global swatches also fill without links.
    s.execute("swatch.newGroup", &json!({"name": "Brand", "swatches": ["Ink", "Leaf"]})).unwrap();
    let mut group = params.clone();
    group["colorGroup"] = json!("Brand");
    let res = s.execute("imageTrace.makeAndExpand", &json!({"id": img.0, "params": group})).unwrap();
    let got = fills(&s, &res["id"]);
    assert!(got.contains(&fill("#c82828", None, 1.0)) && got.contains(&fill("#28a03c", None, 1.0)), "{got:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    // A swatch library's colors stay unlinked.
    let mut web = params.clone();
    web["library"] = json!("web-safe-216");
    let res = s.execute("imageTrace.makeAndExpand", &json!({"id": img.0, "params": web})).unwrap();
    let got = fills(&s, &res["id"]);
    assert!(!got.is_empty() && got.iter().all(|f| f.1.is_none()), "{got:?}");
}

#[test]
fn image_trace_document_library_same_color_stays_unlinked_in_swatch_order() {
    let mut s = session();
    // A process swatch, then two global swatches, all of one color.
    s.execute("swatch.new", &json!({"name": "Red", "color": "#c82828"})).unwrap();
    s.execute("swatch.new", &json!({"name": "Ink", "color": "#c82828", "global": true})).unwrap();
    s.execute("swatch.new", &json!({"name": "Ink 2", "color": "#c82828", "global": true})).unwrap();
    s.execute("swatch.new", &json!({"name": "Leaf", "color": "#28a03c"})).unwrap();
    s.execute("swatch.new", &json!({"name": "Sky", "color": "#2846c8", "global": true})).unwrap();
    let [tr, tg, tb, _] = s.doc().unwrap().doc.tint_paint("Sky", 0.5).unwrap().color().unwrap().to_rgba8(1.0);
    let tint_hex = format!("#{tr:02x}{tg:02x}{tb:02x}");
    s.execute("swatch.new", &json!({"name": "Mist", "color": tint_hex})).unwrap();
    s.execute("swatch.new", &json!({"swatch": "Sky", "tint": 50})).unwrap();
    let r = vectorcraft_trace::Raster::from_fn(60, 60, |x, _| match x {
        0..20 => [200, 40, 40, 255],
        20..40 => [40, 160, 60, 255],
        _ => [tr, tg, tb, 255],
    });
    add_image(&mut s, &r, Affine::IDENTITY);
    let params = json!({"mode": "color", "palette": "documentLibrary", "library": "document", "colors": 8});
    let res = s.execute("imageTrace.makeAndExpand", &json!({"params": params})).unwrap();
    let g = node(&s, NodeId(res["id"].as_u64().unwrap()));
    let got: Vec<Paint> = g.children().unwrap().iter().map(|n| n.appearance.fill_paint()).collect();
    let red = Color::rgb8(200, 40, 40);
    let reds: Vec<&Paint> = got.iter().filter(|p| p.color().map(|c| c.to_rgba8(1.0)) == Some(red.to_rgba8(1.0))).collect();
    assert!(!reds.is_empty(), "{got:?}");
    assert!(reds.iter().all(|p| matches!(p, Paint::Solid { swatch: None, .. })), "{got:?}");
    let tints: Vec<&Paint> = got.iter().filter(|p| p.color().map(|c| c.to_rgba8(1.0)) == Some([tr, tg, tb, 255])).collect();
    assert!(!tints.is_empty(), "{got:?}");
    assert!(tints.iter().all(|p| matches!(p, Paint::Solid { swatch: None, .. })), "{got:?}");
    // Editing the global swatch leaves the trace unchanged.
    s.execute("swatch.edit", &json!({"name": "Ink", "color": "#ff8800"})).unwrap();
    let g = node(&s, NodeId(res["id"].as_u64().unwrap()));
    let orange = Color::rgb8(255, 136, 0).to_rgba8(1.0);
    assert!(!g.children().unwrap().iter().any(|n| n.appearance.fill_paint().color().map(|c| c.to_rgba8(1.0)) == Some(orange)));
    assert_eq!(g.children().unwrap().iter().map(|n| n.appearance.fill_paint()).collect::<Vec<_>>(), got);
}

#[test]
fn image_trace_document_library_same_color_is_unlinked_in_both_swatch_orders() {
    let trace_fill = |global_first: bool| {
        let mut s = session();
        for global in [global_first, !global_first] {
            s.execute("swatch.new", &json!({"name": if global { "Ink" } else { "Red" }, "color": "#c82828", "global": global})).unwrap();
        }
        let r = vectorcraft_trace::Raster::from_fn(60, 60, |_, _| [200, 40, 40, 255]);
        add_image(&mut s, &r, Affine::IDENTITY);
        let res = s
            .execute(
                "imageTrace.makeAndExpand",
                &json!({"params": {"mode": "color", "palette": "documentLibrary", "library": "document", "colors": 2}}),
            )
            .unwrap();
        let g = node(&s, NodeId(res["id"].as_u64().unwrap()));
        let fills: Vec<Paint> = g.children().unwrap().iter().map(|n| n.appearance.fill_paint()).collect();
        assert!(!fills.is_empty());
        assert!(fills.iter().all(|p| matches!(p, Paint::Solid { swatch: None, .. })), "{fills:?}");
        fills
    };
    let process_first = trace_fill(false);
    let global_first = trace_fill(true);
    assert_eq!(process_first, global_first);
    assert!(process_first.iter().all(|p| p.color() == Some(Color::rgb8(200, 40, 40))));
}

#[test]
fn image_trace_document_library_matches_a_lab_spot_by_its_cmyk_equivalent() {
    let _g = crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    // A Lab color outside the CMYK gamut, whose CMYK equivalent shows another color.
    let rgb = |c: Color| {
        let [r, g, b, _] = c.to_rgba8(1.0);
        [r, g, b]
    };
    let cms = vectorcraft_color::cms::active();
    let off = |lab: Color| {
        let cmyk = cms.convert(&lab, vectorcraft_color::cms::Model::Cmyk, cms.settings().intent);
        rgb(lab).iter().zip(rgb(cmyk)).map(|(a, b)| u32::from(a.abs_diff(b))).sum::<u32>()
    };
    let lab = [(30.0, 60.0, -100.0), (60.0, -80.0, 60.0), (55.0, 60.0, 40.0), (90.0, -40.0, 100.0)]
        .map(|(l, a, b)| Color::Lab { l, a, b })
        .into_iter()
        .max_by_key(|c| off(*c))
        .unwrap();
    assert!(off(lab) >= 30, "{lab:?} is off by {}", off(lab));
    let Color::Lab { l, a, b } = lab else { panic!("{lab:?}") };
    // A spot color of it shown by its CMYK equivalent (Spot Colors options with CMYK values).
    s.execute("swatch.new", &json!({"name": "Lab Ink", "color": {"l": l, "a": a, "b": b}, "spot": true})).unwrap();
    s.execute("swatch.spotOptions", &json!({"useLab": false})).unwrap();
    let shown = s.doc().unwrap().doc.global_color("Lab Ink").unwrap();
    let [kr, kg, kb] = rgb(shown);
    let [lr, lg, lb] = rgb(lab);
    // A process color between the two, nearer the color the spot shows.
    let near = |k: u8, l: u8| (f32::from(k) + (f32::from(l) - f32::from(k)) * 0.3).round() as u8;
    let [pr, pg, pb] = [near(kr, lr), near(kg, lg), near(kb, lb)];
    s.execute("swatch.new", &json!({"name": "Near", "color": format!("#{pr:02x}{pg:02x}{pb:02x}")})).unwrap();
    let process = Color::rgb8(pr, pg, pb);
    s.execute("swatch.new", &json!({"name": "Leaf", "color": "#28a03c"})).unwrap();
    // Art in the color the spot shows traces to the spot.
    let r = vectorcraft_trace::Raster::from_fn(60, 60, |x, _| if x < 30 { [kr, kg, kb, 255] } else { [40, 160, 60, 255] });
    add_image(&mut s, &r, Affine::IDENTITY);
    let params = json!({"mode": "color", "palette": "documentLibrary", "library": "document", "colors": 3});
    let res = s.execute("imageTrace.makeAndExpand", &json!({"params": params})).unwrap();
    let g = node(&s, NodeId(res["id"].as_u64().unwrap()));
    let got: Vec<Paint> = g.children().unwrap().iter().map(|n| n.appearance.fill_paint()).collect();
    let spot = Paint::solid(shown);
    assert!(got.contains(&spot), "{got:?}");
    assert!(!got.iter().any(|p| p.color() == Some(process)), "{got:?}");
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

/// Manage Presets: a saved preset is listed after the built-in ones, traces by name in any case,
/// changes from its own settings, and can be renamed and deleted.
#[test]
fn image_trace_saved_presets_are_made_used_renamed_and_deleted() {
    let mut s = session();
    let img = image_doc(&mut s);
    let r = s.execute("imageTrace.presets.save", &json!({"name": "Disc Only", "params": {"ignoreWhite": true, "paths": 70}})).unwrap();
    assert_eq!((r["name"].as_str(), r["created"].as_bool(), r["params"]["ignoreWhite"].as_bool()), (Some("Disc Only"), Some(true), Some(true)));
    let list = s.execute("imageTrace.presets", &json!({})).unwrap();
    let list = list["presets"].as_array().unwrap();
    assert_eq!(list.len(), 16);
    assert!(list[..15].iter().all(|p| p["builtIn"] == true));
    assert_eq!((list[15]["name"].as_str(), list[15]["builtIn"].as_bool()), (Some("Disc Only"), Some(false)));
    assert!(list[15]["enhanced"].is_null());
    assert_eq!(list[15]["params"]["paths"], 70.0);
    // It traces by name, any case, and the object shows its saved name.
    let r = s.execute("imageTrace.make", &json!({"id": img.0, "preset": "disc only"})).unwrap();
    assert_eq!(r["paths"], 1, "the white is left out");
    let g = NodeId(r["id"].as_u64().unwrap());
    let t = node(&s, g).extras().trace.clone().unwrap();
    assert_eq!((t["preset"].as_str(), t["params"]["ignoreWhite"].as_bool()), (Some("Disc Only"), Some(true)));
    let r = s.execute("imageTrace.make", &json!({"id": g.0, "preset": "Disc Only", "params": {"paths": 20}})).unwrap();
    let g = NodeId(r["id"].as_u64().unwrap());
    assert_eq!(node(&s, g).extras().trace.clone().unwrap()["preset"], "Custom");
    // Saving it again changes it from its own settings.
    let r = s.execute("imageTrace.presets.save", &json!({"name": "disc only", "params": {"corners": 10}})).unwrap();
    assert_eq!((r["name"].as_str(), r["created"].as_bool()), (Some("Disc Only"), Some(false)));
    assert_eq!(
        (r["params"]["ignoreWhite"].as_bool(), r["params"]["paths"].as_f64(), r["params"]["corners"].as_f64()),
        (Some(true), Some(70.0), Some(10.0))
    );
    // Rename: the old name no longer traces.
    s.execute("imageTrace.presets.save", &json!({"name": "Light", "preset": "6 Colors"})).unwrap();
    let e = s.execute("imageTrace.presets.save", &json!({"name": "Disc Only", "newName": "light"})).unwrap_err().to_string();
    assert!(e.contains("a preset named `light` exists"), "{e}");
    assert!(s.execute("imageTrace.presets.save", &json!({"name": "Disc Only", "newName": "6 colors"})).is_err());
    let e = s.execute("imageTrace.presets.save", &json!({"name": "Nothing", "newName": "Something"})).unwrap_err().to_string();
    assert!(e.contains("no saved preset named `Nothing` to rename"), "{e}");
    let r = s.execute("imageTrace.presets.save", &json!({"name": "Disc Only", "newName": "Disc"})).unwrap();
    assert_eq!((r["name"].as_str(), r["params"]["corners"].as_f64()), (Some("Disc"), Some(10.0)));
    let e = s.execute("imageTrace.make", &json!({"id": g.0, "preset": "Disc Only"})).unwrap_err().to_string();
    assert!(e.contains("unknown preset `Disc Only`: not a built-in or saved preset"), "{e}");
    assert_eq!(s.execute("imageTrace.make", &json!({"id": g.0, "preset": "Disc"})).unwrap()["paths"], 1);
    // Delete.
    assert_eq!(s.execute("imageTrace.presets.delete", &json!({"name": "DISC"})).unwrap()["deleted"], "Disc");
    assert!(s.execute("imageTrace.presets.delete", &json!({"name": "Disc"})).is_err());
    assert!(s.execute("imageTrace.make", &json!({"id": g.0, "preset": "Disc"})).is_err());
    let names: Vec<String> = s.prefs.trace_presets.iter().map(|p| p.name.clone()).collect();
    assert_eq!(names, ["Light"]);
}

/// The saved presets' revision grows with each save, rename and delete, and not with a refused
/// one; the changes are recorded, and the list and the save's reply carry them.
#[test]
fn image_trace_saved_presets_count_their_changes() {
    let mut s = session();
    let revision = |s: &mut Session| s.execute("imageTrace.presets", &json!({})).unwrap()["revision"].as_u64();
    assert_eq!((s.trace_presets_revision(), revision(&mut s)), (0, Some(0)));
    let r = s.execute("imageTrace.presets.save", &json!({"name": "A"})).unwrap();
    assert!(r["renamedFrom"].is_null());
    let r = s.execute("imageTrace.presets.save", &json!({"name": "a", "newName": "B", "params": {"paths": 30}})).unwrap();
    assert_eq!(r["renamedFrom"], "A");
    assert!(s.execute("imageTrace.presets.save", &json!({"name": "B", "newName": "Default"})).is_err());
    assert!(s.execute("imageTrace.presets.delete", &json!({"name": "A"})).is_err());
    assert!(s.execute("imageTrace.presets.save", &json!({"name": "b", "newName": "B"})).unwrap()["renamedFrom"].is_null());
    s.execute("imageTrace.presets.delete", &json!({"name": "B"})).unwrap();
    assert_eq!((s.trace_presets_revision(), revision(&mut s)), (4, Some(4)));
    let changes: Vec<_> = s.trace_preset_changes_since(1).map(|c| (c.revision, c.name.as_str(), c.renamed_from.as_deref(), c.deleted)).collect();
    assert_eq!(changes, [(2, "B", Some("A"), false), (3, "B", None, false), (4, "B", None, true)]);
    // Preferences with other saved presets give a new revision; the same ones don't.
    let mut prefs = s.prefs.clone();
    s.apply_prefs(prefs.clone());
    assert_eq!(s.trace_presets_revision(), 4);
    s.execute("imageTrace.presets.save", &json!({"name": "C"})).unwrap();
    prefs.trace_presets.clear();
    s.apply_prefs(prefs);
    assert_eq!(s.trace_presets_revision(), 6);
    assert_eq!(s.trace_preset_changes_since(4).count(), 1, "replacing the preferences records no change");
    // A rename stays on record after many later saves.
    s.execute("imageTrace.presets.save", &json!({"name": "C"})).unwrap();
    let seen = s.trace_presets_revision();
    s.execute("imageTrace.presets.save", &json!({"name": "C", "newName": "D"})).unwrap();
    for i in 0..40 {
        s.execute("imageTrace.presets.save", &json!({"name": format!("E{i}")})).unwrap();
    }
    let first = s.trace_preset_changes_since(seen).next().map(|c| (c.name.clone(), c.renamed_from.clone()));
    assert_eq!(first, Some(("D".to_string(), Some("C".to_string()))));
    assert_eq!(s.trace_preset_changes_since(seen).count(), 41);
}

/// Built-in presets stay as they are, and a saved preset's settings are checked as imageTrace.make
/// checks them.
#[test]
fn image_trace_built_in_presets_cannot_be_saved_over_or_deleted() {
    let mut s = session();
    for name in ["6 Colors", "[Default]", "flat logo", "Custom"] {
        let e = s.execute("imageTrace.presets.save", &json!({ "name": name })).unwrap_err().to_string();
        assert!(e.contains("save it under another name") || e.contains("save the preset under another name"), "{name}: {e}");
    }
    for name in ["Default", "outline"] {
        let e = s.execute("imageTrace.presets.delete", &json!({ "name": name })).unwrap_err().to_string();
        assert!(e.contains("is a built-in preset and stays"), "{e}");
    }
    assert!(s.execute("imageTrace.presets.save", &json!({"name": " "})).is_err());
    assert!(s.execute("imageTrace.presets.delete", &json!({})).is_err());
    let e = s.execute("imageTrace.presets.save", &json!({"params": {"fills": false, "strokes": false}})).unwrap_err().to_string();
    assert!(e.contains("`imageTrace.presets.save`") && e.contains("Create needs fills, strokes or both"), "{e}");
    assert!(s.execute("imageTrace.presets.save", &json!({"preset": "No Such"})).is_err());
    assert!(s.prefs.trace_presets.is_empty(), "nothing saved");
    // Without a name, the first free "Tracing Preset N"; a preset and its enhanced version as a start.
    let r = s.execute("imageTrace.presets.save", &json!({"preset": "16 Colors", "enhanced": true})).unwrap();
    assert_eq!(r["name"], "Tracing Preset 1");
    assert_eq!(r["params"], serde_json::to_value(vectorcraft_trace::enhanced_preset("16 Colors").unwrap()).unwrap());
    assert_eq!(s.execute("imageTrace.presets.save", &json!({})).unwrap()["name"], "Tracing Preset 2");
    // Built-in presets come first in the list and are unchanged.
    let list = s.execute("imageTrace.presets", &json!({})).unwrap();
    let six = list["presets"].as_array().unwrap().iter().find(|p| p["name"] == "6 Colors").unwrap().clone();
    assert_eq!(six["params"], serde_json::to_value(vectorcraft_trace::preset("6 Colors").unwrap()).unwrap());
}

/// Saved tracing presets are kept with the preferences, are not a preference key, and survive
/// resetting the preferences.
#[test]
fn image_trace_saved_presets_persist_with_the_preferences() {
    assert!(Prefs::default().to_json().get("tracePresets").is_none(), "nothing saved while there are none");
    assert!(serde_json::from_value::<Prefs>(json!({"keyboardIncrement": 2.0})).unwrap().trace_presets.is_empty());
    let mut s = session();
    s.execute("imageTrace.presets.save", &json!({"name": "Keep", "preset": "3 Colors", "params": {"noise": 40}})).unwrap();
    let back: Prefs = serde_json::from_str(&serde_json::to_string(&s.prefs).unwrap()).unwrap();
    assert_eq!(back.trace_presets, s.prefs.trace_presets);
    let mut t = session();
    t.apply_prefs(back);
    image_doc(&mut t);
    let r = t.execute("imageTrace.make", &json!({"preset": "keep"})).unwrap();
    let t_node = node(&t, NodeId(r["id"].as_u64().unwrap()));
    let stored = t_node.extras().trace.clone().unwrap();
    assert_eq!(
        (stored["preset"].as_str(), stored["params"]["noise"].as_u64(), stored["params"]["colors"].as_u64()),
        (Some("Keep"), Some(40), Some(3))
    );
    assert!(t.execute("prefs.set", &json!({"key": "tracePresets", "value": []})).is_err());
    t.execute("prefs.reset", &json!({})).unwrap();
    assert_eq!(t.prefs.trace_presets.len(), 1);
    assert_eq!(t.execute("prefs.get", &json!({})).unwrap()["prefs"]["tracePresets"][0]["name"], "Keep");
}

/// A preset or name that isn't a string is an error, and changes no saved preset.
#[test]
fn image_trace_presets_reject_names_that_are_not_strings() {
    let mut s = session();
    let img = image_doc(&mut s);
    s.execute("imageTrace.presets.save", &json!({"name": "Keep", "preset": "3 Colors"})).unwrap();
    let kept = s.prefs.trace_presets.clone();
    for p in [json!({"name": "Keep", "preset": 123}), json!({"name": 7}), json!({"name": "Keep", "newName": 5}), json!({"name": ["Keep"]})] {
        let e = s.execute("imageTrace.presets.save", &p).unwrap_err().to_string();
        assert!(e.contains("must be a string"), "{p}: {e}");
    }
    assert_eq!(s.prefs.trace_presets, kept, "nothing saved or changed");
    let e = s.execute("imageTrace.presets.delete", &json!({"name": 1})).unwrap_err().to_string();
    assert!(e.contains("`name` must be a string"), "{e}");
    let e = s.execute("imageTrace.make", &json!({"id": img.0, "preset": 3})).unwrap_err().to_string();
    assert!(e.contains("`preset` must be a string"), "{e}");
}

/// The journal records a saved preset's settings, so a replay traces with them after the preset is
/// changed or deleted.
#[test]
fn image_trace_replays_a_saved_preset_with_the_settings_it_had() {
    let mut s = session();
    let img = image_doc(&mut s);
    s.execute("imageTrace.presets.save", &json!({"name": "Keep", "params": {"ignoreWhite": true}})).unwrap();
    assert_eq!(s.execute("imageTrace.make", &json!({"id": img.0, "preset": "Keep"})).unwrap()["paths"], 1, "the white is left out");
    let (cmd, p) = s.journal.last().unwrap().clone();
    assert_eq!((cmd.as_str(), &p["savedPreset"]["ignoreWhite"]), ("imageTrace.make", &json!(true)));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("imageTrace.presets.save", &json!({"name": "Keep", "params": {"ignoreWhite": false}})).unwrap();
    assert_ne!(s.execute("imageTrace.make", &json!({"id": img.0, "preset": "Keep"})).unwrap()["paths"], 1);
    s.execute("edit.undo", &json!({})).unwrap();
    // Changed: the replay traces as the first run did, and the object no longer names Keep.
    let r = s.execute(&cmd, &p).unwrap();
    assert_eq!(r["paths"], 1);
    assert_eq!(node(&s, NodeId(r["id"].as_u64().unwrap())).extras().trace.clone().unwrap()["preset"], "Custom");
    s.execute("edit.undo", &json!({})).unwrap();
    // Deleted.
    s.execute("imageTrace.presets.delete", &json!({"name": "Keep"})).unwrap();
    assert_eq!(s.execute(&cmd, &p).unwrap()["paths"], 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.execute("imageTrace.make", &json!({"id": img.0, "preset": "Keep", "savedPreset": 5})).is_err());
    // Recorded settings are checked as `params` are.
    let e = s
        .execute("imageTrace.make", &json!({"id": img.0, "preset": "Keep", "savedPreset": {"fills": false, "strokes": false}}))
        .unwrap_err()
        .to_string();
    assert!(e.contains("Create needs fills, strokes or both"), "{e}");
    // A built-in preset records nothing.
    s.execute("imageTrace.make", &json!({"id": img.0, "preset": "6 Colors"})).unwrap();
    assert!(s.journal.last().unwrap().1.get("savedPreset").is_none());
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
fn image_trace_flat_logo_can_trace_only_the_named_colors() {
    // A light disc in a black ring, a blue square on it, and a white spot.
    let art = vectorcraft_trace::Raster::from_fn(120, 120, |x, y| {
        let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
        let d = (fx - 60.0).hypot(fy - 60.0);
        if (fx - 32.0).hypot(fy - 40.0) <= 6.0 {
            [255, 255, 255, 255]
        } else if (45..75).contains(&x) && (45..75).contains(&y) {
            [40, 120, 240, 255]
        } else if d <= 46.0 {
            [240, 240, 220, 255]
        } else if d <= 50.0 {
            [0, 0, 0, 255]
        } else {
            [0, 0, 0, 0]
        }
    });
    let fills = |logo_white: bool| {
        let mut s = session();
        add_image(&mut s, &art, Affine::IDENTITY);
        let params = json!({"params": {"mode": "logo", "logoColors": ["#000000", "#F0F0DC", "#2878F0"], "logoWhite": logo_white}});
        let r = s.execute("imageTrace.makeAndExpand", &params).unwrap();
        let g = node(&s, NodeId(r["id"].as_u64().unwrap()));
        let mut hex: Vec<String> = g.children().unwrap().iter().filter_map(|n| n.appearance.fill_paint().color()).map(|c| c.to_hex()).collect();
        hex.sort();
        hex.dedup();
        (r["colors"].clone(), hex)
    };
    let (colors, hex) = fills(true);
    assert_eq!(colors, 4);
    assert!(hex.contains(&"#ffffff".to_string()), "{hex:?}");
    let (colors, hex) = fills(false);
    assert_eq!(colors, 3);
    assert_eq!(hex, ["#000000", "#2878f0", "#f0f0dc"]);
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

/// Outline views draw the traced paths' outlines and leave out an Abutting trace's seam fills
/// (their clipping box and strokes, or on a stretched image the strokes' filled outlines), renamed
/// too.
#[test]
fn image_trace_outline_view_leaves_out_the_seam_fills() {
    // Red beside blue in the middle of a transparent 60 × 40 image, placed at 50, 50 and scaled 2×,
    // or 2× across and 2.5× down.
    let art = vectorcraft_trace::Raster::from_fn(60, 40, |x, y| match ((10..50).contains(&x) && (10..30).contains(&y), x < 30) {
        (false, _) => [0, 0, 0, 0],
        (true, true) => [200, 30, 30, 255],
        (true, false) => [30, 30, 200, 255],
    });
    for (scale, stretched) in [(Affine::scale(2.0), false), (Affine::scale_non_uniform(2.0, 2.5), true)] {
        let mut s = session();
        add_image(&mut s, &art, Affine::translate((50.0, 50.0)) * scale);
        let r = s.execute("imageTrace.make", &json!({"params": {"mode": "color", "palette": "limited", "colors": 2, "method": "abutting"}})).unwrap();
        let g = NodeId(r["id"].as_u64().unwrap());
        let seams =
            node(&s, g).children().unwrap().iter().find(|c| matches!(c.kind, NodeKind::Group { clip: true, .. })).cloned().expect("seam fills");
        // Strokes, or on the stretched image their outlines, filled.
        let members = &seams.children().unwrap()[1..];
        assert!(!members.is_empty());
        for m in members {
            let (fill, stroke) = (m.appearance.fill_paint(), m.appearance.stroke_paint());
            assert_eq!((fill != Paint::None, stroke != Paint::None), (stretched, !stretched), "{scale:?}: {fill:?}, {stroke:?}");
        }
        s.execute("imageTrace.setView", &json!({"id": g.0, "view": "outlines"})).unwrap();
        let screen = render_doc(&s, true);
        // The image's top edge (y = 50), away from the art: nothing drawn.
        for x in [60, 110, 160] {
            assert!(screen.pixel(x, 50)[0] > 245 && screen.pixel(x, 51)[0] > 245, "{scale:?}, x = {x}: {:?}", screen.pixel(x, 50));
        }
        // The art's left edge (x = 70) is outlined.
        assert!((68..=72).any(|x| screen.pixel(x, 90)[0] < 128), "{scale:?}");
        // Beside the red and blue paths' shared edge (x = 110), inside the seam band (1.5 units
        // each side on the stretched image) but off the traced outlines (x = 109 and 110): nothing.
        for x in [108, 111] {
            assert!(screen.pixel(x, 100)[0] > 245, "{scale:?}, x = {x}: {:?}", screen.pixel(x, 100));
        }
        // Renamed, the group is still left out: the view draws as it did.
        s.execute("layer.setProps", &json!({"id": seams.id.0, "name": "Edges"})).unwrap();
        assert!(render_doc(&s, true).pixels == screen.pixels, "{scale:?}: renamed seam fills drawn");
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
