//! Tests for Shape Builder, Live Paint and Image Trace commands and tools.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, ImageBlob, ImageObject, LineCap, LineJoin, Node, NodeKind};
use vectorcraft_geom::{Affine, FillRule, PathData, Rect};
use vectorcraft_testkit::pdf::{PdfPage, pdf};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use super::*;

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
    let a = rect(s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(s, 50.0, 0.0, 100.0, 100.0);
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
    let r = s.execute("shapeBuilder.merge", &json!({"points": [[50, 70]]})).unwrap();
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert!((area(&s, merged) - 900.0).abs() < 1e-6, "{}", area(&s, merged));
    assert_eq!(node(&s, merged).appearance.fill_paint(), s.paint.fill, "the current fill");
    assert!(!node(&s, merged).appearance.stroke_paint().is_none(), "the lines' stroke");
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
    let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient { kind: GradientKind::Linear, stops })));
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
                NodeKind::Image(Box::new(ImageObject { key: "img1".into(), width: w, height: h, xf, link: None, placement: Default::default() })),
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
