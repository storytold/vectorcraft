//! Tests for Fill Area (`shapeBuilder.copyRegion` and the `fillArea` tool).

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::Node;
use vectorcraft_geom::{FillRule, PathData, Point};
use vectorcraft_tools::{Cursor, Mods, Overlay, PointerEvent, PointerKind};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

fn outline(s: &Session, id: NodeId) -> (PathData, FillRule) {
    vectorcraft_tools::builder::node_outline(&node(s, id)).expect("path")
}

fn area(s: &Session, id: NodeId) -> f64 {
    let (p, r) = outline(s, id);
    vectorcraft_pathops::area(&p, r)
}

fn line(s: &mut Session, a: (f64, f64), b: (f64, f64)) -> NodeId {
    id_of(&s.execute("shape.line", &json!({"x1": a.0, "y1": a.1, "x2": b.0, "y2": b.1})).unwrap())
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

/// Two diagonals and a crossbar: they enclose a triangle of 900 pt² above the crossbar.
fn triangle(s: &mut Session) -> [NodeId; 3] {
    [line(s, (0.0, 0.0), (100.0, 100.0)), line(s, (100.0, 0.0), (0.0, 100.0)), line(s, (-10.0, 80.0), (110.0, 80.0))]
}

/// The ids of the default layer's children, back to front.
fn stack(s: &Session) -> Vec<NodeId> {
    let d = &s.doc().unwrap().doc;
    d.children(d.default_layer()).map(|c| c.iter().map(|n| n.id).collect()).unwrap_or_default()
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn deselect(s: &mut Session) {
    s.execute("select.set", &json!({"ids": []})).unwrap();
}

/// Four circles in a 2×2 grid, each touching its neighbors, enclose a diamond nothing fills. With
/// nothing selected, a click in it adds the diamond and leaves the circles as they were.
#[test]
fn fill_area_adds_the_diamond_four_circles_enclose_and_keeps_them() {
    let mut s = session();
    let mut circles = vec![];
    for (x, y) in [(0.0, 0.0), (100.0, 0.0), (0.0, 100.0), (100.0, 100.0)] {
        circles.push(id_of(&s.execute("shape.ellipse", &json!({"x": x, "y": y, "width": 100, "height": 100})).unwrap()));
    }
    deselect(&mut s);
    let before: Vec<_> = circles.iter().map(|c| outline(&s, *c)).collect();
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [100, 100]})).unwrap();
    let new = id_of(&r);
    assert!((area(&s, new) - 2146.0).abs() < 10.0, "{}", area(&s, new));
    assert_eq!(stack(&s).len(), 5, "the four circles and the diamond");
    let after: Vec<_> = circles.iter().map(|c| outline(&s, *c)).collect();
    assert_eq!(after, before, "the circles keep their geometry");
    assert_eq!(s.doc().unwrap().selection.objects, vec![new], "the new path is selected");
}

/// The overlap of two filled squares is an area too: the new path covers it, and both squares stay
/// whole (a Shape Builder merge there cuts it out of them).
#[test]
fn fill_area_copies_the_overlap_of_two_filled_squares() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 50.0, 0.0, 100.0, 100.0);
    deselect(&mut s);
    let (oa, ob) = (outline(&s, a), outline(&s, b));
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [75, 50]})).unwrap();
    let new = id_of(&r);
    assert!((area(&s, new) - 5000.0).abs() < 1.0, "{}", area(&s, new));
    assert_eq!((outline(&s, a), outline(&s, b)), (oa, ob));
    assert_eq!(stack(&s), vec![a, b, new], "above the front-most square");
    assert_eq!(r["above"], json!(b.0));
}

/// Three lines bound a triangle; the new path has the current fill and stroke.
#[test]
fn fill_area_fills_the_triangle_three_lines_bound_with_the_current_paint() {
    let mut s = session();
    let lines = triangle(&mut s);
    deselect(&mut s);
    s.execute("paint.setFill", &json!({"color": "#336699"})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#993300"})).unwrap();
    let (fill, stroke) = (s.paint.fill.clone(), s.paint.stroke.clone());
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70]})).unwrap();
    let new = id_of(&r);
    assert!((area(&s, new) - 900.0).abs() < 1e-6, "{}", area(&s, new));
    let n = node(&s, new);
    assert_eq!(n.appearance.fill_paint(), fill);
    assert_eq!(n.appearance.stroke_paint(), stroke);
    assert_eq!(stack(&s), vec![lines[0], lines[1], lines[2], new]);
    // `fill` and `stroke` override the current paint.
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70], "fill": "#00ff00", "stroke": {"none": true}})).unwrap();
    let n = node(&s, id_of(&r));
    assert_eq!(n.appearance.fill_paint(), Paint::solid(Color::rgb(0.0, 1.0, 0.0)));
    assert_eq!(n.appearance.stroke_paint(), Paint::None);
}

/// The new path takes the current stroke weight.
#[test]
fn fill_area_takes_the_current_stroke_weight() {
    let mut s = session();
    triangle(&mut s);
    deselect(&mut s);
    s.execute("paint.setStroke", &json!({"color": "#993300"})).unwrap();
    s.paint.stroke_width = 4.0;
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70]})).unwrap();
    assert_eq!(node(&s, id_of(&r)).appearance.stroke_width(), 4.0);
}

/// `ids` names the objects or layers whose paths bound areas: without the crossbar the triangle is
/// open, and naming the layer takes every path in it.
#[test]
fn fill_area_ids_limit_the_paths_that_bound_areas() {
    let mut s = session();
    let lines = triangle(&mut s);
    deselect(&mut s);
    let two = json!({"point": [50, 70], "ids": [lines[0].0, lines[1].0]});
    assert!(s.execute("shapeBuilder.copyRegion", &two).is_err());
    let all = json!({"point": [50, 70], "ids": [lines[0].0, lines[1].0, lines[2].0]});
    let r = s.execute("shapeBuilder.copyRegion", &all).unwrap();
    assert!((area(&s, id_of(&r)) - 900.0).abs() < 1e-6);
    let layer = s.doc().unwrap().doc.default_layer().unwrap();
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70], "ids": [layer.0]})).unwrap();
    assert!((area(&s, id_of(&r)) - 900.0).abs() < 1e-6);
}

