//! Arrange Tools' commands: the geometry each one leaves, one undo step each, when they are off,
//! and bad params refused without a change.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::Rect;

use crate::Session;

/// A new document whose new art has no stroke, so visual and geometric bounds agree.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].as_u64().unwrap())
}

fn select(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

fn near(a: Rect, b: (f64, f64, f64, f64)) -> bool {
    [(a.x0, b.0), (a.y0, b.1), (a.width(), b.2), (a.height(), b.3)].iter().all(|(x, y)| (x - y).abs() < 1e-6)
}

fn steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// Each of `bad` is refused and leaves the document as it was.
fn refuses(s: &mut Session, cmd: &str, bad: &[Value]) {
    let (before, n) = (s.doc().unwrap().doc.clone(), steps(s));
    for p in bad {
        assert!(s.execute(cmd, p).is_err(), "{cmd} {p}");
    }
    assert_eq!(steps(s), n, "{cmd}");
    assert!(*s.doc().unwrap().doc == *before, "{cmd} changed the document");
}

fn enabled(s: &Session, cmd: &str) -> bool {
    s.commands().into_iter().find(|c| c.id == cmd).unwrap().enabled
}

#[test]
fn grid_lays_objects_out_in_rows_and_columns_from_the_back() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 20.0, 10.0);
    let b = rect(&mut s, 300.0, 50.0, 40.0, 30.0);
    let c = rect(&mut s, 10.0, 200.0, 10.0, 10.0);
    let d = rect(&mut s, 50.0, 5.0, 30.0, 20.0);
    select(&mut s, &[d, c, b, a]);
    let n = steps(&s);
    // Four objects: two columns. The grid starts at the selection's top-left (10, 5).
    let r = s.execute("addon.arrange.grid", &json!({"gapX": 5, "gapY": 2})).unwrap();
    assert_eq!(r["count"], json!(4));
    assert_eq!(steps(&s), n + 1, "one undo step");
    // Columns 20 and 40 wide (the widest in each), rows 30 and 20 tall; each object centred.
    assert!(near(bounds(&s, a), (10.0, 15.0, 20.0, 10.0)), "{:?}", bounds(&s, a));
    assert!(near(bounds(&s, b), (35.0, 5.0, 40.0, 30.0)), "{:?}", bounds(&s, b));
    assert!(near(bounds(&s, c), (15.0, 42.0, 10.0, 10.0)), "{:?}", bounds(&s, c));
    assert!(near(bounds(&s, d), (40.0, 37.0, 30.0, 20.0)), "{:?}", bounds(&s, d));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(bounds(&s, a), (100.0, 100.0, 20.0, 10.0)));
}

#[test]
fn grid_lays_out_a_row_or_a_column_and_centres_on_the_artboard() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 20.0, 20.0);
    let b = rect(&mut s, 100.0, 100.0, 20.0, 20.0);
    select(&mut s, &[a, b]);
    s.execute("addon.arrange.grid", &json!({"layout": "row"})).unwrap();
    assert!(near(bounds(&s, b), (30.0, 0.0, 20.0, 20.0)), "gapX defaults to 10");
    s.execute("addon.arrange.grid", &json!({"layout": "column", "gapX": 4})).unwrap();
    assert!(near(bounds(&s, b), (0.0, 24.0, 20.0, 20.0)), "gapY follows gapX");
    // 20 × 44 centred on the 400 × 300 artboard.
    s.execute("addon.arrange.grid", &json!({"layout": "column", "gapY": 4, "centerOnArtboard": true})).unwrap();
    assert!(near(bounds(&s, a), (190.0, 128.0, 20.0, 20.0)), "{:?}", bounds(&s, a));
    // In place already: no step.
    let n = steps(&s);
    s.execute("addon.arrange.grid", &json!({"layout": "column", "gapY": 4, "centerOnArtboard": true})).unwrap();
    assert_eq!(steps(&s), n);
    // More columns than objects: one row.
    s.execute("addon.arrange.grid", &json!({"columns": 50, "gapX": 0})).unwrap();
    assert!(near(bounds(&s, b), (210.0, 128.0, 20.0, 20.0)), "{:?}", bounds(&s, b));
}

