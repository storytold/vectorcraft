//! Pathfinder on clip groups (#8093): a clip group's members take part only where its clipping
//! path shows them, so Divide and the other operations leave out the hidden art.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::NodeKind;
use vectorcraft_geom::{FillRule, PathData, Rect};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap_or_else(|| panic!("no id in {v}")))
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64, fill: Color) -> NodeId {
    let r = id(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap());
    s.edit("fill", |d, _| {
        d.node_mut(r).unwrap().appearance.set_fill(Paint::solid(fill));
        Ok(())
    })
    .unwrap();
    r
}

fn clip(s: &mut Session, ids: &[NodeId]) -> NodeId {
    id(&s.execute("object.clippingMask.make", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap())
}

fn red() -> Color {
    Color::rgb(1.0, 0.0, 0.0)
}

fn blue() -> Color {
    Color::rgb(0.0, 0.0, 1.0)
}

fn green() -> Color {
    Color::rgb(0.0, 1.0, 0.0)
}

/// A red square at (0, 0) and a blue one at (50, 50), both 100 pt, clipped by a 100 pt square
/// mask at (25, 25): the clip group. The mask shows 8750 pt² of them.
fn clipped_pair(s: &mut Session) -> NodeId {
    let a = rect(s, 0.0, 0.0, 100.0, 100.0, red());
    let b = rect(s, 50.0, 50.0, 100.0, 100.0, blue());
    let mask = rect(s, 25.0, 25.0, 100.0, 100.0, green());
    clip(s, &[a, b, mask])
}

/// The shapes `op` makes of the selection, the members of the group it makes or the one shape:
/// (fill, path, rule) each.
fn pieces(s: &mut Session, op: &str) -> Vec<(Paint, PathData, FillRule)> {
    let r = s.execute(op, &json!({})).unwrap();
    let g = s.doc().unwrap().doc.node(NodeId(r["ids"][0].as_u64().unwrap())).cloned().unwrap();
    let shape = |n: &vectorcraft_doc::Node| match &n.kind {
        NodeKind::Path { path, rule, .. } => (n.appearance.fill_paint(), path.clone(), *rule),
        NodeKind::Compound { children, rule } => {
            let subs = children.iter().flat_map(|c| c.path_data().unwrap().subpaths.clone()).collect();
            (n.appearance.fill_paint(), PathData::new(subs), *rule)
        }
        k => panic!("not a path: {k:?}"),
    };
    match &g.kind {
        NodeKind::Group { children, .. } => children.iter().map(|c| shape(c)).collect(),
        _ => vec![shape(&g)],
    }
}

fn area(p: &(Paint, PathData, FillRule)) -> f64 {
    vectorcraft_pathops::area(&p.1, p.2)
}

fn bounds(ps: &[(Paint, PathData, FillRule)]) -> Rect {
    ps.iter().filter_map(|p| p.1.bounds()).reduce(|a, b| a.union(b)).unwrap()
}

fn select(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn assert_inside(r: Rect, within: Rect) {
    assert!(within.inflate(1e-6, 1e-6).contains_rect(r), "{r:?} reaches out of {within:?}");
}

/// The issue's case: Divide on a clip group gives the pieces the mask shows, cut along it, and the
/// hidden art goes.
#[test]
fn divide_on_a_clip_group_keeps_only_what_the_mask_shows() {
    let mut s = session();
    let g = clipped_pair(&mut s);
    select(&mut s, &[g]);
    let ps = pieces(&mut s, "object.pathfinder.divide");
    assert_eq!(ps.len(), 3, "red alone, the overlap and blue alone: {ps:?}");
    let total: f64 = ps.iter().map(area).sum();
    assert!((total - 8750.0).abs() < 1.0, "the visible area, not the whole art: {total}");
    assert_inside(bounds(&ps), Rect::new(25.0, 25.0, 125.0, 125.0));
    let blue: f64 = ps.iter().filter(|p| p.0 == Paint::solid(blue())).map(area).sum();
    assert!((blue - 5625.0).abs() < 1.0, "the overlap takes the front (blue) fill: {blue}");
    assert!(ps.iter().all(|p| p.0 != Paint::solid(green())), "the clipping path adds no piece: {ps:?}");
}

/// An irregular mask: a square clipped by a circle divides into a disc.
#[test]
fn divide_cuts_along_a_curved_clipping_path() {
    let mut s = session();
    let square = rect(&mut s, 100.0, 100.0, 300.0, 300.0, red());
    let circle = id(&s.execute("shape.ellipse", &json!({"x": 150, "y": 150, "width": 200, "height": 200})).unwrap());
    let g = clip(&mut s, &[square, circle]);
    select(&mut s, &[g]);
    let ps = pieces(&mut s, "object.pathfinder.divide");
    assert_eq!(ps.len(), 1, "{ps:?}");
    let disc = std::f64::consts::PI * 100.0 * 100.0;
    assert!((area(&ps[0]) - disc).abs() < disc * 1e-3, "{} vs {disc}", area(&ps[0]));
    assert_inside(bounds(&ps), Rect::new(150.0, 150.0, 350.0, 350.0));
}

/// A clip group inside a clip group: its members show only where both masks do.
#[test]
fn nested_clip_groups_cut_by_both_masks() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 200.0, 200.0, red());
    let inner_mask = rect(&mut s, 0.0, 0.0, 100.0, 200.0, green());
    let inner = clip(&mut s, &[a, inner_mask]);
    let outer_mask = rect(&mut s, 0.0, 50.0, 200.0, 100.0, green());
    let outer = clip(&mut s, &[inner, outer_mask]);
    select(&mut s, &[outer]);
    let ps = pieces(&mut s, "object.pathfinder.divide");
    assert_eq!(ps.len(), 1, "{ps:?}");
    assert!((area(&ps[0]) - 10000.0).abs() < 1.0, "100 × 100 shows: {}", area(&ps[0]));
    assert_inside(bounds(&ps), Rect::new(0.0, 50.0, 100.0, 150.0));
}