/// The new path goes above the group, Live Paint group or clipping group that holds the paths
/// along the area, never inside it, and the group keeps its members.
#[test]
fn fill_area_stacks_above_the_group_that_holds_the_paths() {
    let mut s = session();
    let lines = triangle(&mut s);
    let ids: Vec<u64> = lines.iter().map(|l| l.0).collect();
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    deselect(&mut s);
    let members = |s: &Session, g: NodeId| -> Vec<NodeId> { node(s, g).children().unwrap().iter().map(|n| n.id).collect() };
    let before = members(&s, g);
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70]})).unwrap();
    assert_eq!(stack(&s), vec![g, id_of(&r)]);
    assert_eq!(r["above"], json!(g.0));
    assert_eq!(members(&s, g), before);

    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 50.0, 0.0, 100.0, 100.0);
    let lp = id_of(&s.execute("livePaint.make", &json!({"ids": [a.0, b.0]})).unwrap());
    deselect(&mut s);
    let before = members(&s, lp);
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [75, 50]})).unwrap();
    assert!((area(&s, id_of(&r)) - 5000.0).abs() < 1.0, "{}", area(&s, id_of(&r)));
    assert_eq!(stack(&s), vec![lp, id_of(&r)]);
    assert_eq!(members(&s, lp), before);

    let mut s = session();
    let lines = triangle(&mut s);
    let mask = rect(&mut s, -20.0, -20.0, 140.0, 140.0);
    let ids: Vec<u64> = lines.iter().map(|l| l.0).chain([mask.0]).collect();
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    let clip = s.doc().unwrap().selection.objects[0];
    deselect(&mut s);
    let before = members(&s, clip);
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70]})).unwrap();
    assert_eq!(stack(&s), vec![clip, id_of(&r)]);
    assert_eq!(members(&s, clip), before);
}

/// `ids` clipped by a new rectangle mask in front of them, as the clipping group's id.
fn clip_by(s: &mut Session, ids: &[NodeId], x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let mask = rect(s, x, y, w, h);
    let ids: Vec<u64> = ids.iter().map(|i| i.0).chain([mask.0]).collect();
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap())
}

/// Clipped art bounds areas only where its clipping mask shows it: a click in the part the mask
/// hides finds no area and changes nothing, and a click in the part it shows fills only that part.
#[test]
fn fill_area_takes_clipped_art_only_where_its_mask_shows_it() {
    let mut s = session();
    let square = rect(&mut s, 50.0, 0.0, 100.0, 100.0);
    let clip = clip_by(&mut s, &[square], 0.0, -10.0, 100.0, 120.0);
    deselect(&mut s);
    let (undo, kids) = (undo_len(&s), stack(&s));
    assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [125, 50]})).is_err());
    assert_eq!((undo_len(&s), stack(&s)), (undo, kids));
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [75, 50]})).unwrap();
    assert!((area(&s, id_of(&r)) - 5000.0).abs() < 1.0, "{}", area(&s, id_of(&r)));
    assert_eq!(stack(&s), vec![clip, id_of(&r)], "the new path goes above the clipping group");

    // Without a fill only the outline shows, and the mask's edge does not close the part it shows.
    let mut s = session();
    let square = rect(&mut s, 50.0, 0.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    clip_by(&mut s, &[square], 0.0, -10.0, 100.0, 120.0);
    deselect(&mut s);
    let (undo, kids) = (undo_len(&s), stack(&s));
    assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [75, 50]})).is_err());
    assert_eq!((undo_len(&s), stack(&s)), (undo, kids));

    // Lines the mask cuts bound no area where the parts that close it are hidden.
    let mut s = session();
    let lines = triangle(&mut s);
    clip_by(&mut s, &lines, -20.0, -20.0, 60.0, 140.0);
    deselect(&mut s);
    let (undo, kids) = (undo_len(&s), stack(&s));
    assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [30, 75]})).is_err());
    assert_eq!((undo_len(&s), stack(&s)), (undo, kids));
}

/// In a clipping group inside another, clipped art shows only where both masks show it.
#[test]
fn fill_area_takes_art_in_nested_clipping_groups_only_where_every_mask_shows_it() {
    let mut s = session();
    let square = rect(&mut s, 50.0, 0.0, 100.0, 100.0);
    let inner = clip_by(&mut s, &[square], 0.0, -10.0, 100.0, 120.0);
    let outer = clip_by(&mut s, &[inner], 0.0, -20.0, 80.0, 140.0);
    deselect(&mut s);
    let (undo, kids) = (undo_len(&s), stack(&s));
    for p in [[125, 50], [90, 50]] {
        assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": p})).is_err(), "{p:?}");
    }
    assert_eq!((undo_len(&s), stack(&s)), (undo, kids));
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [65, 50]})).unwrap();
    assert!((area(&s, id_of(&r)) - 3000.0).abs() < 1.0, "{}", area(&s, id_of(&r)));
    assert_eq!(stack(&s), vec![outer, id_of(&r)]);
}

