//! Tests for `addon.cutting.banner`: hem lines and grommet marks on the sides of a banner.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{Appearance, Node, NodeKind};
use vectorcraft_geom::{Point, Rect, shapes};

use super::{grommet_centers, hem_lines};
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

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

/// Runs Banner Hems and Grommets with `p` and gives the first group it made.
fn banner(s: &mut Session, p: Value) -> (Value, Node) {
    let r = s.execute("addon.cutting.banner", &p).unwrap();
    let g = node(s, id_of(&r));
    (r, g)
}

fn round(p: Point) -> (f64, f64) {
    let r = |v: f64| (v * 1000.0).round() / 1000.0;
    (r(p.x), r(p.y))
}

/// The hem lines in group `g` as their two ends.
fn hems(g: &Node) -> Vec<((f64, f64), (f64, f64))> {
    g.children()
        .unwrap()
        .iter()
        .filter(|c| c.name.as_deref() == Some("Hem"))
        .map(|c| match &c.kind {
            NodeKind::Path { path, .. } => {
                let a: Vec<Point> = path.anchors().map(|(_, _, a)| a.p).collect();
                assert_eq!(a.len(), 2, "a hem line is one straight segment");
                (round(a[0]), round(a[1]))
            }
            other => panic!("not a path: {other:?}"),
        })
        .collect()
}

/// The grommet marks in group `g` as their centers.
fn grommets(g: &Node) -> Vec<(f64, f64)> {
    g.children().unwrap().iter().filter(|c| c.name.as_deref() == Some("Grommet")).map(|c| round(c.geometric_bounds().unwrap().center())).collect()
}

#[test]
fn hems_and_grommets_go_round_a_selected_banner() {
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 400.0, 200.0);
    let undo = s.doc().unwrap().history.undo.len();
    let (r, g) = banner(&mut s, json!({"hem": 20, "grommetInset": 10, "spacing": 150, "diameter": 8}));
    assert_eq!(g.name.as_deref(), Some("Banner Marks"));
    assert_eq!(
        hems(&g),
        vec![((0.0, 20.0), (400.0, 20.0)), ((380.0, 0.0), (380.0, 200.0)), ((0.0, 180.0), (400.0, 180.0)), ((20.0, 0.0), (20.0, 200.0))]
    );
    // 380 pt along the top and bottom need 3 gaps at most 150 apart, 180 down the sides 2; the
    // corners are shared.
    assert_eq!(
        grommets(&g),
        vec![
            (10.0, 10.0),
            (136.667, 10.0),
            (263.333, 10.0),
            (390.0, 10.0),
            (390.0, 100.0),
            (390.0, 190.0),
            (263.333, 190.0),
            (136.667, 190.0),
            (10.0, 190.0),
            (10.0, 100.0)
        ]
    );
    assert_eq!(r["hems"], 4);
    assert_eq!(r["grommets"], 10);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    assert_eq!(s.doc().unwrap().selection.objects, [g.id]);
}

#[test]
fn a_grommet_is_a_circle_with_a_cross_stroked_in_registration() {
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let (_, g) = banner(&mut s, json!({"hem": 0, "grommetInset": 10, "diameter": 6, "weight": 0.5}));
    let first = g.children().unwrap().iter().find(|c| c.name.as_deref() == Some("Grommet")).cloned().unwrap();
    assert_eq!(first.geometric_bounds(), Some(Rect::new(7.0, 7.0, 13.0, 13.0)));
    let parts = first.children().unwrap();
    assert_eq!(parts.len(), 3, "a circle and two lines");
    for c in parts.iter() {
        assert!(matches!(c.kind, NodeKind::Path { .. }), "a path");
        assert_eq!(c.appearance.fill_paint(), Paint::None);
        assert!(c.appearance.stroke_paint().is_registration());
        assert_eq!(c.appearance.stroke_width(), 0.5);
    }
}

#[test]
fn every_artboard_is_marked_when_nothing_is_selected() {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 900, "y": 0, "width": 200, "height": 150})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let r = s.execute("addon.cutting.banner", &json!({"grommets": false})).unwrap();
    let ids: Vec<NodeId> = r["ids"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect();
    assert_eq!(ids.len(), 2);
    let second = node(&s, ids[1]);
    assert_eq!(hems(&second)[0], ((900.0, 72.0), (1100.0, 72.0)), "the default hem is 72 pt");
    assert!(grommets(&second).is_empty());
    assert_eq!(r["grommets"], 0);
}

#[test]
fn only_the_chosen_sides_are_marked() {
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 300.0, 100.0);
    let (_, g) = banner(&mut s, json!({"hem": 10, "grommetInset": 5, "spacing": 1000, "right": false, "bottom": false, "left": false}));
    assert_eq!(hems(&g), vec![((0.0, 10.0), (300.0, 10.0))]);
    assert_eq!(grommets(&g), vec![(5.0, 5.0), (295.0, 5.0)]);
}

#[test]
fn units_are_read_and_bad_params_are_refused() {
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 400.0, 400.0);
    let (_, g) = banner(&mut s, json!({"hem": "1 in", "grommetInset": "0.5 in", "spacing": "10 in"}));
    assert_eq!(hems(&g)[0], ((0.0, 72.0), (400.0, 72.0)));
    assert_eq!(grommets(&g)[0], (36.0, 36.0));
    let before = s.doc().unwrap().history.undo.len();
    for p in [
        json!({"hem": -1}),
        json!({"hem": "x"}),
        json!({"spacing": 0}),
        json!({"diameter": 0}),
        json!({"weight": 1e9}),
        json!({"top": "yes"}),
        json!({"top": false, "right": false, "bottom": false, "left": false}),
        json!({"hem": 0, "grommets": false}),
        json!({"hem": 250}),
        json!({"grommetInset": 250}),
        json!({"spacing": 0.001}),
        json!({"swatch": "no such swatch"}),
    ] {
        assert!(s.execute("addon.cutting.banner", &p).is_err(), "{p} is refused");
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), before, "a refused run changes nothing");
}