#[test]
fn grid_measures_visual_bounds() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 20.0, 20.0);
    let b = rect(&mut s, 100.0, 0.0, 20.0, 20.0);
    select(&mut s, &[a, b]);
    s.execute("paint.setStroke", &json!({"color": "#000000"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 4})).unwrap();
    s.execute("addon.arrange.grid", &json!({"layout": "row", "gapX": 0})).unwrap();
    // Visual boxes 24 wide, touching.
    assert!(near(bounds(&s, b), (24.0, 0.0, 20.0, 20.0)), "{:?}", bounds(&s, b));
}

#[test]
fn swap_trades_places_and_optionally_sizes() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 20.0, 10.0);
    let b = rect(&mut s, 100.0, 100.0, 40.0, 40.0);
    let c = rect(&mut s, 200.0, 200.0, 5.0, 5.0);
    select(&mut s, &[a]);
    assert!(!enabled(&s, "addon.arrange.swap"));
    select(&mut s, &[a, b, c]);
    assert!(!enabled(&s, "addon.arrange.swap"));
    refuses(&mut s, "addon.arrange.swap", &[json!({})]);
    select(&mut s, &[a, b]);
    assert!(enabled(&s, "addon.arrange.swap"));
    let n = steps(&s);
    s.execute("addon.arrange.swap", &json!({})).unwrap();
    assert_eq!(steps(&s), n + 1);
    // Centres (10, 5) and (120, 120) swap; sizes stay.
    assert!(near(bounds(&s, a), (110.0, 115.0, 20.0, 10.0)), "{:?}", bounds(&s, a));
    assert!(near(bounds(&s, b), (-10.0, -15.0, 40.0, 40.0)), "{:?}", bounds(&s, b));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("addon.arrange.swap", &json!({"size": true})).unwrap();
    assert!(near(bounds(&s, a), (100.0, 100.0, 40.0, 40.0)), "{:?}", bounds(&s, a));
    assert!(near(bounds(&s, b), (0.0, 0.0, 20.0, 10.0)), "{:?}", bounds(&s, b));
}

#[test]
fn swap_with_sizes_scales_strokes_as_asked() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 100.0, 0.0, 20.0, 20.0);
    select(&mut s, &[a, b]);
    s.execute("paint.setStroke", &json!({"color": "#000000"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 2})).unwrap();
    let weight = |s: &Session, id| s.doc().unwrap().doc.node(id).unwrap().appearance.stroke_width();
    s.execute("addon.arrange.swap", &json!({"size": true})).unwrap();
    assert_eq!((weight(&s, a), weight(&s, b)), (2.0, 2.0), "off by default (the preference)");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("addon.arrange.swap", &json!({"size": true, "scaleStrokes": true})).unwrap();
    assert!((weight(&s, a) - 4.0).abs() < 1e-9 && (weight(&s, b) - 1.0).abs() < 1e-9, "{} {}", weight(&s, a), weight(&s, b));
}

#[test]
fn replace_puts_a_copy_of_the_top_object_in_each_place() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 20.0, 40.0);
    let b = rect(&mut s, 100.0, 100.0, 40.0, 40.0);
    let top = rect(&mut s, 200.0, 200.0, 10.0, 5.0);
    select(&mut s, &[top]);
    assert!(!enabled(&s, "addon.arrange.replace"));
    refuses(&mut s, "addon.arrange.replace", &[json!({})]);
    select(&mut s, &[a, b, top]);
    let n = steps(&s);
    let r = s.execute("addon.arrange.replace", &json!({})).unwrap();
    assert_eq!(steps(&s), n + 1);
    let ids: Vec<NodeId> = r["ids"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect();
    assert_eq!(ids.len(), 2);
    let st = s.doc().unwrap();
    assert!(st.doc.node(a).is_none() && st.doc.node(b).is_none(), "replaced objects deleted");
    assert!(st.doc.node(top).is_some(), "the top object stays");
    assert_eq!(st.selection.objects, ids, "the copies are selected");
    // Kept size, centred on (10, 20) and (120, 120); each in its object's place (bottom, then middle).
    assert!(near(bounds(&s, ids[0]), (5.0, 17.5, 10.0, 5.0)), "{:?}", bounds(&s, ids[0]));
    assert!(near(bounds(&s, ids[1]), (115.0, 117.5, 10.0, 5.0)), "{:?}", bounds(&s, ids[1]));
    let layer = &s.doc().unwrap().doc.layers[0];
    let order: Vec<NodeId> = layer.children().unwrap().iter().map(|n| n.id).collect();
    assert_eq!(order, [ids[0], ids[1], top]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(a).is_some());
}