/// A member the mask hides altogether gives no piece.
#[test]
fn a_member_the_mask_hides_gives_no_piece() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0, red());
    let b = rect(&mut s, 300.0, 300.0, 100.0, 100.0, blue());
    let mask = rect(&mut s, 0.0, 0.0, 150.0, 150.0, green());
    let g = clip(&mut s, &[a, b, mask]);
    select(&mut s, &[g]);
    let ps = pieces(&mut s, "object.pathfinder.divide");
    assert_eq!(ps.len(), 1, "only the red square shows: {ps:?}");
    assert_eq!(ps[0].0, Paint::solid(red()));
}

/// A cutting line in a clip group cuts only where the mask shows it: its hidden part leaves the
/// other selected art alone.
#[test]
fn a_clipped_cutting_line_cuts_only_where_it_shows() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0, red());
    let line = id(&s.execute("shape.line", &json!({"x1": 50, "y1": -50, "x2": 50, "y2": 400})).unwrap());
    let mask = rect(&mut s, 0.0, 0.0, 100.0, 100.0, green());
    let g = clip(&mut s, &[a, line, mask]);
    let other = rect(&mut s, 0.0, 200.0, 100.0, 100.0, blue());
    select(&mut s, &[g, other]);
    let ps = pieces(&mut s, "object.pathfinder.divide");
    let red = ps.iter().filter(|p| p.0 == Paint::solid(red())).count();
    let blue = ps.iter().filter(|p| p.0 == Paint::solid(blue())).count();
    assert_eq!((red, blue), (2, 1), "the line halves the red square, not the blue one: {ps:?}");
}

/// Every Shape Mode and Pathfinder button takes the members as the mask shows them (#3367 asks
/// this of Merge). The mask, 100 × 60 pt at (25, 25), cuts through the squares' overlap: it shows
/// 4500 pt² of red, 2625 pt² of blue and 1750 pt² of their overlap.
#[test]
fn every_operation_takes_only_what_the_mask_shows() {
    for (op, want) in [
        ("unite", Some(5375.0)),
        ("minusFront", Some(2750.0)),
        ("minusBack", Some(875.0)),
        ("intersect", Some(1750.0)),
        ("exclude", Some(3625.0)),
        ("trim", Some(5375.0)),
        ("merge", Some(5375.0)),
        ("crop", Some(1750.0)),
        ("outline", None),
    ] {
        let mut s = session();
        let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0, red());
        let b = rect(&mut s, 50.0, 50.0, 100.0, 100.0, blue());
        let mask = rect(&mut s, 25.0, 25.0, 100.0, 60.0, green());
        let g = clip(&mut s, &[a, b, mask]);
        select(&mut s, &[g]);
        let ps = pieces(&mut s, &format!("object.pathfinder.{op}"));
        assert_inside(bounds(&ps), Rect::new(25.0, 25.0, 125.0, 85.0));
        if let Some(want) = want {
            let total: f64 = ps.iter().map(area).sum();
            assert!((total - want).abs() < 1.0, "{op}: {total}, want {want}");
        }
    }
}

/// Among other objects a clip group acts as one shape: the union of what its mask shows.
#[test]
fn a_clip_group_among_other_objects_is_the_area_its_mask_shows() {
    let mut s = session();
    let g = clipped_pair(&mut s);
    let other = rect(&mut s, 300.0, 0.0, 50.0, 50.0, blue());
    select(&mut s, &[g, other]);
    let ps = pieces(&mut s, "object.pathfinder.unite");
    let total: f64 = ps.iter().map(area).sum();
    assert!((total - (8750.0 + 2500.0)).abs() < 1.0, "{total}");
}