/// In isolation mode only the isolated group's paths bound areas, for the command and the tool: a
/// click in an area that art outside the group encloses adds nothing, and the new path goes inside
/// the group, above the path along the area.
#[test]
fn fill_area_stays_inside_the_isolated_group() {
    let mut s = session();
    let outside =
        [line(&mut s, (200.0, 0.0), (300.0, 100.0)), line(&mut s, (300.0, 0.0), (200.0, 100.0)), line(&mut s, (190.0, 80.0), (310.0, 80.0))];
    let lines = triangle(&mut s);
    let ids: Vec<u64> = lines.iter().map(|l| l.0).collect();
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    deselect(&mut s);
    let members = |s: &Session| -> Vec<NodeId> { node(s, g).children().unwrap().iter().map(|n| n.id).collect() };
    let (undo, kids) = (undo_len(&s), stack(&s));
    assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [250, 70]})).is_err(), "art outside the isolated group");
    let named: Vec<u64> = outside.iter().map(|l| l.0).collect();
    assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [250, 70], "ids": named})).is_err(), "named art outside it");
    assert_eq!((undo_len(&s), stack(&s), members(&s)), (undo, kids.clone(), lines.to_vec()));

    let v = ViewInfo::default();
    s.select_tool("fillArea", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Move, 250.0, 70.0), v).unwrap();
    assert!(
        !s.overlays(v).iter().any(|o| matches!(o, Overlay::Path { color: [0xff, 0x2a, 0x2a], .. })),
        "no area outside the isolated group is outlined"
    );
    s.pointer(&PointerEvent::new(PointerKind::Down, 250.0, 70.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 250.0, 70.0), v).unwrap();
    assert_eq!((undo_len(&s), stack(&s), members(&s)), (undo, kids.clone(), lines.to_vec()));

    s.pointer(&PointerEvent::new(PointerKind::Move, 50.0, 70.0), v).unwrap();
    let red = s.overlays(v).into_iter().filter(|o| matches!(o, Overlay::Path { color: [0xff, 0x2a, 0x2a], .. })).count();
    assert_eq!(red, 1, "the isolated group's area is outlined");
    s.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 70.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 50.0, 70.0), v).unwrap();
    let new = s.doc().unwrap().selection.objects[0];
    assert_eq!(s.doc().unwrap().doc.parent_of(new), Some(g));
    assert_eq!(members(&s), [lines.as_slice(), &[new]].concat());
    assert_eq!(stack(&s), kids);
    assert!((area(&s, new) - 900.0).abs() < 1.0, "{}", area(&s, new));

    // An isolated object that no longer exists counts as no isolation, for the tool as for the
    // command: the area outside the group fills.
    s.doc_mut().unwrap().isolation = Some(NodeId(999_999));
    s.pointer(&PointerEvent::new(PointerKind::Move, 250.0, 70.0), v).unwrap();
    let red = s.overlays(v).into_iter().filter(|o| matches!(o, Overlay::Path { color: [0xff, 0x2a, 0x2a], .. })).count();
    assert_eq!(red, 1, "the area outside the group is outlined");
    s.pointer(&PointerEvent::new(PointerKind::Down, 250.0, 70.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 250.0, 70.0), v).unwrap();
    let new = s.doc().unwrap().selection.objects[0];
    assert_eq!(s.doc().unwrap().doc.parent_of(new), s.doc().unwrap().doc.default_layer());
}

/// In isolation mode, `ids` naming the isolated group, or the layer or a group that holds it, takes
/// every path of the isolated group. The other paths of that layer or group, and other listed
/// objects outside the isolated group, do not bound areas.
#[test]
fn fill_area_ids_naming_what_holds_the_isolated_group_take_its_paths() {
    let mut s = session();
    let outside =
        [line(&mut s, (200.0, 0.0), (300.0, 100.0)), line(&mut s, (300.0, 0.0), (200.0, 100.0)), line(&mut s, (190.0, 80.0), (310.0, 80.0))];
    let lines = triangle(&mut s);
    let ids: Vec<u64> = lines.iter().map(|l| l.0).collect();
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    let outer = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    deselect(&mut s);
    let layer = s.doc().unwrap().doc.default_layer().unwrap();
    let (undo, kids) = (undo_len(&s), stack(&s));
    for named in [vec![layer.0], vec![outer.0], vec![g.0], vec![layer.0, outside[0].0]] {
        let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [250, 70], "ids": named}));
        assert!(r.is_err(), "the area outside the isolated group, ids {named:?}");
        assert_eq!((undo_len(&s), stack(&s)), (undo, kids.clone()));
        let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70], "ids": named})).unwrap();
        let new = id_of(&r);
        assert_eq!(s.doc().unwrap().doc.parent_of(new), Some(g), "ids {named:?}");
        assert!((area(&s, new) - 900.0).abs() < 1.0, "{}", area(&s, new));
        s.execute("edit.undo", &json!({})).unwrap();
    }
    let named: Vec<u64> = outside.iter().map(|l| l.0).collect();
    assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70], "ids": named})).is_err(), "only art outside it listed");
    assert_eq!((undo_len(&s), stack(&s)), (undo, kids));
}