#[test]
fn hems_and_grommets_on_one_side_may_go_past_the_middle() {
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 300.0, 100.0);
    let one = json!({"right": false, "bottom": false, "left": false, "spacing": 1000});
    let with = |extra: Value| {
        let mut p = one.clone();
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        p
    };
    let (_, g) = banner(&mut s, with(json!({"hem": 60, "grommetInset": 70})));
    assert_eq!(hems(&g), vec![((0.0, 60.0), (300.0, 60.0))]);
    assert_eq!(grommets(&g), vec![(70.0, 70.0), (230.0, 70.0)]);
    for p in [with(json!({"hem": 100})), with(json!({"hem": 0, "grommetInset": 100})), with(json!({"grommetInset": 151}))] {
        assert!(s.execute("addon.cutting.banner", &p).is_err(), "{p} reaches the far edge or past the ends");
    }
    // A grommet inset of half the width puts the grommets of a side on one point: one grommet.
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 100.0, 300.0);
    let (_, g) = banner(&mut s, with(json!({"hem": 0, "grommetInset": 50, "spacing": 10})));
    assert_eq!(grommets(&g), vec![(50.0, 50.0)]);
}

#[test]
fn hems_and_grommets_on_opposite_sides_stay_apart() {
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 300.0, 100.0);
    let pair = |p: Value| {
        let mut q = json!({"right": false, "left": false, "spacing": 1000});
        q.as_object_mut().unwrap().extend(p.as_object().unwrap().clone());
        q
    };
    // Half the height would put the top and bottom marks on the same line.
    let refused = [
        pair(json!({"hem": 50})),
        pair(json!({"hem": 0, "grommetInset": 50})),
        json!({"hem": 0, "grommetInset": 150, "top": false, "bottom": false}),
    ];
    for p in refused {
        assert!(s.execute("addon.cutting.banner", &p).is_err(), "{p} is refused");
    }
    let (_, g) = banner(&mut s, pair(json!({"hem": 49.5, "grommetInset": 49.5})));
    assert_eq!(hems(&g), vec![((0.0, 49.5), (300.0, 49.5)), ((0.0, 50.5), (300.0, 50.5))]);
    assert_eq!(grommets(&g), vec![(49.5, 49.5), (250.5, 49.5), (250.5, 50.5), (49.5, 50.5)]);
}

#[test]
fn the_marks_go_outside_a_clip_group_edited_in_isolation() {
    let mut s = session();
    let parent = s.doc().unwrap().target_parent().unwrap();
    let (g, art) = s
        .edit("clip group", |d, _| {
            let clip = Arc::new(Node::path(d.alloc_id(), shapes::rectangle(Rect::new(100.0, 100.0, 150.0, 140.0)), Appearance::default()));
            let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 400.0, 200.0)), Appearance::default());
            let art_id = art.id;
            let mut g = Node::group(d.alloc_id(), vec![clip, Arc::new(art)]);
            if let NodeKind::Group { clip, .. } = &mut g.kind {
                *clip = true;
            }
            let id = g.id;
            d.insert(parent, usize::MAX, g)?;
            Ok((id, art_id))
        })
        .unwrap();
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    s.execute("select.set", &json!({"ids": [art.0]})).unwrap();
    let (_, marks) = banner(&mut s, json!({"hem": 20, "grommets": false}));
    // The clipping path would hide marks inside the clip group.
    assert_eq!(s.doc().unwrap().doc.parent_of(marks.id), parent);
    assert_eq!(hems(&marks)[0], ((0.0, 20.0), (400.0, 20.0)), "the selected art is the banner");
}

#[test]
fn all_artboards_marks_the_artboards_even_with_a_selection() {
    let mut s = session();
    rect(&mut s, 10.0, 10.0, 100.0, 100.0);
    let (_, g) = banner(&mut s, json!({"allArtboards": true, "grommets": false}));
    assert_eq!(hems(&g)[0], ((0.0, 72.0), (800.0, 72.0)));
}

#[test]
fn an_empty_document_has_no_banner_to_mark_when_artboards_are_off() {
    let mut s = session();
    assert!(s.execute("addon.cutting.banner", &json!({"allArtboards": false})).is_err());
}

#[test]
fn a_side_shorter_than_the_spacing_has_a_grommet_at_each_end() {
    let r = Rect::new(0.0, 0.0, 100.0, 50.0);
    let c = grommet_centers(r, 0.0, 1000.0, [true, false, false, false], 10).unwrap();
    assert_eq!(c, vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0)]);
    // Spacing that divides the side exactly does not add a grommet, even when the division
    // rounds up (2.1 / 0.3 is 7.000000000000001).
    let c = grommet_centers(Rect::new(0.0, 0.0, 2.1, 1.0), 0.0, 0.3, [true, false, false, false], 100).unwrap();
    assert_eq!(c.len(), 8);
    assert!(grommet_centers(r, 0.0, 1.0, [true; 4], 10).is_none(), "over the limit");
    assert!(hem_lines(r, 0.0, [true; 4]).is_empty());
}