/// Among other objects a clip group takes the style of its top member that shows: a member the
/// mask hides paints nothing.
#[test]
fn a_hidden_top_member_gives_no_style() {
    let mut s = session();
    // Below the clip group, so the result takes the group's style.
    let other = rect(&mut s, 50.0, 50.0, 100.0, 100.0, green());
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0, red());
    let b = rect(&mut s, 300.0, 300.0, 100.0, 100.0, blue());
    let mask = rect(&mut s, 0.0, 0.0, 150.0, 150.0, green());
    let g = clip(&mut s, &[a, b, mask]);
    select(&mut s, &[other, g]);
    let ps = pieces(&mut s, "object.pathfinder.unite");
    assert_eq!(ps.len(), 1, "{ps:?}");
    assert_eq!(ps[0].0, Paint::solid(red()), "not the hidden blue member's fill");
    assert!((area(&ps[0]) - 17500.0).abs() < 1.0, "{}", area(&ps[0]));
}

/// A selection the masks hide altogether says so.
#[test]
fn art_the_masks_hide_altogether_is_refused_with_a_reason() {
    let mut s = session();
    let a = rect(&mut s, 300.0, 300.0, 100.0, 100.0, red());
    let mask = rect(&mut s, 0.0, 0.0, 100.0, 100.0, green());
    let g = clip(&mut s, &[a, mask]);
    select(&mut s, &[g]);
    let undo = s.doc().unwrap().history.undo.len();
    let e = s.execute("object.pathfinder.divide", &json!({})).unwrap_err().to_string();
    assert!(e.contains("clipping masks hide"), "{e}");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "nothing changes");
}

/// A clip group its mask hides altogether, selected with a red square at (0, 0) and a blue one at
/// (50, 50), each 100 pt, at `position` in the stacking order (0: at the back).
fn with_a_hidden_clip_group(s: &mut Session, position: usize) {
    let mut ids = vec![];
    for i in 0..3 {
        if i == position {
            // Two members, so among other objects the group is one shape: their union.
            let a = rect(s, 300.0, 300.0, 100.0, 100.0, green());
            let b = rect(s, 450.0, 300.0, 100.0, 100.0, green());
            let mask = rect(s, 0.0, 0.0, 100.0, 100.0, green());
            ids.push(clip(s, &[a, b, mask]));
        } else {
            let at = if ids.len() == usize::from(position == 0) { 0.0 } else { 50.0 };
            ids.push(rect(s, at, at, 100.0, 100.0, if at == 0.0 { red() } else { blue() }));
        }
    }
    select(s, &ids);
}

/// A selected object its masks hide altogether is an empty operand: Intersect, Minus Front with
/// it at the back, Minus Back with it in front and Crop with it on top give nothing, and the other
/// operations make what they make without it.
#[test]
fn a_hidden_operand_is_an_empty_shape_where_it_changes_the_result() {
    for (op, position) in [("intersect", 1), ("minusFront", 0), ("minusBack", 2), ("crop", 2)] {
        let mut s = session();
        with_a_hidden_clip_group(&mut s, position);
        let undo = s.doc().unwrap().history.undo.len();
        let e = s.execute(&format!("object.pathfinder.{op}"), &json!({})).unwrap_err().to_string();
        assert!(e.contains("empty result"), "{op}: {e}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo, "{op}: nothing changes");
    }
    for (op, position, want) in [("unite", 2, 17500.0), ("exclude", 2, 15000.0), ("minusFront", 2, 7500.0), ("minusBack", 0, 7500.0)] {
        let mut s = session();
        with_a_hidden_clip_group(&mut s, position);
        let ps = pieces(&mut s, &format!("object.pathfinder.{op}"));
        let total: f64 = ps.iter().map(area).sum();
        assert!((total - want).abs() < 1.0, "{op}: {total}, want {want}");
        assert!(ps.iter().all(|p| p.0 != Paint::solid(green())), "{op}: the hidden group's style: {ps:?}");
    }
}

/// A clip group whose clipping path clips by nothing (here an empty group) hides its members, as on
/// the canvas: they are a hidden operand too, not left out of the selection.
#[test]
fn a_clipping_path_that_clips_by_nothing_hides_the_members() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0, red());
    let b = rect(&mut s, 50.0, 50.0, 100.0, 100.0, blue());
    let m1 = rect(&mut s, 300.0, 300.0, 100.0, 100.0, green());
    let m2 = rect(&mut s, 450.0, 300.0, 100.0, 100.0, green());
    let mask = rect(&mut s, 300.0, 300.0, 300.0, 100.0, green());
    let g = clip(&mut s, &[m1, m2, mask]);
    s.edit("empty clipping path", |d, _| {
        let empty = vectorcraft_doc::Node::group(d.alloc_id(), vec![]);
        let NodeKind::Group { children, .. } = &mut d.node_mut(g).unwrap().kind else { panic!("not a group") };
        children[0] = std::sync::Arc::new(empty);
        Ok(())
    })
    .unwrap();
    select(&mut s, &[a, b, g]);
    let e = s.execute("object.pathfinder.intersect", &json!({})).unwrap_err().to_string();
    assert!(e.contains("empty result"), "not the squares' overlap: {e}");
    select(&mut s, &[g]);
    let e = s.execute("object.pathfinder.divide", &json!({})).unwrap_err().to_string();
    assert!(e.contains("clipping masks hide"), "{e}");
}