#[test]
fn replace_fits_the_copies_and_can_keep_the_originals() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 20.0, 40.0);
    let top = rect(&mut s, 200.0, 200.0, 10.0, 5.0);
    select(&mut s, &[a, top]);
    let r = s.execute("addon.arrange.replace", &json!({"fit": "bounds", "keep": true})).unwrap();
    let copy = NodeId(r["ids"][0].as_u64().unwrap());
    assert!(near(bounds(&s, copy), (0.0, 0.0, 20.0, 40.0)), "{:?}", bounds(&s, copy));
    let order: Vec<NodeId> = s.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id).collect();
    assert_eq!(order, [a, copy, top], "kept: the copy goes right above it");
    s.execute("edit.undo", &json!({})).unwrap();
    select(&mut s, &[a, top]);
    let r = s.execute("addon.arrange.replace", &json!({"fit": "proportional"})).unwrap();
    let copy = NodeId(r["ids"][0].as_u64().unwrap());
    // 10 × 5 into 20 × 40: twice as large, centred on (10, 20).
    assert!(near(bounds(&s, copy), (0.0, 15.0, 20.0, 10.0)), "{:?}", bounds(&s, copy));
}

#[test]
fn size_gives_each_object_a_width_and_height() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 20.0, 10.0);
    let b = rect(&mut s, 100.0, 100.0, 40.0, 40.0);
    select(&mut s, &[a, b]);
    let n = steps(&s);
    s.execute("addon.arrange.size", &json!({"width": 30})).unwrap();
    assert_eq!(steps(&s), n + 1);
    // About each centre; the height stays.
    assert!(near(bounds(&s, a), (-5.0, 0.0, 30.0, 10.0)), "{:?}", bounds(&s, a));
    assert!(near(bounds(&s, b), (105.0, 100.0, 30.0, 40.0)), "{:?}", bounds(&s, b));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("addon.arrange.size", &json!({"width": 40, "proportional": true, "reference": 0})).unwrap();
    assert!(near(bounds(&s, a), (0.0, 0.0, 40.0, 20.0)), "{:?}", bounds(&s, a));
    assert!(near(bounds(&s, b), (100.0, 100.0, 40.0, 40.0)), "{:?}", bounds(&s, b));
    s.execute("addon.arrange.size", &json!({"width": 10, "height": 12, "proportional": true, "reference": 8})).unwrap();
    assert!(near(bounds(&s, a), (30.0, 8.0, 10.0, 12.0)), "both given: both set, {:?}", bounds(&s, a));
}

#[test]
fn bad_params_are_refused() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 20.0, 10.0);
    let b = rect(&mut s, 100.0, 100.0, 40.0, 40.0);
    select(&mut s, &[a, b]);
    refuses(
        &mut s,
        "addon.arrange.grid",
        &[
            json!({"layout": "spiral"}),
            json!({"columns": 0}),
            json!({"columns": 1.5}),
            json!({"columns": 1e12}),
            json!({"gapX": 1e308}),
            json!({"gapY": "10"}),
            json!({"centerOnArtboard": "yes"}),
        ],
    );
    refuses(&mut s, "addon.arrange.swap", &[json!({"size": 1}), json!({"scaleStrokes": "no"})]);
    refuses(&mut s, "addon.arrange.replace", &[json!({"fit": "stretch"}), json!({"keep": 0})]);
    refuses(
        &mut s,
        "addon.arrange.size",
        &[
            json!({}),
            json!({"width": 0}),
            json!({"height": -5}),
            json!({"width": 1e308}),
            json!({"width": "10"}),
            json!({"width": 10, "reference": 9}),
            json!({"width": 10, "proportional": "yes"}),
        ],
    );
}