/// While a compound path is isolated, Fill Area finds no area, for the command and the tool, and
/// changes nothing: the compound path cannot hold the new path. The same goes for an isolated
/// Objects on Path group, a group with a Transform effect and a group that is a clipping mask.
#[test]
fn fill_area_finds_nothing_while_a_compound_path_is_isolated() {
    let mut s = session();
    let outer = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let inner = rect(&mut s, 25.0, 25.0, 50.0, 50.0);
    s.execute("select.set", &json!({"ids": [outer.0, inner.0]})).unwrap();
    let c = id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap());
    deselect(&mut s);
    assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [10, 10]})).is_ok(), "the ring fills outside isolation");
    s.execute("edit.undo", &json!({})).unwrap();
    s.doc_mut().unwrap().isolation = Some(c);
    let (undo, kids) = (undo_len(&s), stack(&s));
    assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [10, 10]})).is_err());
    let v = ViewInfo::default();
    s.select_tool("fillArea", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Move, 10.0, 10.0), v).unwrap();
    assert!(!s.overlays(v).iter().any(|o| matches!(o, Overlay::Path { color: [0xff, 0x2a, 0x2a], .. })));
    s.pointer(&PointerEvent::new(PointerKind::Down, 10.0, 10.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 10.0, 10.0), v).unwrap();
    assert_eq!((undo_len(&s), stack(&s)), (undo, kids));

    // An Objects on Path group moves its members along the path, so a new path in it would not
    // stay where the area is.
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 40.0, 40.0);
    let b = rect(&mut s, 100.0, 0.0, 40.0, 40.0);
    let path = line(&mut s, (0.0, 200.0), (300.0, 200.0));
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let g = id_of(&s.execute("object.objectsOnPath.attach", &json!({"path": path.0})).unwrap());
    deselect(&mut s);
    let c = s.doc().unwrap().doc.node(a).unwrap().geometric_bounds().unwrap().center();
    assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [c.x, c.y]})).is_ok(), "the square fills outside isolation");
    s.execute("edit.undo", &json!({})).unwrap();
    s.doc_mut().unwrap().isolation = Some(g);
    let (undo, kids) = (undo_len(&s), stack(&s));
    assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [c.x, c.y]})).is_err());
    assert_eq!((undo_len(&s), stack(&s)), (undo, kids));

    // A group with a Transform effect moves what is in it, so a new path in it would not be drawn
    // where the area is.
    let mut s = session();
    let lines = triangle(&mut s);
    let ids: Vec<u64> = lines.iter().map(|l| l.0).collect();
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.execute("effect.apply", &json!({"effect": "distort.transform", "params": {"moveH": 100}, "ids": [g.0], "item": null})).unwrap();
    deselect(&mut s);
    s.doc_mut().unwrap().isolation = Some(g);
    let (undo, kids) = (undo_len(&s), stack(&s));
    assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70]})).is_err());
    assert_eq!((undo_len(&s), stack(&s), node(&s, g).children().unwrap().len()), (undo, kids, 3));

    // A group that is a clipping mask clips and is not painted, so it takes no new path.
    let mut s = session();
    let back = rect(&mut s, -20.0, -20.0, 140.0, 140.0);
    let edge = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [back.0, edge.0]})).unwrap();
    let clip = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    s.execute("select.set", &json!({"ids": [edge.0]})).unwrap();
    let mask = id_of(&s.execute("object.group", &json!({})).unwrap());
    // A filled square drawn in the mask group bounds an area of its own.
    s.doc_mut().unwrap().isolation = Some(mask);
    let square = rect(&mut s, 10.0, 10.0, 80.0, 80.0);
    assert_eq!(s.doc().unwrap().doc.parent_of(square), Some(mask));
    deselect(&mut s);
    let first = node(&s, clip).children().unwrap()[0].id;
    assert_eq!(first, mask, "the group is the clipping mask");
    let (undo, kids) = (undo_len(&s), stack(&s));
    let members = node(&s, mask).children().unwrap().len();
    assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70]})).is_err());
    assert_eq!((undo_len(&s), stack(&s), node(&s, mask).children().unwrap().len()), (undo, kids, members));
    // The clipping group itself, and a group of the art it clips, take no new path either: the
    // mask would hide or cut it.
    s.doc_mut().unwrap().isolation = None;
    s.execute("select.set", &json!({"ids": [back.0]})).unwrap();
    let clipped = id_of(&s.execute("object.group", &json!({})).unwrap());
    assert_eq!(s.doc().unwrap().doc.parent_of(clipped), Some(clip));
    deselect(&mut s);
    for iso in [clip, clipped] {
        s.doc_mut().unwrap().isolation = Some(iso);
        let (undo, kids) = (undo_len(&s), stack(&s));
        assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [5, 5]})).is_err(), "{iso:?}");
        assert_eq!((undo_len(&s), stack(&s)), (undo, kids), "{iso:?}");
    }
}

/// The new path goes directly above the front-most object along the area, not above art that
/// lies elsewhere, and paths on other layers bound areas too.
#[test]
fn fill_area_takes_paths_on_every_layer_and_stacks_above_the_area() {
    let mut s = session();
    let first = s.doc().unwrap().doc.default_layer();
    let l1 = line(&mut s, (0.0, 0.0), (100.0, 100.0));
    let l2 = line(&mut s, (100.0, 0.0), (0.0, 100.0));
    let far = rect(&mut s, 400.0, 400.0, 50.0, 50.0);
    let other = id_of(&s.execute("layer.new", &json!({"name": "Other"})).unwrap());
    let l3 = line(&mut s, (-10.0, 80.0), (110.0, 80.0));
    let top = rect(&mut s, 500.0, 400.0, 50.0, 50.0);
    deselect(&mut s);
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70]})).unwrap();
    let new = id_of(&r);
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.parent_of(new), Some(other));
    let kids: Vec<NodeId> = d.children(Some(other)).unwrap().iter().map(|n| n.id).collect();
    assert_eq!(kids, vec![l3, new, top]);
    let kids: Vec<NodeId> = d.children(first).unwrap().iter().map(|n| n.id).collect();
    assert_eq!(kids, vec![l1, l2, far]);
}

/// An area far larger than the paths near the point: four long lines bound a 400 pt square whose
/// middle no path's bounds hold, and a small triangle inside it is a hole in the area.
#[test]
fn fill_area_finds_an_area_that_reaches_far_from_the_point() {
    let mut s = session();
    line(&mut s, (-20.0, 0.0), (420.0, 0.0));
    line(&mut s, (-20.0, 400.0), (420.0, 400.0));
    line(&mut s, (0.0, -20.0), (0.0, 420.0));
    line(&mut s, (400.0, -20.0), (400.0, 420.0));
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [200, 200]})).unwrap();
    assert!((area(&s, id_of(&r)) - 160_000.0).abs() < 1.0, "{}", area(&s, id_of(&r)));
    for (a, b) in [((150.0, 250.0), (250.0, 350.0)), ((250.0, 250.0), (150.0, 350.0)), ((140.0, 330.0), (260.0, 330.0))] {
        line(&mut s, a, b);
    }
    deselect(&mut s);
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 50]})).unwrap();
    assert!((area(&s, id_of(&r)) - (160_000.0 - 900.0)).abs() < 1.0, "the square less the triangle: {}", area(&s, id_of(&r)));
}

