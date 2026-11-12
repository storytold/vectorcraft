//! Tests for `addon.cutting.bridges`: bridges cut out of the selected art keep a stencil's islands
//! attached.

use kurbo::Shape;
use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;

use super::super::super::super::pathops::node_path;
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

fn select(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

/// `outer` less each of `holes` (x, y, width, height), as one compound path, selected.
fn shape_with_holes(s: &mut Session, outer: (f64, f64, f64, f64), holes: &[(f64, f64, f64, f64)]) -> NodeId {
    let mut ids = vec![rect(s, outer.0, outer.1, outer.2, outer.3)];
    for h in holes {
        ids.push(rect(s, h.0, h.1, h.2, h.3));
    }
    select(s, &ids);
    let r = s.execute("object.pathfinder.minusFront", &json!({})).unwrap();
    let id = NodeId(r["ids"][0].as_u64().unwrap());
    select(s, &[id]);
    id
}

/// A square O: 100 pt across at (100, 100), with bars 20 pt thick.
fn letter_o(s: &mut Session) -> NodeId {
    shape_with_holes(s, (100.0, 100.0, 100.0, 100.0), &[(120.0, 120.0, 60.0, 60.0)])
}

fn id_of(r: &Value) -> NodeId {
    NodeId(r["id"].as_u64().unwrap())
}

fn node(s: &Session, id: NodeId) -> vectorcraft_doc::Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

/// Is `(x, y)` inside the area object `id` fills?
fn fills(s: &Session, id: NodeId, x: f64, y: f64) -> bool {
    let (path, rule) = node_path(&node(s, id)).unwrap();
    let w = path.to_bezpath().winding(kurbo::Point::new(x, y));
    match rule {
        vectorcraft_geom::FillRule::EvenOdd => w % 2 != 0,
        vectorcraft_geom::FillRule::NonZero => w != 0,
    }
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn an_o_gets_a_bridge_and_the_original_goes_to_a_hidden_layer() {
    let mut s = session();
    let o = letter_o(&mut s);
    let fill = node(&s, o).appearance.fill_paint();
    let steps = undo_steps(&s);
    let r = s.execute("addon.cutting.bridges", &json!({"width": 4})).unwrap();
    assert_eq!((r["islands"].clone(), r["bridges"].clone(), r["unbridged"].clone()), (json!(1), json!(1), json!(0)));
    assert!(r.get("warning").is_none(), "{r}");
    let made = id_of(&r);
    assert_eq!(s.doc().unwrap().selection.objects, vec![made]);
    assert_eq!(undo_steps(&s), steps + 1, "one undo step");
    // The bridge crosses the top bar in the middle; the rest of the O stays, painted as it was.
    assert!(!fills(&s, made, 150.0, 110.0));
    assert!(fills(&s, made, 130.0, 110.0) && fills(&s, made, 150.0, 190.0) && fills(&s, made, 110.0, 150.0));
    assert_eq!(node(&s, made).appearance.fill_paint(), fill);
    // The original, unchanged, is on a new hidden layer.
    let layer = NodeId(r["original"].as_u64().unwrap());
    let l = node(&s, layer);
    assert_eq!((l.name.as_deref(), l.visible), (Some("Stencil Original"), false));
    let held = s.doc().unwrap().doc.children(Some(layer)).map(|c| c.iter().map(|n| n.id).collect::<Vec<_>>());
    assert_eq!(held, Some(vec![o]));
    // The island is joined to the sheet: the result has none left.
    let again = s.execute("addon.cutting.bridges", &json!({"width": 4})).unwrap();
    assert_eq!(again["islands"], json!(0));
    assert!(again["warning"].as_str().is_some_and(|w| w.contains("no islands")), "{again}");
    assert_eq!(undo_steps(&s), steps + 1, "no islands: no undo step");
    // Undo puts the O back where it was and takes the layer away.
    s.execute("edit.undo", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.node(layer).is_none() && d.node(made).is_none() && d.node(o).is_some_and(|n| n.visible));
}

#[test]
fn keep_original_off_replaces_the_art() {
    let mut s = session();
    let o = letter_o(&mut s);
    let layers = s.doc().unwrap().doc.layers.len();
    let r = s.execute("addon.cutting.bridges", &json!({"width": 4, "keepOriginal": false})).unwrap();
    assert_eq!(r["original"], Value::Null);
    let d = &s.doc().unwrap().doc;
    assert!(d.node(o).is_none() && d.node(id_of(&r)).is_some());
    assert_eq!(d.layers.len(), layers);
}

#[test]
fn the_bridge_is_as_wide_as_asked() {
    for width in [2.0, 8.0] {
        let mut s = session();
        letter_o(&mut s);
        let r = s.execute("addon.cutting.bridges", &json!({"width": width})).unwrap();
        let made = id_of(&r);
        let half = width / 2.0;
        assert!(!fills(&s, made, 150.0 - half * 0.9, 110.0) && !fills(&s, made, 150.0 + half * 0.9, 110.0), "{width}");
        assert!(fills(&s, made, 150.0 - half * 1.2, 110.0) && fills(&s, made, 150.0 + half * 1.2, 110.0), "{width}");
    }
}

#[test]
fn the_width_takes_units() {
    let mut s = session();
    letter_o(&mut s);
    let r = s.execute("addon.cutting.bridges", &json!({"width": "2 mm"})).unwrap();
    let made = id_of(&r);
    // 2 mm is about 5.67 pt.
    assert!(!fills(&s, made, 152.5, 110.0) && fills(&s, made, 154.0, 110.0));
}

#[test]
fn the_shortest_crossing_takes_the_thinnest_bar() {
    let mut s = session();
    // The left bar is 5 pt thick, the others 20.
    shape_with_holes(&mut s, (100.0, 100.0, 100.0, 100.0), &[(105.0, 120.0, 75.0, 60.0)]);
    let r = s.execute("addon.cutting.bridges", &json!({"width": 4, "direction": "shortest"})).unwrap();
    let made = id_of(&r);
    assert!(!fills(&s, made, 102.5, 150.0), "the bridge crosses the thin bar");
    assert!(fills(&s, made, 150.0, 110.0) && fills(&s, made, 190.0, 150.0));
}

#[test]
fn horizontal_bridges_cross_the_sides() {
    let mut s = session();
    letter_o(&mut s);
    let r = s.execute("addon.cutting.bridges", &json!({"width": 4, "direction": "horizontal"})).unwrap();
    let made = id_of(&r);
    assert!(!fills(&s, made, 110.0, 150.0) || !fills(&s, made, 190.0, 150.0));
    assert!(fills(&s, made, 150.0, 110.0) && fills(&s, made, 150.0, 190.0));
}

#[test]
fn a_shape_with_two_holes_gets_a_bridge_for_each() {
    let mut s = session();
    shape_with_holes(&mut s, (100.0, 100.0, 100.0, 50.0), &[(110.0, 110.0, 30.0, 30.0), (160.0, 110.0, 30.0, 30.0)]);
    let r = s.execute("addon.cutting.bridges", &json!({"width": 3, "count": 2})).unwrap();
    assert_eq!((r["islands"].clone(), r["bridges"].clone(), r["unbridged"].clone()), (json!(2), json!(4), json!(0)));
    let made = id_of(&r);
    for x in [125.0, 175.0] {
        assert!(!fills(&s, made, x, 105.0) && !fills(&s, made, x, 145.0), "{x}: a bridge above and below each hole");
    }
}

#[test]
fn type_is_bridged_by_its_glyph_outlines_and_stays_type() {
    let mut s = session();
    let t = NodeId(s.execute("text.create", &json!({"x": 100, "y": 300, "text": "O", "size": 144})).unwrap()["id"].as_u64().unwrap());
    select(&mut s, &[t]);
    let r = s.execute("addon.cutting.bridges", &json!({"width": 3})).unwrap();
    assert_eq!((r["islands"].clone(), r["bridges"].clone(), r["unbridged"].clone()), (json!(1), json!(1), json!(0)), "{r}");
    assert!(matches!(node(&s, id_of(&r)).kind, NodeKind::Path { .. } | NodeKind::Compound { .. }));
    // The type itself is not converted: it is on the hidden layer, still type.
    assert!(matches!(node(&s, t).kind, NodeKind::Text(_)));
    assert!(!node(&s, NodeId(r["original"].as_u64().unwrap())).visible);
}

#[test]
fn the_result_is_painted_as_the_topmost_object_with_outlines() {
    let mut s = session();
    let o = letter_o(&mut s);
    // Type holding only a space draws no outlines, so it does not set the paint, though it is on top.
    let t = NodeId(s.execute("text.create", &json!({"x": 100, "y": 300, "text": " ", "size": 24})).unwrap()["id"].as_u64().unwrap());
    select(&mut s, &[o]);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let red = node(&s, o).appearance.fill_paint();
    select(&mut s, &[o, t]);
    let r = s.execute("addon.cutting.bridges", &json!({"width": 4})).unwrap();
    assert_eq!(node(&s, id_of(&r)).appearance.fill_paint(), red);
}

#[test]
fn hidden_members_of_a_group_are_left_out() {
    let mut s = session();
    let o = letter_o(&mut s);
    // A hidden square that would fill the O's hole, grouped with it.
    let cover = rect(&mut s, 120.0, 120.0, 60.0, 60.0);
    select(&mut s, &[o, cover]);
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    select(&mut s, &[cover]);
    s.execute("object.hide", &json!({})).unwrap();
    select(&mut s, &[g]);
    let r = s.execute("addon.cutting.bridges", &json!({"width": 4})).unwrap();
    assert_eq!((r["islands"].clone(), r["bridges"].clone()), (json!(1), json!(1)), "{r}");
}

#[test]
fn art_without_islands_changes_nothing() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    select(&mut s, &[a]);
    let (steps, count) = (undo_steps(&s), s.doc().unwrap().doc.node_count());
    let r = s.execute("addon.cutting.bridges", &json!({})).unwrap();
    assert_eq!((r["islands"].clone(), r["bridges"].clone(), r["id"].clone()), (json!(0), json!(0), Value::Null));
    assert!(r["warning"].as_str().is_some(), "{r}");
    assert_eq!((undo_steps(&s), s.doc().unwrap().doc.node_count()), (steps, count));
}

#[test]
fn an_island_too_narrow_for_the_width_is_reported() {
    let mut s = session();
    shape_with_holes(&mut s, (100.0, 100.0, 100.0, 100.0), &[(149.0, 120.0, 2.0, 60.0)]);
    let steps = undo_steps(&s);
    let r = s.execute("addon.cutting.bridges", &json!({"width": 4})).unwrap();
    assert_eq!((r["islands"].clone(), r["bridges"].clone(), r["unbridged"].clone()), (json!(1), json!(0), json!(1)));
    assert!(r["warning"].as_str().is_some_and(|w| w.contains("No bridge")), "{r}");
    assert_eq!(undo_steps(&s), steps);
}

#[test]
fn islands_left_without_a_bridge_are_reported_beside_the_ones_bridged() {
    let mut s = session();
    // A wide hole on the left and a 2 pt slot on the right.
    shape_with_holes(&mut s, (100.0, 100.0, 200.0, 100.0), &[(120.0, 120.0, 60.0, 60.0), (249.0, 120.0, 2.0, 60.0)]);
    let steps = undo_steps(&s);
    let r = s.execute("addon.cutting.bridges", &json!({"width": 4})).unwrap();
    assert_eq!((r["islands"].clone(), r["bridges"].clone(), r["unbridged"].clone()), (json!(2), json!(1), json!(1)));
    assert!(r["warning"].as_str().is_some_and(|w| w.contains("could not be joined")), "{r}");
    assert_eq!(undo_steps(&s), steps + 1);
    assert!(!fills(&s, id_of(&r), 150.0, 110.0), "the wide hole is bridged");
}

#[test]
fn each_run_keeps_its_original_on_a_layer_of_its_own() {
    let mut s = session();
    letter_o(&mut s);
    let first = s.execute("addon.cutting.bridges", &json!({"width": 4})).unwrap();
    shape_with_holes(&mut s, (300.0, 100.0, 100.0, 100.0), &[(320.0, 120.0, 60.0, 60.0)]);
    let second = s.execute("addon.cutting.bridges", &json!({"width": 4})).unwrap();
    let name = |r: &Value| node(&s, NodeId(r["original"].as_u64().unwrap())).name;
    assert_eq!((name(&first).as_deref(), name(&second).as_deref()), (Some("Stencil Original"), Some("Stencil Original 2")));
}

#[test]
fn bad_params_are_refused() {
    let mut s = session();
    letter_o(&mut s);
    let steps = undo_steps(&s);
    for (key, p) in [
        ("width", json!({"width": 0})),
        ("width", json!({"width": 5000})),
        ("width", json!({"width": "wide"})),
        ("count", json!({"count": 0})),
        ("count", json!({"count": 1.5})),
        ("count", json!({"count": 17})),
        ("count", json!({"count": "two"})),
        ("direction", json!({"direction": "diagonal"})),
        ("direction", json!({"direction": 90})),
        ("keepOriginal", json!({"keepOriginal": "yes"})),
    ] {
        let e = s.execute("addon.cutting.bridges", &p).unwrap_err().to_string();
        assert!(e.contains(key), "{p}: {e}");
    }
    assert_eq!(undo_steps(&s), steps);
    assert!(s.execute("addon.cutting.bridges", &json!({"width": 4})).is_ok(), "good params run");
}

#[test]
fn selected_objects_without_outlines_stay_where_they_are() {
    for keep in [true, false] {
        let mut s = session();
        let o = letter_o(&mut s);
        // Type holding only a space has no outlines.
        let t = NodeId(s.execute("text.create", &json!({"x": 100, "y": 300, "text": " ", "size": 24})).unwrap()["id"].as_u64().unwrap());
        let layer = |s: &Session, id: NodeId| s.doc().unwrap().doc.layer_of(id);
        let before = layer(&s, t);
        select(&mut s, &[o, t]);
        let r = s.execute("addon.cutting.bridges", &json!({"width": 4, "keepOriginal": keep})).unwrap();
        assert_eq!(r["bridges"], json!(1), "{r}");
        assert!(node(&s, t).visible && layer(&s, t) == before, "keep {keep}: the type stays where it was");
        if keep {
            let held =
                s.doc().unwrap().doc.children(Some(NodeId(r["original"].as_u64().unwrap()))).map(|c| c.iter().map(|n| n.id).collect::<Vec<_>>());
            assert_eq!(held, Some(vec![o]));
        }
    }
}

#[test]
fn the_result_takes_the_place_of_the_topmost_selected_object() {
    let mut s = session();
    let o = letter_o(&mut s);
    let between = rect(&mut s, 400.0, 100.0, 50.0, 50.0);
    let o2 = shape_with_holes(&mut s, (500.0, 100.0, 100.0, 100.0), &[(520.0, 120.0, 60.0, 60.0)]);
    let above = rect(&mut s, 400.0, 300.0, 50.0, 50.0);
    select(&mut s, &[o, o2]);
    let r = s.execute("addon.cutting.bridges", &json!({"width": 4, "keepOriginal": false})).unwrap();
    let made = id_of(&r);
    let d = &s.doc().unwrap().doc;
    let order: Vec<NodeId> = d.children(d.layer_of(made)).unwrap().iter().map(|n| n.id).collect();
    let at = |id: NodeId| order.iter().position(|i| *i == id).unwrap();
    assert!(at(between) < at(made) && at(made) < at(above), "{order:?}");
}

#[test]
fn the_result_is_one_path_when_one_outline_is_left() {
    for (count, plain) in [(1, true), (2, false)] {
        let mut s = session();
        letter_o(&mut s);
        let r = s.execute("addon.cutting.bridges", &json!({"width": 4, "count": count})).unwrap();
        let kind = node(&s, id_of(&r)).kind;
        // One bridge leaves a C, one outline; two leave two pieces.
        assert_eq!(matches!(kind, NodeKind::Path { .. }), plain, "count {count}");
        assert_eq!(matches!(kind, NodeKind::Compound { .. }), !plain, "count {count}");
    }
}