#[test]
fn off_without_a_selection() {
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 20.0, 10.0);
    s.execute("select.none", &json!({})).unwrap();
    for c in [
        "addon.arrange.grid",
        "addon.arrange.swap",
        "addon.arrange.replace",
        "addon.arrange.size",
        "addon.arrange.groupOverlapping",
        "addon.arrange.randomizeOpacity",
        "addon.arrange.sort",
    ] {
        assert!(!enabled(&s, c), "{c}");
    }
    assert!(!enabled(&Session::new(), "addon.arrange.swap"));
}

fn ids_of(v: &Value) -> Vec<NodeId> {
    v.as_array().unwrap().iter().map(|i| NodeId(i.as_u64().unwrap())).collect()
}

fn children(s: &Session, id: NodeId) -> Vec<NodeId> {
    s.doc().unwrap().doc.node(id).unwrap().children().unwrap().iter().map(|n| n.id).collect()
}

#[test]
fn group_overlapping_groups_chains_and_leaves_loners() {
    let mut s = session();
    // a–b–c a chain (c only touches b), d alone, e over f.
    let a = rect(&mut s, 0.0, 0.0, 20.0, 20.0);
    let d = rect(&mut s, 200.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 15.0, 15.0, 20.0, 20.0);
    let e = rect(&mut s, 100.0, 100.0, 30.0, 30.0);
    let c = rect(&mut s, 35.0, 0.0, 10.0, 20.0);
    let f = rect(&mut s, 110.0, 110.0, 5.0, 5.0);
    select(&mut s, &[a, b, c, d, e, f]);
    let n = steps(&s);
    let r = s.execute("addon.arrange.groupOverlapping", &json!({})).unwrap();
    assert_eq!(steps(&s), n + 1, "one undo step");
    assert_eq!(r["grouped"], json!(5));
    let groups = ids_of(&r["groups"]);
    assert_eq!(groups.len(), 2);
    assert_eq!(children(&s, groups[0]), [a, b, c], "members keep their stacking order");
    assert_eq!(children(&s, groups[1]), [e, f]);
    // Each group where its frontmost member was: a d [abc] [ef] → d, group(abc) above it, then (ef).
    let layer: Vec<NodeId> = s.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id).collect();
    assert_eq!(layer, [d, groups[0], groups[1]]);
    let sel = &s.doc().unwrap().selection.objects;
    assert!(sel.contains(&groups[0]) && sel.contains(&groups[1]) && sel.contains(&d) && sel.len() == 3, "{sel:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 6);
}

#[test]
fn group_overlapping_keeps_layers_apart_and_measures_visual_bounds() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 20.0, 20.0);
    let gap = rect(&mut s, 22.0, 0.0, 20.0, 20.0);
    s.execute("layer.new", &json!({})).unwrap();
    let other = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    select(&mut s, &[a, gap, other]);
    // 2 pt apart: nothing overlaps within a layer, and a and `other` are in different layers.
    let n = steps(&s);
    let r = s.execute("addon.arrange.groupOverlapping", &json!({})).unwrap();
    assert_eq!((r["grouped"].clone(), steps(&s)), (json!(0), n), "no groups, no step");
    // A 4 pt stroke reaches 2 pt out: the two in the first layer now touch.
    select(&mut s, &[a, gap]);
    s.execute("paint.setStroke", &json!({"color": "#000000"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 4})).unwrap();
    select(&mut s, &[a, gap, other]);
    let r = s.execute("addon.arrange.groupOverlapping", &json!({})).unwrap();
    let groups = ids_of(&r["groups"]);
    assert_eq!(groups.len(), 1);
    assert_eq!(children(&s, groups[0]), [a, gap]);
    let doc = &s.doc().unwrap().doc;
    assert_ne!(doc.layer_of(other), doc.layer_of(a), "left in its own layer");
}