/// Distant art with more segments than one Shape Builder map takes does not stop four lines from
/// bounding the square between them: the search maps the lines near the point first, and a window
/// that reaches the distant art leaves it out.
#[test]
fn fill_area_finds_a_nearby_area_beside_a_huge_distant_path() {
    for (at, huge_x) in [(0.0, 20_000.0), (100.0, 1_300.0)] {
        let mut s = session();
        let (lo, hi) = (at, at + 800.0);
        line(&mut s, (lo - 20.0, lo), (hi + 20.0, lo));
        line(&mut s, (lo - 20.0, hi), (hi + 20.0, hi));
        line(&mut s, (lo, lo - 20.0), (lo, hi + 20.0));
        line(&mut s, (hi, lo - 20.0), (hi, hi + 20.0));
        let n = vectorcraft_pathops::SHAPE_BUILDER_MAX_SEGMENTS;
        let pts: Vec<Point> = (0..=n).map(|k| Point::new(huge_x + 400.0 * k as f64 / n as f64, 500.0 + (k % 2) as f64)).collect();
        let look = vectorcraft_doc::Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
        s.edit("t", |d, _| {
            let id = d.alloc_id();
            let top = d.children(d.default_layer()).map_or(0, |c| c.len());
            d.insert(d.default_layer(), top, Node::path(id, PathData::single(vectorcraft_geom::SubPath::polyline(&pts, false)), look))?;
            Ok(id)
        })
        .unwrap();
        deselect(&mut s);
        let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [at + 400.0, at + 400.0]})).unwrap();
        assert!((area(&s, id_of(&r)) - 640_000.0).abs() < 1.0, "{huge_x}: {}", area(&s, id_of(&r)));
    }
}

/// A path with too many segments near a long, flat area stays out of the map, and the area's far
/// ends still bound it.
#[test]
fn fill_area_finds_a_long_area_beside_a_huge_path() {
    let mut s = session();
    line(&mut s, (-10.0, -20.0), (810.0, -20.0));
    line(&mut s, (-10.0, 20.0), (810.0, 20.0));
    line(&mut s, (0.0, -30.0), (0.0, 30.0));
    line(&mut s, (800.0, -30.0), (800.0, 30.0));
    line(&mut s, (-10.0, 0.0), (810.0, 0.0));
    let n = vectorcraft_pathops::SHAPE_BUILDER_MAX_SEGMENTS;
    let pts: Vec<Point> = (0..=n).map(|k| Point::new(300.0 + 200.0 * k as f64 / n as f64, 60.0 + (k % 2) as f64 * 0.5)).collect();
    let look = vectorcraft_doc::Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
    s.edit("t", |d, _| {
        let id = d.alloc_id();
        let top = d.children(d.default_layer()).map_or(0, |c| c.len());
        d.insert(d.default_layer(), top, Node::path(id, PathData::single(vectorcraft_geom::SubPath::polyline(&pts, false)), look))?;
        Ok(id)
    })
    .unwrap();
    deselect(&mut s);
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [400, 0.5]})).unwrap();
    assert!((area(&s, id_of(&r)) - 16_000.0).abs() < 1.0, "{}", area(&s, id_of(&r)));
}

/// A path with too many segments that crosses the area cannot be left out, and the paths together
/// are past the segment limit: the square it cuts is not filled across it, whether four lines or one
/// filled rectangle bound the square.
#[test]
fn fill_area_does_not_fill_across_a_huge_path_it_left_out() {
    for closed in [false, true] {
        let mut s = session();
        if closed {
            rect(&mut s, 0.0, 0.0, 400.0, 400.0);
        } else {
            line(&mut s, (-20.0, 0.0), (420.0, 0.0));
            line(&mut s, (-20.0, 400.0), (420.0, 400.0));
            line(&mut s, (0.0, -20.0), (0.0, 420.0));
            line(&mut s, (400.0, -20.0), (400.0, 420.0));
        }
        let n = vectorcraft_pathops::SHAPE_BUILDER_MAX_SEGMENTS;
        let pts: Vec<Point> = (0..=n).map(|k| Point::new(-20.0 + 440.0 * k as f64 / n as f64, 300.0)).collect();
        let look = vectorcraft_doc::Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
        s.edit("t", |d, _| {
            let id = d.alloc_id();
            let top = d.children(d.default_layer()).map_or(0, |c| c.len());
            d.insert(d.default_layer(), top, Node::path(id, PathData::single(vectorcraft_geom::SubPath::polyline(&pts, false)), look))?;
            Ok(id)
        })
        .unwrap();
        deselect(&mut s);
        let (undo, kids) = (undo_len(&s), stack(&s));
        assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [200, 100]})).is_err(), "closed: {closed}");
        assert_eq!((undo_len(&s), stack(&s)), (undo, kids));
    }
}

/// A path with too many segments whose box holds the square, but no segment of it comes near: its
/// subpaths lie far apart on either side of the square, or one curve arcs around the square and its
/// box holds the square. It is left out and the square is filled.
#[test]
fn fill_area_fills_an_area_that_a_left_out_paths_box_holds_but_its_segments_miss() {
    // A cubic from (-1000, 1500) up over the square to (1400, 1500); at its top it is at y -750.
    let mut arc = vectorcraft_geom::SubPath::polyline(&[Point::new(-1000.0, 1500.0), Point::new(1400.0, 1500.0)], false);
    arc.anchors[0].h_out = Point::new(-1000.0, -1500.0);
    arc.anchors[1].h_in = Point::new(1400.0, -1500.0);
    let far = vectorcraft_geom::SubPath::polyline(&[Point::new(2000.0, 1500.0), Point::new(2100.0, 1500.0)], false);
    for second in [far, arc] {
        let mut s = session();
        line(&mut s, (-20.0, 0.0), (420.0, 0.0));
        line(&mut s, (-20.0, 400.0), (420.0, 400.0));
        line(&mut s, (0.0, -20.0), (0.0, 420.0));
        line(&mut s, (400.0, -20.0), (400.0, 420.0));
        let n = vectorcraft_pathops::SHAPE_BUILDER_MAX_SEGMENTS;
        let pts: Vec<Point> = (0..=n).map(|k| Point::new(-2000.0 + 400.0 * k as f64 / n as f64, -1000.0 + (k % 2) as f64)).collect();
        let path = PathData::new(vec![vectorcraft_geom::SubPath::polyline(&pts, false), second.clone()]);
        let look = vectorcraft_doc::Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
        s.edit("t", |d, _| {
            let id = d.alloc_id();
            let top = d.children(d.default_layer()).map_or(0, |c| c.len());
            d.insert(d.default_layer(), top, Node::path(id, path, look))?;
            Ok(id)
        })
        .unwrap();
        deselect(&mut s);
        let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [200, 200]})).unwrap();
        assert!((area(&s, id_of(&r)) - 160_000.0).abs() < 1.0, "{second:?}: {}", area(&s, id_of(&r)));
    }
}

/// Puts `path` with `look` in front on the current layer.
fn insert_path(s: &mut Session, path: PathData, look: vectorcraft_doc::Appearance) -> NodeId {
    s.edit("t", |d, _| {
        let id = d.alloc_id();
        let top = d.children(d.default_layer()).map_or(0, |c| c.len());
        d.insert(d.default_layer(), top, Node::path(id, path, look))?;
        Ok(id)
    })
    .unwrap()
}

/// A path with too many segments in the empty corner of a filled L, well clear of both arms, is
/// left out and does not stop the L from being filled, though the L's box holds it.
#[test]
fn fill_area_fills_an_l_whose_empty_corner_holds_a_left_out_path() {
    let mut s = session();
    let l = [(0.0, 0.0), (400.0, 0.0), (400.0, 100.0), (100.0, 100.0), (100.0, 400.0), (0.0, 400.0)].map(|(x, y)| Point::new(x, y));
    let solid = vectorcraft_doc::Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0);
    insert_path(&mut s, PathData::single(vectorcraft_geom::SubPath::polyline(&l, true)), solid);
    let n = vectorcraft_pathops::SHAPE_BUILDER_MAX_SEGMENTS;
    let pts: Vec<Point> = (0..=n).map(|k| Point::new(250.0 + 100.0 * k as f64 / n as f64, 250.0 + (k % 2) as f64)).collect();
    let look = vectorcraft_doc::Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
    insert_path(&mut s, PathData::single(vectorcraft_geom::SubPath::polyline(&pts, false)), look);
    deselect(&mut s);
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 50]})).unwrap();
    assert!((area(&s, id_of(&r)) - 70_000.0).abs() < 1.0, "{}", area(&s, id_of(&r)));
}

/// A filled contour with too many segments on a higher layer surrounds a small square, its outline
/// far from it: it is left out of the map, and the new path still goes above it, since it covers
/// the area.
#[test]
fn fill_area_stacks_above_a_left_out_path_that_covers_the_area() {
    let mut s = session();
    let layer = s.doc().unwrap().doc.default_layer();
    rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let over = id_of(&s.execute("layer.new", &json!({"name": "Over"})).unwrap());
    let n = vectorcraft_pathops::SHAPE_BUILDER_MAX_SEGMENTS + 1;
    let pts: Vec<Point> = (0..n)
        .map(|k| {
            let a = k as f64 * std::f64::consts::TAU / n as f64;
            Point::new(50.0 + 3000.0 * a.cos(), 50.0 + 3000.0 * a.sin())
        })
        .collect();
    let solid = vectorcraft_doc::Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0);
    let contour = insert_path(&mut s, PathData::single(vectorcraft_geom::SubPath::polyline(&pts, true)), solid);
    deselect(&mut s);
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 50]})).unwrap();
    assert!((area(&s, id_of(&r)) - 10_000.0).abs() < 1.0, "{}", area(&s, id_of(&r)));
    assert_eq!(r["above"], json!(contour.0));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.parent_of(id_of(&r)), Some(over));
    assert_ne!(d.parent_of(id_of(&r)), layer);
}

/// With Gap Detection off, a long line on a higher layer 0.75 pt from a square's edge neither
/// touches nor covers the square, so the new path stays above the square.
#[test]
fn fill_area_does_not_stack_above_a_line_that_does_not_touch_the_area() {
    let mut s = session();
    let layer = s.doc().unwrap().doc.default_layer();
    let square = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    id_of(&s.execute("layer.new", &json!({"name": "Other"})).unwrap());
    line(&mut s, (-5000.0, -0.75), (5000.0, -0.75));
    deselect(&mut s);
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 50]})).unwrap();
    assert_eq!(r["above"], json!(square.0));
    assert_eq!(s.doc().unwrap().doc.parent_of(id_of(&r)), layer);
}

/// With Gap Detection on, a square on a higher layer within the gap of the area's corners does not
/// bound the area, so the new path stays above the square that does.
#[test]
fn fill_area_gap_does_not_stack_above_a_nearby_path() {
    let mut s = session();
    let first = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let layer = s.doc().unwrap().doc.default_layer();
    let other = id_of(&s.execute("layer.new", &json!({"name": "Other"})).unwrap());
    rect(&mut s, 102.0, 0.0, 100.0, 100.0);
    deselect(&mut s);
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 50], "gap": 3})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.parent_of(id_of(&r)), layer);
    assert_ne!(d.parent_of(id_of(&r)), Some(other));
    assert_eq!(r["above"], json!(first.0));
}

/// An object listed in `ids` with the layer that holds it counts once, so the new path still goes
/// above the front-most of two overlapping squares.
#[test]
fn fill_area_ids_count_an_object_and_its_layer_once() {
    let mut s = session();
    let back = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let front = rect(&mut s, 50.0, 0.0, 100.0, 100.0);
    deselect(&mut s);
    let layer = s.doc().unwrap().doc.default_layer().unwrap();
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [75, 50], "ids": [layer.0, back.0]})).unwrap();
    assert_eq!(stack(&s), vec![back, front, id_of(&r)]);
}

/// A flat window that reaches a path with too many segments leaves that path out and grows to the
/// paths it had not picked: a crossbar that no path near the point leads to still splits the area.
#[test]
fn fill_area_keeps_paths_a_capped_window_had_not_picked() {
    let mut s = session();
    // Four lines bound a box; a line through the point and a crossbar higher up cut it.
    line(&mut s, (50.0, -60.0), (50.0, 410.0));
    line(&mut s, (350.0, -60.0), (350.0, 410.0));
    line(&mut s, (40.0, -50.0), (360.0, -50.0));
    line(&mut s, (40.0, 400.0), (360.0, 400.0));
    line(&mut s, (0.0, 100.0), (400.0, 100.0));
    line(&mut s, (40.0, 30.0), (360.0, 30.0));
    let n = vectorcraft_pathops::SHAPE_BUILDER_MAX_SEGMENTS;
    let pts: Vec<Point> = (0..=n).map(|k| Point::new(380.0 + 20.0 * k as f64 / n as f64, 100.0 + (k % 2) as f64 * 0.5)).collect();
    let look = vectorcraft_doc::Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
    s.edit("t", |d, _| {
        let id = d.alloc_id();
        let top = d.children(d.default_layer()).map_or(0, |c| c.len());
        d.insert(d.default_layer(), top, Node::path(id, PathData::single(vectorcraft_geom::SubPath::polyline(&pts, false)), look))?;
        Ok(id)
    })
    .unwrap();
    deselect(&mut s);
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [200, 99.5]})).unwrap();
    assert!((area(&s, id_of(&r)) - 21_000.0).abs() < 1.0, "between the crossbar and the line: {}", area(&s, id_of(&r)));
}