#[test]
fn group_overlapping_is_off_with_one_object_and_scales_to_many() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 20.0, 20.0);
    select(&mut s, &[a]);
    assert!(!enabled(&s, "addon.arrange.groupOverlapping"));
    // A long row of pairs: each pair overlaps, neighbouring pairs don't.
    let rects: Vec<(usize, Rect)> =
        (0..4000).map(|i| (0, Rect::new(i as f64 * 10.0, 0.0, i as f64 * 10.0 + if i % 2 == 0 { 12.0 } else { 3.0 }, 5.0))).collect();
    let sets = super::overlapping(&rects);
    assert_eq!(sets.len(), 2000);
    assert!(sets.iter().all(|s| s.len() == 2 && s[1] == s[0] + 1));
}

fn opacity(s: &Session, id: NodeId) -> f32 {
    s.doc().unwrap().doc.node(id).unwrap().opacity
}

#[test]
fn randomize_opacity_gives_each_object_its_own_opacity_within_the_range() {
    let mut s = session();
    let ids: Vec<NodeId> = (0..20).map(|i| rect(&mut s, i as f64 * 10.0, 0.0, 5.0, 5.0)).collect();
    select(&mut s, &ids);
    let n = steps(&s);
    let r = s.execute("addon.arrange.randomizeOpacity", &json!({"min": 30, "max": 60, "seed": 7})).unwrap();
    assert_eq!(r["count"], json!(20));
    assert_eq!(steps(&s), n + 1, "one undo step");
    let first: Vec<f32> = ids.iter().map(|id| opacity(&s, *id)).collect();
    assert!(first.iter().all(|o| (0.3..=0.6).contains(o)), "{first:?}");
    assert!(first.windows(2).any(|w| w[0] != w[1]), "each its own: {first:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(ids.iter().all(|id| opacity(&s, *id) == 1.0), "undone");
    // The same seed repeats them; another seed doesn't.
    s.execute("addon.arrange.randomizeOpacity", &json!({"min": 30, "max": 60, "seed": 7})).unwrap();
    assert_eq!(ids.iter().map(|id| opacity(&s, *id)).collect::<Vec<_>>(), first);
    s.execute("addon.arrange.randomizeOpacity", &json!({"min": 30, "max": 60, "seed": 8})).unwrap();
    assert_ne!(ids.iter().map(|id| opacity(&s, *id)).collect::<Vec<_>>(), first);
    // Defaults 30 to 100; min = max gives every object that opacity.
    s.execute("addon.arrange.randomizeOpacity", &json!({})).unwrap();
    assert!(ids.iter().all(|id| (0.3..=1.0).contains(&opacity(&s, *id))));
    s.execute("addon.arrange.randomizeOpacity", &json!({"min": 45, "max": 45})).unwrap();
    assert!(ids.iter().all(|id| opacity(&s, *id) == 0.45));
}

#[test]
fn randomize_opacity_takes_ids_and_leaves_the_rest() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 5.0, 5.0);
    let b = rect(&mut s, 10.0, 0.0, 5.0, 5.0);
    select(&mut s, &[a, b]);
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    let c = rect(&mut s, 20.0, 0.0, 5.0, 5.0);
    s.execute("select.none", &json!({})).unwrap();
    assert!(!enabled(&s, "addon.arrange.randomizeOpacity"));
    s.execute("addon.arrange.randomizeOpacity", &json!({"ids": [c.0], "min": 10, "max": 10})).unwrap();
    assert_eq!((opacity(&s, a), opacity(&s, b), opacity(&s, c)), (1.0, 1.0, 0.1));
    // A group takes one opacity of its own; the objects inside keep theirs.
    select(&mut s, &[g]);
    s.execute("addon.arrange.randomizeOpacity", &json!({"min": 20, "max": 20})).unwrap();
    assert_eq!((opacity(&s, g), opacity(&s, a), opacity(&s, b)), (0.2, 1.0, 1.0));
}

#[test]
fn randomize_opacity_refuses_bad_params() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 20.0, 10.0);
    select(&mut s, &[a]);
    refuses(
        &mut s,
        "addon.arrange.randomizeOpacity",
        &[
            json!({"min": -1}),
            json!({"max": 101}),
            json!({"min": 80, "max": 20}),
            json!({"min": 1e308}),
            json!({"max": "50"}),
            json!({"seed": -1}),
            json!({"seed": 1.5}),
            json!({"seed": "x"}),
            json!({"ids": [999_999]}),
            json!({"ids": "x"}),
        ],
    );
    let e = s.execute("addon.arrange.randomizeOpacity", &json!({"min": 80, "max": 20})).unwrap_err().to_string();
    assert!(e.contains("`min` is greater than `max`"), "{e}");
}