/// Two diagonals and a crossbar about 3 pt short of each: the area closes only with a gap of 3 pt
/// or more, and a gap out of 0 to 72 pt is an error.
#[test]
fn fill_area_gap_closes_small_gaps() {
    let mut s = session();
    line(&mut s, (0.0, 0.0), (100.0, 100.0));
    line(&mut s, (100.0, 0.0), (0.0, 100.0));
    line(&mut s, (24.0, 80.0), (76.0, 80.0));
    deselect(&mut s);
    assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70]})).is_err());
    assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70], "gap": 2})).is_err());
    let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70], "gap": 3})).unwrap();
    let a = area(&s, id_of(&r));
    assert!(a > 850.0 && a < 900.0, "the triangle and its two bridges: {a}");
    for gap in [json!(-1), json!(73), json!("3")] {
        assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70], "gap": gap})).is_err(), "{gap}");
    }
}

/// Locked and hidden paths do not bound an area: the triangle fills while its crossbar is unlocked
/// and visible, and not once the crossbar is locked or hidden.
#[test]
fn fill_area_skips_locked_and_hidden_paths() {
    for how in ["object.lock", "object.hide"] {
        let mut s = session();
        let lines = triangle(&mut s);
        deselect(&mut s);
        let r = s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70]})).unwrap();
        assert!((area(&s, id_of(&r)) - 900.0).abs() < 1e-6, "{how}");
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute(how, &json!({"ids": [lines[2].0]})).unwrap();
        deselect(&mut s);
        let (undo, kids) = (undo_len(&s), stack(&s));
        assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70]})).is_err(), "{how}");
        assert_eq!((undo_len(&s), stack(&s)), (undo, kids), "{how}");
    }
}

/// A point in no enclosed area changes nothing and records no undo step; bad params are errors. A
/// point inside the triangle fills it.
#[test]
fn fill_area_outside_every_area_changes_nothing() {
    let mut s = session();
    triangle(&mut s);
    deselect(&mut s);
    assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70]})).is_ok());
    s.execute("edit.undo", &json!({})).unwrap();
    let (undo, kids) = (undo_len(&s), stack(&s));
    for p in [json!({"point": [500, 500]}), json!({"point": [50, 90]}), json!({"point": [-500, -500]})] {
        assert!(s.execute("shapeBuilder.copyRegion", &p).is_err(), "{p}");
    }
    for p in [json!({}), json!({"point": [50]}), json!({"point": [50, 70], "fill": "nope"}), json!({"point": [50, 70], "stroke": 3})] {
        assert!(s.execute("shapeBuilder.copyRegion", &p).is_err(), "{p}");
    }
    for paint in [json!({"swatch": 123}), json!({"color": 5}), json!({"color": "nope"})] {
        for key in ["fill", "stroke"] {
            let p = json!({"point": [50, 70], key: paint});
            assert!(s.execute("shapeBuilder.copyRegion", &p).is_err(), "{p}");
        }
    }
    assert!(s.execute("shapeBuilder.copyRegion", &json!({"point": [50, 70], "ids": [999_999]})).is_err());
    assert_eq!((undo_len(&s), stack(&s)), (undo, kids));
}

/// The tool outlines the area under the pointer, shows the bucket over it, and a click adds the
/// path; a click outside every area records nothing. It uses the Shape Builder's Gap Detection.
#[test]
fn fill_area_tool_drives_the_command() {
    let mut s = session();
    line(&mut s, (0.0, 0.0), (100.0, 100.0));
    line(&mut s, (100.0, 0.0), (0.0, 100.0));
    line(&mut s, (24.0, 80.0), (76.0, 80.0));
    deselect(&mut s);
    let v = ViewInfo::default();
    s.select_tool("fillArea", v).unwrap();
    assert_eq!(s.tool_id(), "fillArea");
    s.pointer(&PointerEvent::new(PointerKind::Move, 50.0, 70.0), v).unwrap();
    assert!(s.overlays(v).is_empty(), "the gap leaves the triangle open");
    assert_eq!(s.cursor(Point::new(50.0, 70.0), Mods::default(), v), Cursor::LivePaintBucket);
    let (undo, kids) = (undo_len(&s), stack(&s));
    s.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 70.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 50.0, 70.0), v).unwrap();
    assert_eq!((undo_len(&s), stack(&s)), (undo, kids), "nothing to fill");
    // The Shape Builder's options reach the active Fill Area tool.
    s.set_tool_option_cmd(&json!({"tool": "shapeBuilder", "values": {"gapDetection": true, "gapLength": 3}})).unwrap();
    assert_eq!(s.tool_options_of("fillArea")["gapLength"], json!(3.0));
    s.pointer(&PointerEvent::new(PointerKind::Move, 50.0, 70.0), v).unwrap();
    let red: Vec<_> = s.overlays(v).into_iter().filter(|o| matches!(o, Overlay::Path { color: [0xff, 0x2a, 0x2a], .. })).collect();
    assert_eq!(red.len(), 1, "the area is outlined in red");
    assert_eq!(s.cursor(Point::new(50.0, 70.0), Mods::default(), v), Cursor::LivePaintBucketFace);
    s.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 70.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 50.0, 70.0), v).unwrap();
    assert_eq!(s.journal.last().unwrap().0, "shapeBuilder.copyRegion");
    assert_eq!(stack(&s).len(), 4);
    assert_eq!(undo_len(&s), undo + 1);
    assert_eq!(s.tool_id(), "fillArea");
    assert!(s.set_tool_option_cmd(&json!({"tool": "fillArea", "values": {"highlightColor": "#ff0000"}})).is_err());
    assert!(s.set_tool_option_cmd(&json!({"tool": "fillArea", "values": {"gapLength": "wide"}})).is_err());
    // Setting them on Fill Area sets them for the Shape Builder too.
    s.set_tool_option_cmd(&json!({"tool": "fillArea", "values": {"gapLength": 5, "openFilledAsClosed": false}})).unwrap();
    let opts = s.tool_options_of("shapeBuilder");
    assert_eq!((opts["gapLength"].clone(), opts["openFilledAsClosed"].clone()), (json!(5.0), json!(false)));
}