fn layer_order(s: &Session) -> Vec<NodeId> {
    s.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id).collect()
}

/// Three squares: `a` 30 wide at x 0, `b` 10 wide at x 100, `c` 20 wide at x 200, all selected.
fn squares(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    let a = rect(s, 0.0, 0.0, 30.0, 30.0);
    let b = rect(s, 100.0, 0.0, 10.0, 10.0);
    let c = rect(s, 200.0, 0.0, 20.0, 20.0);
    select(s, &[a, b, c]);
    (a, b, c)
}

#[test]
fn sort_by_area_lines_objects_up_along_x() {
    let mut s = session();
    let (a, b, c) = squares(&mut s);
    let n = steps(&s);
    let r = s.execute("addon.arrange.sort", &json!({"by": "area", "apply": "x"})).unwrap();
    assert_eq!(r["count"], json!(3));
    assert_eq!(steps(&s), n + 1, "one undo step");
    // Smallest first: b, c, a take the left edges 0, 100, 200; y stays.
    assert!(near(bounds(&s, b), (0.0, 0.0, 10.0, 10.0)), "{:?}", bounds(&s, b));
    assert!(near(bounds(&s, c), (100.0, 0.0, 20.0, 20.0)), "{:?}", bounds(&s, c));
    assert!(near(bounds(&s, a), (200.0, 0.0, 30.0, 30.0)), "{:?}", bounds(&s, a));
    // Large to small: a, c, b.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("addon.arrange.sort", &json!({"by": "area", "apply": "x", "order": "descending"})).unwrap();
    assert_eq!((bounds(&s, a).x0, bounds(&s, c).x0, bounds(&s, b).x0), (0.0, 100.0, 200.0));
    // Already in order: no step.
    let n = steps(&s);
    s.execute("addon.arrange.sort", &json!({"by": "area", "apply": "x", "order": "descending"})).unwrap();
    assert_eq!(steps(&s), n);
}

#[test]
fn sort_distributes_values_evenly() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 10.0, 50.0, 10.0, 50.0);
    let c = rect(&mut s, 200.0, 0.0, 10.0, 20.0);
    select(&mut s, &[a, b, c]);
    // Left edges 0, 10, 200 step evenly: 0, 100, 200.
    s.execute("addon.arrange.sort", &json!({"by": "x", "apply": "x", "distribute": true})).unwrap();
    assert_eq!((bounds(&s, a).x0, bounds(&s, b).x0, bounds(&s, c).x0), (0.0, 100.0, 200.0));
    // Heights that step evenly with x (10 to 50), each about its centre.
    s.execute("addon.arrange.sort", &json!({"by": "x", "apply": "height", "distribute": true})).unwrap();
    assert!(near(bounds(&s, a), (0.0, 0.0, 10.0, 10.0)), "{:?}", bounds(&s, a));
    assert!(near(bounds(&s, b), (100.0, 60.0, 10.0, 30.0)), "{:?}", bounds(&s, b));
    assert!(near(bounds(&s, c), (200.0, -15.0, 10.0, 50.0)), "{:?}", bounds(&s, c));
}

#[test]
fn sort_restacks_within_each_layer() {
    let mut s = session();
    let (a, b, c) = squares(&mut s);
    // Largest at the back.
    s.execute("addon.arrange.sort", &json!({"by": "area", "apply": "stacking", "order": "descending"})).unwrap();
    assert_eq!(layer_order(&s), [a, c, b]);
    s.execute("addon.arrange.sort", &json!({})).unwrap();
    assert_eq!(layer_order(&s), [b, c, a], "by area, small to large, into the stacking order by default");
    // An object in another layer keeps its layer; the others trade places among themselves.
    s.execute("layer.new", &json!({})).unwrap();
    let big = rect(&mut s, 0.0, 100.0, 50.0, 50.0);
    select(&mut s, &[a, b, c, big]);
    s.execute("addon.arrange.sort", &json!({"order": "descending"})).unwrap();
    assert_eq!(layer_order(&s), [a, c, b]);
    let doc = &s.doc().unwrap().doc;
    assert_ne!(doc.layer_of(big), doc.layer_of(a), "left in its own layer");
}