/// The hover search for a document of more than 128 segments runs on another thread: nothing is
/// outlined until the tool is polled, and a click finds the area at once.
#[test]
fn fill_area_tool_hovers_a_large_document_on_another_thread() {
    let mut s = session();
    s.execute("shape.polygon", &json!({"cx": 300, "cy": 300, "radius": 100, "sides": 200})).unwrap();
    deselect(&mut s);
    let v = ViewInfo::default();
    s.select_tool("fillArea", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Move, 300.0, 300.0), v).unwrap();
    assert!(s.tool_waiting());
    assert!(s.overlays(v).is_empty());
    // The pointer moves on inside the area before the search is done: its result still serves.
    s.pointer(&PointerEvent::new(PointerKind::Move, 305.0, 300.0), v).unwrap();
    s.tool_poll(v, false).unwrap();
    s.tool_poll(v, true).unwrap();
    assert!(!s.tool_waiting());
    assert_eq!(s.overlays(v).len(), 1, "the polygon is outlined");
    // Leaving the area takes the outline away at once, while the next search runs.
    s.pointer(&PointerEvent::new(PointerKind::Move, 500.0, 500.0), v).unwrap();
    assert!(s.overlays(v).is_empty());
    s.tool_poll(v, true).unwrap();
    assert!(s.overlays(v).is_empty());
    s.pointer(&PointerEvent::new(PointerKind::Move, 300.0, 300.0), v).unwrap();
    s.tool_poll(v, true).unwrap();
    assert_eq!(s.overlays(v).len(), 1);
    // A click while a search runs waits for it: the pointer leaves the area, which starts a
    // search, and the press inside the area comes before any poll.
    s.pointer(&PointerEvent::new(PointerKind::Move, 500.0, 500.0), v).unwrap();
    assert!(s.tool_waiting());
    let undo = undo_len(&s);
    s.pointer(&PointerEvent::new(PointerKind::Down, 310.0, 300.0), v).unwrap();
    assert!(!s.tool_waiting(), "the press waited for the search");
    s.pointer(&PointerEvent::new(PointerKind::Up, 310.0, 300.0), v).unwrap();
    assert_eq!(undo_len(&s), undo + 1);
}

/// A press that is dropped before the release (a long or canceled touch) takes the new path back
/// and records no undo step.
#[test]
fn fill_area_tool_dropped_press_adds_nothing() {
    let mut s = session();
    triangle(&mut s);
    deselect(&mut s);
    let v = ViewInfo::default();
    s.select_tool("fillArea", v).unwrap();
    let (undo, kids) = (undo_len(&s), stack(&s));
    s.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 70.0), v).unwrap();
    assert_eq!(stack(&s).len(), kids.len() + 1, "the path shows during the press");
    s.drop_tool_press(v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 50.0, 70.0), v).unwrap();
    assert_eq!((undo_len(&s), stack(&s)), (undo, kids));
}

/// The outline under a still pointer goes when the document changes, and the canvas hovers the
/// tool again then.
#[test]
fn fill_area_tool_drops_a_hover_the_document_no_longer_has() {
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let v = ViewInfo::default();
    s.select_tool("fillArea", v).unwrap();
    assert!(s.tool_wants_rehover());
    s.pointer(&PointerEvent::new(PointerKind::Move, 50.0, 50.0), v).unwrap();
    assert_eq!(s.overlays(v).len(), 1);
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(s.overlays(v).is_empty());
    assert_eq!(s.cursor(Point::new(50.0, 50.0), Mods::default(), v), Cursor::LivePaintBucket);
}

/// Moving the pointer from an area with a hole into the area that fills the hole outlines the
/// inner area instead.
#[test]
fn fill_area_tool_hover_moves_from_an_area_into_its_hole() {
    let mut s = session();
    line(&mut s, (-20.0, 0.0), (420.0, 0.0));
    line(&mut s, (-20.0, 400.0), (420.0, 400.0));
    line(&mut s, (0.0, -20.0), (0.0, 420.0));
    line(&mut s, (400.0, -20.0), (400.0, 420.0));
    for (a, b) in [((150.0, 250.0), (250.0, 350.0)), ((250.0, 250.0), (150.0, 350.0)), ((140.0, 330.0), (260.0, 330.0))] {
        line(&mut s, a, b);
    }
    deselect(&mut s);
    let v = ViewInfo::default();
    s.select_tool("fillArea", v).unwrap();
    let outlined = |s: &mut Session| -> Vec<f64> {
        s.overlays(v)
            .into_iter()
            .filter_map(|o| match o {
                Overlay::Path { path, .. } => Some(kurbo::Shape::bounding_box(&path).width()),
                _ => None,
            })
            .collect()
    };
    s.pointer(&PointerEvent::new(PointerKind::Move, 50.0, 50.0), v).unwrap();
    let w = outlined(&mut s);
    assert!(w.len() == 1 && (w[0] - 400.0).abs() < 1e-6, "the square: {w:?}");
    s.pointer(&PointerEvent::new(PointerKind::Move, 200.0, 320.0), v).unwrap();
    let w = outlined(&mut s);
    assert!(w.len() == 1 && (w[0] - 60.0).abs() < 1e-6, "the triangle in its hole: {w:?}");
}