#[test]
fn sort_by_stacking_and_opacity() {
    let mut s = session();
    let (a, b, c) = squares(&mut s);
    // Back to front a, b, c; reversed: c, b, a take the left edges 0, 100, 200.
    s.execute("addon.arrange.sort", &json!({"by": "stacking", "apply": "x", "order": "descending"})).unwrap();
    assert_eq!((bounds(&s, c).x0, bounds(&s, b).x0, bounds(&s, a).x0), (0.0, 100.0, 200.0));
    for (id, o) in [(a, 20), (b, 80), (c, 50)] {
        s.execute("object.setProps", &json!({"ids": [id.0], "opacity": o})).unwrap();
    }
    select(&mut s, &[a, b, c]);
    let opacity = |s: &Session, id| s.doc().unwrap().doc.node(id).unwrap().opacity;
    // By x, large to small: a (x 200) gets the least opacity, then b, then c.
    s.execute("addon.arrange.sort", &json!({"by": "x", "apply": "opacity", "order": "descending"})).unwrap();
    assert_eq!((opacity(&s, a), opacity(&s, b), opacity(&s, c)), (0.2, 0.5, 0.8));
    // By opacity into the stacking order, large to small: the most opaque at the back.
    s.execute("addon.arrange.sort", &json!({"by": "opacity", "order": "descending"})).unwrap();
    assert_eq!(layer_order(&s), [c, b, a]);
}

#[test]
fn sort_by_area_into_areas_scales_in_proportion() {
    let mut s = session();
    let (a, b, c) = squares(&mut s);
    // Areas 900, 100, 400 by x: a (x 0) 100, b 500, c 900.
    s.execute("addon.arrange.sort", &json!({"by": "x", "apply": "area", "distribute": true})).unwrap();
    let side = |s: &Session, id| bounds(s, id).width();
    assert!((side(&s, a) - 10.0).abs() < 1e-6 && (side(&s, b) - 500f64.sqrt()).abs() < 1e-6 && (side(&s, c) - 30.0).abs() < 1e-6);
    assert!((bounds(&s, b).height() - 500f64.sqrt()).abs() < 1e-6, "in proportion");
}

#[test]
fn sort_in_random_order_repeats_with_a_seed() {
    let mut s = session();
    let ids: Vec<NodeId> = (0..12).map(|i| rect(&mut s, i as f64 * 10.0, 0.0, 5.0, 5.0)).collect();
    select(&mut s, &ids);
    s.execute("addon.arrange.sort", &json!({"order": "random", "seed": 5})).unwrap();
    let first = layer_order(&s);
    assert_ne!(first, ids, "shuffled");
    let mut sorted = first.clone();
    sorted.sort_by_key(|i| i.0);
    assert_eq!(sorted, ids, "the same objects");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(layer_order(&s), ids);
    s.execute("addon.arrange.sort", &json!({"order": "random", "seed": 5})).unwrap();
    assert_eq!(layer_order(&s), first, "the same seed, the same order");
}

#[test]
fn sort_takes_ids_and_refuses_bad_params() {
    let mut s = session();
    let (a, b, c) = squares(&mut s);
    select(&mut s, &[a]);
    assert!(!enabled(&s, "addon.arrange.sort"));
    s.execute("addon.arrange.sort", &json!({"ids": [b.0, a.0], "apply": "x"})).unwrap();
    assert_eq!((bounds(&s, b).x0, bounds(&s, a).x0, bounds(&s, c).x0), (0.0, 100.0, 200.0), "c is left alone");
    select(&mut s, &[a, b, c]);
    refuses(
        &mut s,
        "addon.arrange.sort",
        &[
            json!({"by": "color"}),
            json!({"apply": 3}),
            json!({"order": "up"}),
            json!({"distribute": "yes"}),
            json!({"seed": -1}),
            json!({"seed": 0.5}),
            json!({"scaleStrokes": 1}),
            json!({"ids": "x"}),
            json!({"ids": [999_999]}),
        ],
    );
}
