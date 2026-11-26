//! Live Corners: `object.setLiveShape` rounds the given corners of a live rectangle, the
//! Direct-Selected ones or all four, sets corner kinds, and keeps selected corners selected; and
//! does the same on any path's corners: a star, a live polygon, a pen path (#511).

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_doc::{AnchorRef, LiveCorners, LiveShape, NodeKind};
use vectorcraft_geom::Affine;
use vectorcraft_geom::shapes::CornerKind;

use crate::*;

/// A 100 × 60 pt live rectangle at (10, 10), selected.
fn session() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 60})).unwrap()["id"].as_u64().unwrap();
    (s, NodeId(id))
}

fn run(s: &mut Session, p: Value) {
    s.execute("object.setLiveShape", &p).unwrap_or_else(|e| panic!("{p}: {e}"));
}

fn corners(s: &Session, id: NodeId) -> ([f64; 4], [CornerKind; 4], usize) {
    let n = s.doc().unwrap().doc.node(id).unwrap();
    let NodeKind::Path { live: Some(l), path, .. } = &n.kind else { panic!("not a live shape") };
    let LiveShape::Rectangle { radii, kinds, .. } = &**l else { panic!("not a live rectangle") };
    (*radii, *kinds, path.anchor_count())
}

fn selected_anchors(s: &Session, id: NodeId) -> Option<BTreeSet<AnchorRef>> {
    s.doc().unwrap().selection.partial(id).cloned()
}

fn anchors(v: &[usize]) -> BTreeSet<AnchorRef> {
    v.iter().map(|ai| (0, *ai)).collect()
}

#[test]
fn a_whole_rectangle_rounds_all_corners_and_given_corners_alone() {
    let (mut s, id) = session();
    run(&mut s, json!({"radius": 8}));
    assert_eq!(corners(&s, id), ([8.0; 4], [CornerKind::Round; 4], 8));
    run(&mut s, json!({"id": id.0, "radius": 0}));
    // One corner: only it rounds, and the path gains one anchor (one undo step).
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, json!({"ids": [id.0], "corners": [1], "radius": 12}));
    assert_eq!(corners(&s, id), ([0.0, 12.0, 0.0, 0.0], [CornerKind::Round; 4], 5));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    let b = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
    assert!((b.x1 - 110.0).abs() < 1e-9 && (b.y0 - 10.0).abs() < 1e-9, "the shape keeps its bounds: {b:?}");
    // Kinds alone, then both; the others stay as they were.
    run(&mut s, json!({"corners": [1, 3], "kind": "chamfer"}));
    run(&mut s, json!({"corners": [3], "radius": 5, "kind": "invertedRound"}));
    assert_eq!(corners(&s, id), ([0.0, 12.0, 0.0, 5.0], [CornerKind::Round, CornerKind::Chamfer, CornerKind::Round, CornerKind::InvertedRound], 6));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(corners(&s, id).1[3], CornerKind::Chamfer);
}

/// The Corner menus of Rectangle Properties: a kind for all four corners or one, set on corners
/// with no radius too and leaving the radii as they are, in one undo step; the kind the corners
/// already have adds no step.
#[test]
fn rectangle_kind_menu_targets_all_or_one_corner_and_skips_noop_undo() {
    let (mut s, id) = session();
    run(&mut s, json!({"corners": [1], "radius": 12}));
    let steps = |s: &Session| s.doc().unwrap().history.undo.len();
    let undo = steps(&s);
    run(&mut s, json!({"corners": [0, 1, 2, 3], "kind": "invertedRound"}));
    assert_eq!(corners(&s, id).0, [0.0, 12.0, 0.0, 0.0]);
    assert_eq!(corners(&s, id).1, [CornerKind::InvertedRound; 4]);
    assert_eq!(steps(&s), undo + 1);
    run(&mut s, json!({"corners": [0, 1, 2, 3], "kind": "invertedRound"}));
    run(&mut s, json!({"corners": [2], "kind": "invertedRound"}));
    assert_eq!(steps(&s), undo + 1, "no step when nothing changes");
    run(&mut s, json!({"corners": [2], "kind": "chamfer"}));
    assert_eq!(corners(&s, id).1, [CornerKind::InvertedRound, CornerKind::InvertedRound, CornerKind::Chamfer, CornerKind::InvertedRound]);
    assert_eq!(corners(&s, id).0, [0.0, 12.0, 0.0, 0.0]);
    assert_eq!(steps(&s), undo + 2);
}

/// A kind the corners already have leaves a rectangle saved stretched by an older version as it
/// is, alone or beside a rectangle the kind changes: its scale stays in its transform.
#[test]
fn a_kind_the_corners_have_leaves_a_stretched_rectangle() {
    let (mut s, id) = session();
    let other = NodeId(s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap());
    let stretched = LiveShape::Rectangle {
        w: 50.0,
        h: 60.0,
        radii: [10.0; 4],
        kinds: [CornerKind::Chamfer; 4],
        radii_relative: false,
        xf: Affine::translate((10.0, 10.0)) * Affine::scale_non_uniform(2.0, 1.0),
    };
    {
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        let NodeKind::Path { path, live: Some(live), .. } = &mut d.node_mut(id).unwrap().kind else { panic!("not live") };
        **live = stretched.clone();
        *path = live.to_path();
    }
    let live = |s: &Session| node_path(s, id).1;
    let steps = |s: &Session| s.doc().unwrap().history.undo.len();
    let undo = steps(&s);
    run(&mut s, json!({"ids": [id.0], "kind": "chamfer"}));
    assert_eq!(live(&s), Some(stretched.clone()));
    assert_eq!(steps(&s), undo, "no step when nothing changes");
    run(&mut s, json!({"ids": [id.0, other.0], "kind": "chamfer"}));
    assert_eq!(live(&s), Some(stretched));
    assert_eq!(corners(&s, other).1, [CornerKind::Chamfer; 4]);
    assert_eq!(steps(&s), undo + 1);
}

/// A radius is stored on a corner that stores a larger radius drawn at the new value, alone or
/// beside another rectangle; the radius every target already stores adds no undo step.
#[test]
fn a_radius_reaches_a_rectangle_that_already_draws_it() {
    let (mut s, id) = session();
    let other = NodeId(s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 100, "height": 60})).unwrap()["id"].as_u64().unwrap());
    // 80 pt on every corner draws at 30 pt, half the 60 pt side.
    run(&mut s, json!({"ids": [id.0], "radius": 80}));
    assert_eq!(corners(&s, id).0, [80.0; 4]);
    let (path, live) = node_path(&s, id);
    let drawn = LiveCorners::new(&path, live.as_ref()).unwrap().drawn_radius(0);
    assert!((drawn - 30.0).abs() < 1e-9, "draws at {drawn}");
    let steps = |s: &Session| s.doc().unwrap().history.undo.len();
    let undo = steps(&s);
    run(&mut s, json!({"ids": [id.0], "corners": [1], "radius": 30}));
    assert_eq!(corners(&s, id).0, [80.0, 30.0, 80.0, 80.0]);
    assert_eq!(steps(&s), undo + 1);
    run(&mut s, json!({"ids": [id.0, other.0], "corners": [0], "radius": 30}));
    assert_eq!(corners(&s, id).0[0], 30.0);
    assert_eq!(corners(&s, other).0[0], 30.0);
    // The radius both already store adds no step.
    run(&mut s, json!({"ids": [id.0, other.0], "corners": [0], "radius": 30}));
    assert_eq!(steps(&s), undo + 2);
}

/// `rectanglesOnly` sets the live rectangles among the selection and leaves a polygon as it is; an
/// action that records it holds no ids, so playing it sets the rectangles selected then.
#[test]
fn rectangles_only_sets_the_selected_rectangles_when_an_action_plays() {
    let (mut s, id) = session();
    let polygon = NodeId(s.execute("shape.polygon", &json!({"cx": 300, "cy": 300, "radius": 40, "sides": 6})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0, polygon.0]})).unwrap();
    let start = s.journal.len();
    run(&mut s, json!({"rectanglesOnly": true, "corners": [0], "kind": "chamfer"}));
    assert_eq!(corners(&s, id).1[0], CornerKind::Chamfer);
    let NodeKind::Path { live: Some(l), .. } = &s.doc().unwrap().doc.node(polygon).unwrap().kind else { panic!("not live") };
    let LiveShape::Polygon { kinds, .. } = &**l else { panic!("not a polygon") };
    assert!(kinds.iter().all(|k| *k == CornerKind::Round), "the polygon keeps its corners: {kinds:?}");
    let action = s.journal_for_action(start);
    assert_eq!(action.len(), 1);
    assert_eq!(action[0].1.get("ids"), None);
    let other = NodeId(s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [other.0, polygon.0]})).unwrap();
    s.replay(&action[0].0, &action[0].1).unwrap();
    assert_eq!(corners(&s, other).1[0], CornerKind::Chamfer);
}

/// #938: `items` rounds several objects in one step, each its own corners (or all of them).
#[test]
fn items_round_several_objects_each_its_own_corners() {
    let (mut s, a) = session();
    let b = NodeId(s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 100, "height": 60})).unwrap()["id"].as_u64().unwrap());
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, json!({"items": [{"id": a.0, "corners": [2]}, {"id": b.0}], "radius": 6}));
    assert_eq!(corners(&s, a).0, [0.0, 0.0, 6.0, 0.0]);
    assert_eq!(corners(&s, b).0, [6.0; 4]);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one step");
    for p in [
        json!({"items": 3, "radius": 1}),
        json!({"items": [{"corners": [1]}], "radius": 1}),
        json!({"items": [{"id": a.0, "corners": [9]}], "radius": 1}),
    ] {
        assert!(s.execute("object.setLiveShape", &p).is_err(), "{p}");
    }
}

#[test]
fn bad_corners_and_kinds_are_errors() {
    let (mut s, id) = session();
    for p in
        [json!({"corners": [4], "radius": 5}), json!({"corners": ["a"]}), json!({"corners": 1}), json!({"corners": [-1]}), json!({"kind": "wavy"})]
    {
        assert!(s.execute("object.setLiveShape", &p).is_err(), "{p}");
    }
    assert_eq!(corners(&s, id), ([0.0; 4], [CornerKind::Round; 4], 4), "nothing changed");
}

/// A side count that isn't a whole number from 3 to 1000, a negative radius and an unknown key
/// (at the top level or in one of `items`) are errors naming the key; the shapes and the undo
/// history stay as they were. A radius of 0 and the ends of the side range still work.
#[test]
fn bad_sides_radii_and_keys_are_errors() {
    let (mut s, id) = session();
    let poly = NodeId(s.execute("shape.polygon", &json!({"cx": 200, "cy": 200, "radius": 80, "sides": 6})).unwrap()["id"].as_u64().unwrap());
    let state = |s: &Session| {
        let st = s.doc().unwrap();
        (corners(s, id), st.doc.node(poly).unwrap().clone(), st.history.undo.len())
    };
    let before = state(&s);
    for (p, key) in [
        (json!({"ids": [poly.0], "sides": 2.5}), "`sides` must be a whole number from 3 to 1000, not 2.5"),
        (json!({"ids": [poly.0], "sides": -1}), "`sides` must be a whole number from 3 to 1000, not -1"),
        (json!({"ids": [poly.0], "sides": 2}), "`sides` must be a whole number from 3 to 1000, not 2"),
        (json!({"ids": [poly.0], "sides": 1001}), "`sides` must be a whole number from 3 to 1000, not 1001"),
        (json!({"ids": [id.0, poly.0], "radius": -4}), "`radius` must be a radius of 0 or more, not -4"),
        (json!({"items": [{"id": id.0, "corners": [0]}], "radius": -0.5}), "`radius` must be a radius of 0 or more, not -0.5"),
        (json!({"id": id.0, "path": [[0, 0]], "radius": 5}), "unknown parameter `path`"),
        (json!({"items": [{"id": id.0, "corner": [0]}], "radius": 5}), "unknown parameter `items[0].corner`; did you mean `items[0].corners`?"),
    ] {
        match s.execute("object.setLiveShape", &p) {
            Err(EngineError::BadParams { cmd, msg }) => assert!(cmd == "object.setLiveShape" && msg.starts_with(key), "{p}: {msg}"),
            other => panic!("{p}: {other:?}"),
        }
        assert_eq!(state(&s), before, "{p}");
    }
    run(&mut s, json!({"ids": [poly.0], "sides": 3}));
    assert!(matches!(node_path(&s, poly).1, Some(LiveShape::Polygon { sides: 3, .. })));
    run(&mut s, json!({"ids": [poly.0], "sides": 1000.0}));
    assert!(matches!(node_path(&s, poly).1, Some(LiveShape::Polygon { sides: 1000, .. })));
    run(&mut s, json!({"id": id.0, "radius": 6}));
    run(&mut s, json!({"id": id.0, "radius": 0}));
    assert_eq!(corners(&s, id).0, [0.0; 4]);
}

/// Saved steps play as they did before `sides` and `radius` were checked: a negative radius
/// removes the rounding, a whole side count outside 3 to 1000 is brought into that range, and a
/// fractional or negative side count is ignored.
#[test]
fn saved_steps_with_out_of_range_sides_or_radius_still_play() {
    let (mut s, id) = session();
    let poly = NodeId(s.execute("shape.polygon", &json!({"cx": 200, "cy": 200, "radius": 80, "sides": 6})).unwrap()["id"].as_u64().unwrap());
    let replay = |s: &mut Session, p: Value| s.replay("object.setLiveShape", &p).unwrap_or_else(|e| panic!("{p}: {e}"));
    run(&mut s, json!({"id": id.0, "radius": 6}));
    replay(&mut s, json!({"id": id.0, "radius": -4}));
    assert_eq!(corners(&s, id).0, [0.0; 4]);
    let sides = |s: &Session| match node_path(s, poly).1 {
        Some(LiveShape::Polygon { sides, .. }) => sides,
        other => panic!("{other:?}"),
    };
    for (n, want) in [(json!(2), 3), (json!(1001), 1000), (json!(2.5), 1000), (json!(-1), 1000), (json!(5.0), 5)] {
        replay(&mut s, json!({"ids": [poly.0], "sides": n}));
        assert_eq!(sides(&s), want, "{n}");
    }
    // A direct call still refuses them.
    assert!(s.execute("object.setLiveShape", &json!({"id": id.0, "radius": -4})).is_err());
    assert!(s.execute("object.setLiveShape", &json!({"ids": [poly.0], "sides": 2})).is_err());
}

#[test]
fn direct_selected_corners_round_alone_and_stay_selected() {
    let (mut s, id) = session();
    // Direct Selection picks the top-right and bottom-left corners.
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 1], [0, 3]], "mode": "set"})).unwrap();
    run(&mut s, json!({"radius": 10}));
    assert_eq!(corners(&s, id), ([0.0, 10.0, 0.0, 10.0], [CornerKind::Round; 4], 6));
    // Anchors: top-left, top-right's two, bottom-right, bottom-left's two.
    assert_eq!(selected_anchors(&s, id), Some(anchors(&[1, 2, 4, 5])));
    // The same corners again (a field in the Properties panel), now with their two anchors each.
    run(&mut s, json!({"kind": "chamfer"}));
    assert_eq!(corners(&s, id).1, [CornerKind::Round, CornerKind::Chamfer, CornerKind::Round, CornerKind::Chamfer]);
    run(&mut s, json!({"radius": 0}));
    assert_eq!(corners(&s, id).2, 4);
    assert_eq!(selected_anchors(&s, id), Some(anchors(&[1, 3])));
    // Undo brings back the selection with the shape.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(selected_anchors(&s, id), Some(anchors(&[1, 2, 4, 5])));
    // Explicit corners win over the selection, which follows its own corners.
    run(&mut s, json!({"corners": [0], "radius": 4}));
    assert_eq!(corners(&s, id).0, [4.0, 10.0, 0.0, 10.0]);
    assert_eq!(selected_anchors(&s, id), Some(anchors(&[1, 2, 4, 5])), "the top-left corner comes first and last");
}

#[test]
fn a_drag_previews_one_corner_and_commits_one_step() {
    let (mut s, id) = session();
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 2]], "mode": "set"})).unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    s.begin_interaction("Corner Radius").unwrap();
    for r in [5.0, 9.0, 14.0] {
        s.preview("object.setLiveShape", &json!({"id": id.0, "radius": r, "corners": [2]})).unwrap();
    }
    s.commit_interaction().unwrap();
    assert_eq!(corners(&s, id), ([0.0, 0.0, 14.0, 0.0], [CornerKind::Round; 4], 5));
    assert_eq!(selected_anchors(&s, id), Some(anchors(&[2, 3])));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
}

/// How far each corner's cut reaches along x and y (top-left, top-right, bottom-right,
/// bottom-left) of a rectangle with four cut corners: equal for a circular arc.
fn spans(s: &Session, id: NodeId) -> Vec<(f64, f64)> {
    let NodeKind::Path { path, .. } = &s.doc().unwrap().doc.node(id).unwrap().kind else { panic!("not a path") };
    let a: Vec<_> = path.subpaths[0].anchors.iter().map(|a| a.p).collect();
    assert_eq!(a.len(), 8, "four cut corners");
    [(7, 0), (1, 2), (3, 4), (5, 6)].iter().map(|(i, j)| ((a[*j].x - a[*i].x).abs(), (a[*j].y - a[*i].y).abs())).collect()
}

fn assert_spans(s: &Session, id: NodeId, want: [f64; 4]) {
    let got = spans(s, id);
    let ok = got.iter().zip(want).all(|((x, y), r)| (x - r).abs() < 1e-9 && (y - r).abs() < 1e-9);
    assert!(ok, "corners span {got:?}, want circles of radii {want:?}");
}

/// #442: on a rectangle that isn't square, every corner is the same circle, also past the limit
/// and on a shape from a file that kept an uneven scale in its transform (before #291).
#[test]
fn corners_of_a_non_square_rectangle_are_alike_circles() {
    let (mut s, id) = session();
    run(&mut s, json!({"radius": 12}));
    assert_spans(&s, id, [12.0; 4]);
    // Past half the shorter side, every corner stops there.
    run(&mut s, json!({"radius": 45}));
    assert_spans(&s, id, [30.0; 4]);
    // A 50 × 60 rectangle with 10 pt corners, saved stretched to 100 × 60: 20 × 10 ellipses.
    {
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        let NodeKind::Path { path, live: Some(live), .. } = &mut d.node_mut(id).unwrap().kind else { panic!("not live") };
        let xf = Affine::translate((10.0, 10.0)) * Affine::scale_non_uniform(2.0, 1.0);
        **live = LiveShape::Rectangle { w: 50.0, h: 60.0, radii: [10.0; 4], kinds: Default::default(), radii_relative: false, xf };
        *path = live.to_path();
    }
    assert_eq!(spans(&s, id)[0], (20.0, 10.0));
    // Rounding one corner makes every corner a circle: the others of the mean radius.
    run(&mut s, json!({"corners": [1], "radius": 12}));
    let mean = 10.0 * 2f64.sqrt();
    assert_spans(&s, id, [mean, 12.0, mean, mean]);
    let b = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
    assert!((b.x0 - 10.0).abs() < 1e-9 && (b.x1 - 110.0).abs() < 1e-9 && (b.y1 - 70.0).abs() < 1e-9, "same bounds: {b:?}");
    run(&mut s, json!({"radius": 8}));
    assert_spans(&s, id, [8.0; 4]);
}

/// A selected five-pointed star at (200, 200) of radii 60 and 30: a plain path.
fn star(s: &mut Session) -> NodeId {
    NodeId(s.execute("shape.star", &json!({"cx": 200, "cy": 200, "radius1": 60, "radius2": 30})).unwrap()["id"].as_u64().unwrap())
}

fn node_path(s: &Session, id: NodeId) -> (vectorcraft_geom::PathData, Option<LiveShape>) {
    let NodeKind::Path { path, live, .. } = &s.doc().unwrap().doc.node(id).unwrap().kind else { panic!("not a path") };
    (path.clone(), live.as_deref().cloned())
}

/// The radius and kind every corner of `id` shares, and how many corners it has.
fn style(s: &Session, id: NodeId) -> (Option<f64>, Option<CornerKind>, usize) {
    let c = LiveCorners::of(s.doc().unwrap().doc.node(id).unwrap()).unwrap();
    let (r, k) = c.style(&c.all());
    (r, k, c.corners.len())
}

/// The radius set on each of the first `N` corners of `id`.
fn radii<const N: usize>(s: &Session, id: NodeId) -> [f64; N] {
    let c = LiveCorners::of(s.doc().unwrap().doc.node(id).unwrap()).unwrap();
    std::array::from_fn(|k| c.radius(k))
}

/// #511: a star (a plain path) rounds all ten corners as one step, keeps them editable (a new
/// radius cuts the outline again rather than the cut path), and is the plain star again at 0.
#[test]
fn a_star_rounds_its_corners_and_keeps_them_live() {
    let (mut s, _) = session();
    let id = star(&mut s);
    let (sharp, _) = node_path(&s, id);
    assert_eq!(style(&s, id), (Some(0.0), Some(CornerKind::Round), 10));
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, json!({"radius": 4}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    let (path, live) = node_path(&s, id);
    assert_eq!(path.anchor_count(), 20);
    assert!(matches!(&live, Some(LiveShape::Path { base, .. }) if *base == sharp), "{live:?}");
    assert_eq!(style(&s, id), (Some(4.0), Some(CornerKind::Round), 10));
    assert_eq!(s.doc().unwrap().doc.node(id).unwrap().kind_label(), "Path");
    run(&mut s, json!({"radius": 6, "kind": "chamfer"}));
    assert_eq!((node_path(&s, id).0.anchor_count(), style(&s, id)), (20, (Some(6.0), Some(CornerKind::Chamfer), 10)));
    run(&mut s, json!({"radius": 0, "kind": "round"}));
    assert_eq!(node_path(&s, id), (sharp, None));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(style(&s, id).0, Some(6.0));
}

/// Each corner stops at its own limit: its cut reaches halfway along its shorter side, so a
/// star's neighbouring corners meet without overlapping.
#[test]
fn star_corners_stop_at_their_own_limit() {
    let (mut s, _) = session();
    let id = star(&mut s);
    run(&mut s, json!({"radius": 1000}));
    let (path, live) = node_path(&s, id);
    let Some(LiveShape::Path { base, .. }) = live else { panic!("live corners") };
    // Every cut ends at the middle of a side: each side's two cuts meet there.
    let sp = &base.subpaths[0];
    for (i, a) in sp.anchors.iter().enumerate() {
        let mid = a.p.midpoint(sp.anchors[(i + 1) % 10].p);
        assert!(path.anchors().any(|(_, _, b)| b.p.distance(mid) < 1e-9), "a cut ends at {mid:?}");
    }
    assert_eq!(style(&s, id).0, Some(1000.0), "the radius set is kept; it draws as large as fits");
}

/// #511: Direct Selection picks one of the star's anchors: that corner alone rounds, and both of
/// its new anchors stay selected.
#[test]
fn a_direct_selected_star_anchor_rounds_its_corner_alone() {
    let (mut s, _) = session();
    let id = star(&mut s);
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 0]], "mode": "set"})).unwrap();
    run(&mut s, json!({"radius": 5}));
    assert_eq!(node_path(&s, id).0.anchor_count(), 11);
    // The top tip's cut ends the path and starts it.
    assert_eq!(selected_anchors(&s, id), Some(anchors(&[0, 10])));
    assert_eq!(radii::<2>(&s, id), [5.0, 0.0]);
    // Explicit corners, as MCP and the widgets give them.
    run(&mut s, json!({"id": id.0, "corners": [1, 3], "radius": 2}));
    assert_eq!(radii::<4>(&s, id), [5.0, 2.0, 0.0, 2.0]);
    // Past the outline's last anchor is an error, and nothing changes.
    assert!(s.execute("object.setLiveShape", &json!({"id": id.0, "corners": [10], "radius": 1})).is_err());
    assert_eq!(node_path(&s, id).0.anchor_count(), 13);
}

/// A live polygon stays live with cut corners, which keep their radius as its sides change and
/// stay circular when it is scaled unevenly (the radius scales by the mean scale).
#[test]
fn a_polygon_keeps_its_corners_through_sides_and_scales() {
    let (mut s, _) = session();
    let id = NodeId(s.execute("shape.polygon", &json!({"cx": 200, "cy": 200, "radius": 80, "sides": 6})).unwrap()["id"].as_u64().unwrap());
    run(&mut s, json!({"radius": 10}));
    let (path, live) = node_path(&s, id);
    assert_eq!(path.anchor_count(), 12);
    assert!(matches!(live, Some(LiveShape::Polygon { sides: 6, .. })));
    run(&mut s, json!({"sides": 8}));
    assert_eq!((node_path(&s, id).0.anchor_count(), style(&s, id)), (16, (Some(10.0), Some(CornerKind::Round), 8)));
    s.execute("object.scale", &json!({"sx": 400, "sy": 100, "corners": true})).unwrap();
    assert!(matches!(node_path(&s, id).1, Some(LiveShape::Polygon { .. })));
    assert!((style(&s, id).0.unwrap() - 20.0).abs() < 1e-9);
    // Every cut is a circle of that radius: its ends are as far from its centre.
    let path = node_path(&s, id).0;
    let c = LiveCorners::of(s.doc().unwrap().doc.node(id).unwrap()).unwrap();
    for k in &c.corners {
        let centre = k.on_bisector(20.0 / k.sin());
        let ends = path.anchors().filter(|(_, _, a)| (a.p.distance(centre) - 20.0).abs() < 1e-6).count();
        assert_eq!(ends, 2, "corner {} is circular", k.index);
    }
}

/// #511: a pen path's corners round; its ends don't. An ellipse has no corners to round.
#[test]
fn a_pen_path_rounds_its_corners_but_not_its_ends() {
    let (mut s, _) = session();
    let pen = json!({"anchors": [{"x": 10, "y": 200}, {"x": 110, "y": 100}, {"x": 210, "y": 200}, {"x": 310, "y": 100}]});
    let id = NodeId(s.execute("path.create", &pen).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    run(&mut s, json!({"radius": 8}));
    let path = node_path(&s, id).0;
    assert_eq!(path.anchor_count(), 6);
    let sp = &path.subpaths[0];
    assert_eq!((sp.anchors[0].p.x, sp.anchors[5].p.x), (10.0, 310.0), "the ends stay");
    let e = NodeId(s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 50, "height": 30})).unwrap()["id"].as_u64().unwrap());
    let before = node_path(&s, e);
    run(&mut s, json!({"id": e.0, "radius": 8}));
    assert_eq!(node_path(&s, e), before);
}

/// #812: an ellipse's Pie Start and End Angle cut it as a pie; Invert Pie shows the other part;
/// one undo step each; a bad angle is an error.
#[test]
fn an_ellipse_takes_pie_angles_and_inverts() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    let id = NodeId(s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap());
    let pie = |s: &Session| match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Path { live: Some(l), path, .. } => match &**l {
            LiveShape::Ellipse { pie, .. } => (*pie, path.bounds().unwrap()),
            k => panic!("{k:?}"),
        },
        k => panic!("{k:?}"),
    };
    s.execute("object.setLiveShape", &json!({"pieStart": 0, "pieEnd": 90})).unwrap();
    let (p, b) = pie(&s);
    assert_eq!(p, (0.0, 90.0));
    assert!((b.x0 - 100.0).abs() < 1e-6 && (b.y1 - 50.0).abs() < 1e-6, "the quarter up and right of the centre: {b:?}");
    s.execute("object.setLiveShape", &json!({"invertPie": true})).unwrap();
    let (p, b) = pie(&s);
    assert_eq!(p, (90.0, 360.0));
    assert!(b.x0.abs() < 1e-6 && (b.y1 - 100.0).abs() < 1e-6, "the other three quarters: {b:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(pie(&s).0, (0.0, 90.0));
    // 0 to 360 (or an end of 0) is the whole ellipse again.
    s.execute("object.setLiveShape", &json!({"pieStart": 0, "pieEnd": 0})).unwrap();
    assert_eq!(pie(&s).0, (0.0, 360.0));
    assert_eq!(s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().anchor_count(), 4);
    assert!(s.execute("object.setLiveShape", &json!({"pieStart": "half"})).is_err());
}

/// Constrain Pie Angle (`constrainPie`): a new Pie Start or End Angle moves the other angle by as
/// much, so each selected ellipse keeps its own span, across 0° too; a whole ellipse stays whole;
/// after Invert Pie the inverted span is kept; one undo step for all of them; the angle each
/// already has records nothing; and `constrainPie` with both angles, with neither, with Invert Pie
/// or as text is an error that changes nothing.
#[test]
fn constrain_pie_keeps_each_ellipses_span() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    let id = |v: Value| NodeId(v["id"].as_u64().unwrap());
    let [a, b, c] = [0, 100, 200].map(|y| id(s.execute("shape.ellipse", &json!({"x": 0, "y": y, "width": 200, "height": 80})).unwrap()));
    run(&mut s, json!({"ids": [a.0], "pieStart": 300, "pieEnd": 60}));
    run(&mut s, json!({"ids": [b.0], "pieStart": 0, "pieEnd": 90}));
    let pies = |s: &Session| {
        [a, b, c].map(|i| match &s.doc().unwrap().doc.node(i).unwrap().kind {
            NodeKind::Path { live: Some(l), .. } => match &**l {
                LiveShape::Ellipse { pie, .. } => *pie,
                k => panic!("{k:?}"),
            },
            k => panic!("{k:?}"),
        })
    };
    let near = |p: (f64, f64), q: (f64, f64)| (p.0 - q.0).abs() < 1e-9 && (p.1 - q.1).abs() < 1e-9;
    let undo = |s: &Session| s.doc().unwrap().history.undo.len();
    let n = undo(&s);
    // 300° to 60° spans 120°, through 0°; 0° to 90° spans 90°; the third ellipse is whole.
    run(&mut s, json!({"ids": [a.0, b.0, c.0], "pieStart": 350, "constrainPie": true}));
    let [pa, pb, pc] = pies(&s);
    assert!(near(pa, (350.0, 110.0)) && near(pb, (350.0, 80.0)), "{pa:?} {pb:?}");
    assert!(near(pc, (350.0, 350.0)), "{pc:?}");
    assert_eq!(s.doc().unwrap().doc.node(c).unwrap().path_data().unwrap().anchor_count(), 4, "the whole ellipse stays whole");
    assert_eq!(undo(&s), n + 1, "one undo step");
    run(&mut s, json!({"ids": [a.0, b.0], "pieEnd": 20, "constrainPie": true}));
    let [pa, pb, _] = pies(&s);
    assert!(near(pa, (260.0, 20.0)) && near(pb, (290.0, 20.0)), "{pa:?} {pb:?}");
    let n = undo(&s);
    run(&mut s, json!({"ids": [a.0, b.0], "pieEnd": 380, "constrainPie": true}));
    assert_eq!(undo(&s), n, "the end each already has: no undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    let [pa, pb, _] = pies(&s);
    assert!(near(pa, (350.0, 110.0)) && near(pb, (350.0, 80.0)), "{pa:?} {pb:?}");
    // Inverted, 80° to 350° spans 270°, and a linked edit keeps that.
    run(&mut s, json!({"ids": [b.0], "invertPie": true}));
    run(&mut s, json!({"ids": [b.0], "pieStart": 0, "constrainPie": true}));
    assert!(near(pies(&s)[1], (0.0, 270.0)), "{:?}", pies(&s)[1]);
    let before = (undo(&s), pies(&s));
    for (p, want) in [
        (json!({"ids": [b.0], "pieStart": 10, "pieEnd": 20, "constrainPie": true}), "needs one of"),
        (json!({"ids": [b.0], "constrainPie": true}), "needs one of"),
        (json!({"ids": [b.0], "pieStart": 10, "constrainPie": true, "invertPie": true}), "needs one of"),
        (json!({"ids": [b.0], "pieStart": 10, "constrainPie": "yes"}), "`constrainPie` must be true or false"),
    ] {
        let r = s.execute("object.setLiveShape", &p);
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg.contains(want)), "{p}: {r:?}");
        assert_eq!((undo(&s), pies(&s)), before, "{p}");
    }
    // Off, each angle is set on its own, as before.
    run(&mut s, json!({"ids": [b.0], "pieStart": 90, "constrainPie": false}));
    assert!(near(pies(&s)[1], (90.0, 270.0)), "{:?}", pies(&s)[1]);
}

/// Invert Pie and Make Sides Equal given as text or as another value that is not true or false are
/// an error naming the key: the ellipse's pie, the polygon and the undo history stay as they were.
/// True makes the change, and false and null leave the shape as it is.
#[test]
fn invert_pie_and_make_sides_equal_refuse_a_boolean_given_as_text() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    let id = |v: Value| NodeId(v["id"].as_u64().unwrap());
    let ellipse = id(s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 200, "height": 100})).unwrap());
    run(&mut s, json!({"id": ellipse.0, "pieStart": 0, "pieEnd": 90}));
    let polygon = id(s.execute("shape.polygon", &json!({"cx": 200, "cy": 200, "radius": 50, "sides": 6})).unwrap());
    s.execute("object.scale", &json!({"sx": 200, "sy": 100})).unwrap();
    let shapes = |s: &Session| {
        let d = &s.doc().unwrap().doc;
        [ellipse, polygon].map(|i| match &d.node(i).unwrap().kind {
            NodeKind::Path { live: Some(l), .. } => (**l).clone(),
            k => panic!("{k:?}"),
        })
    };
    let undo = |s: &Session| s.doc().unwrap().history.undo.len();
    let before = (undo(&s), shapes(&s));
    for (key, p) in [
        ("invertPie", json!({"id": ellipse.0, "invertPie": "true"})),
        ("invertPie", json!({"id": ellipse.0, "pieEnd": 180, "invertPie": 1})),
        ("makeSidesEqual", json!({"id": polygon.0, "makeSidesEqual": "true"})),
        ("makeSidesEqual", json!({"id": polygon.0, "sides": 5, "makeSidesEqual": [true]})),
    ] {
        let r = s.execute("object.setLiveShape", &p);
        let want = format!("`{key}` must be true or false");
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == want), "{p}: {r:?}");
        assert_eq!((undo(&s), shapes(&s)), before, "{p}");
    }
    for p in [json!({"id": ellipse.0, "invertPie": false}), json!({"id": polygon.0, "makeSidesEqual": null})] {
        run(&mut s, p);
    }
    assert_eq!((undo(&s), shapes(&s)), before);
    run(&mut s, json!({"id": ellipse.0, "invertPie": true}));
    run(&mut s, json!({"id": polygon.0, "makeSidesEqual": true}));
    let [LiveShape::Ellipse { pie, .. }, l] = shapes(&s) else { panic!("an ellipse and a polygon") };
    assert_eq!(pie, (90.0, 360.0));
    assert!(l.polygon_sides_equal());
}

/// The Selection tool's pie widgets: dragging the End widget from 0° to straight up sets Pie End
/// Angle to 90 in one undo step; on an inverted pie a drag moves only the dragged angle; a
/// double-click on a widget makes the ellipse whole again in one step.
#[test]
fn dragging_and_double_clicking_the_pie_widgets() {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind};
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    let id = NodeId(s.execute("shape.ellipse", &json!({"x": 50, "y": 100, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap());
    let pie = |s: &Session| match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Path { live: Some(l), .. } => match &**l {
            LiveShape::Ellipse { pie, .. } => *pie,
            k => panic!("{k:?}"),
        },
        k => panic!("{k:?}"),
    };
    let near = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9;
    let steps = |s: &Session| s.doc().unwrap().history.undo.len();
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let gesture = |s: &mut Session, events: &[(PointerKind, f64, f64)]| {
        for (kind, x, y) in events {
            s.pointer(&PointerEvent::new(*kind, *x, *y).with_mods(Mods::default()), v).unwrap();
        }
    };
    // The center is (150, 150); both widgets of the whole ellipse sit 12 pt right of (250, 150).
    let n = steps(&s);
    gesture(&mut s, &[(PointerKind::Down, 262.0, 150.0), (PointerKind::Drag, 150.0, 20.0), (PointerKind::Up, 150.0, 20.0)]);
    assert!(near(pie(&s), (0.0, 90.0)), "{:?}", pie(&s));
    assert_eq!(steps(&s), n + 1);
    // Inverted, the pie runs from 90° to 360°: dragging the End widget (back at 0°) to 270°
    // leaves the start at 90°.
    s.execute("object.setLiveShape", &json!({"invertPie": true})).unwrap();
    assert!(near(pie(&s), (90.0, 360.0)));
    gesture(&mut s, &[(PointerKind::Down, 262.0, 150.0), (PointerKind::Drag, 150.0, 280.0), (PointerKind::Up, 150.0, 280.0)]);
    assert!(near(pie(&s), (90.0, 270.0)), "{:?}", pie(&s));
    // The Start widget sits 12 pt above the top of the outline, at 90°.
    let n = steps(&s);
    gesture(
        &mut s,
        &[(PointerKind::Down, 150.0, 88.0), (PointerKind::Up, 150.0, 88.0), (PointerKind::Down, 150.0, 88.0), (PointerKind::Up, 150.0, 88.0)],
    );
    assert_eq!(steps(&s), n, "two clicks record no undo step");
    gesture(&mut s, &[(PointerKind::DoubleClick, 150.0, 88.0)]);
    assert_eq!(pie(&s), (0.0, 360.0));
    assert_eq!(steps(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(pie(&s), (90.0, 270.0)));
}

/// #812: a polygon's Polygon Properties: its angle, radius and side length, each one undo step;
/// Make Sides Equal after an uneven scale; bad values are errors.
#[test]
fn a_polygon_takes_its_angle_radius_side_length_and_equal_sides() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = NodeId(s.execute("shape.polygon", &json!({"cx": 200, "cy": 200, "radius": 50, "sides": 6})).unwrap()["id"].as_u64().unwrap());
    let live = |s: &Session| match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Path { live: Some(l), path, .. } if matches!(**l, LiveShape::Polygon { .. }) => ((**l).clone(), path.bounds().unwrap()),
        k => panic!("{k:?}"),
    };
    let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
    run(&mut s, json!({"polygonAngle": 90}));
    let (l, b) = live(&s);
    assert!(close(l.polygon_angle().unwrap(), 90.0), "{l:?}");
    // Its first vertex, straight up before, points left now: the hexagon is 100 wide, 86.6 tall.
    assert!(close(b.width(), 100.0) && close(b.height(), 86.602_540_378) && close(b.center().x, 200.0), "{b:?}");
    run(&mut s, json!({"polygonRadius": 80}));
    assert!(close(live(&s).0.polygon_radius().unwrap(), 80.0));
    // A hexagon's side is as long as its radius; with 4 sides the radius is side / √2.
    run(&mut s, json!({"sideLength": 30}));
    assert!(close(live(&s).0.polygon_radius().unwrap(), 30.0));
    run(&mut s, json!({"sides": 4, "sideLength": 30}));
    assert!(close(live(&s).0.polygon_radius().unwrap(), 30.0 / 2f64.sqrt()));
    s.execute("edit.undo", &json!({})).unwrap();
    let LiveShape::Polygon { sides, .. } = live(&s).0 else { panic!("polygon") };
    assert_eq!(sides, 6, "one undo step");
    // Scaled unevenly its sides differ; Make Sides Equal makes them equal again, in place.
    s.execute("object.scale", &json!({"sx": 200, "sy": 100})).unwrap();
    let (l, b) = live(&s);
    assert!(!l.polygon_sides_equal() && l.polygon_angle().is_some());
    run(&mut s, json!({"makeSidesEqual": true}));
    let (l, after) = live(&s);
    assert!(l.polygon_sides_equal() && close(after.center().x, b.center().x) && close(after.center().y, b.center().y), "{b:?} → {after:?}");
    for bad in [json!({"polygonRadius": 0}), json!({"sideLength": -3}), json!({"polygonRadius": "big"}), json!({"polygonAngle": "left"})] {
        assert!(s.execute("object.setLiveShape", &bad).is_err(), "{bad}");
    }
}

/// Rectangle Angle and Ellipse Angle: `rectangleAngle` and `ellipseAngle` turn each selected live
/// rectangle or ellipse about its own center, keeping its size, corners and pie, as one undo step.
/// The angle it already has records no step, each key leaves the other kind as it is, and a value
/// that is not a number is an error.
#[test]
fn a_rectangle_and_an_ellipse_take_their_angle() {
    let (mut s, rect) = session();
    run(&mut s, json!({"radius": 8}));
    let ellipse = NodeId(s.execute("shape.ellipse", &json!({"x": 200, "y": 200, "width": 120, "height": 80})).unwrap()["id"].as_u64().unwrap());
    run(&mut s, json!({"pieStart": 30, "pieEnd": 120}));
    let live = |s: &Session, id: NodeId| {
        let n = s.doc().unwrap().doc.node(id).unwrap();
        (n.live_shape().unwrap().clone(), n.geometric_bounds().unwrap())
    };
    let steps = |s: &Session| s.doc().unwrap().history.undo.len();
    let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let angle = |s: &Session, id: NodeId| live(s, id).0.shape_angle().unwrap();
    // The ellipse's center: the middle of its untransformed box, wherever its pie leaves the path.
    let ellipse_center = |s: &Session| match live(s, ellipse).0 {
        LiveShape::Ellipse { w, h, xf, .. } => xf * vectorcraft_geom::Point::new(w / 2.0, h / 2.0),
        l => panic!("{l:?}"),
    };
    let center = ellipse_center(&s);

    s.execute("select.set", &json!({"ids": [rect.0]})).unwrap();
    let n = steps(&s);
    run(&mut s, json!({"rectangleAngle": 90}));
    let (l, b) = live(&s, rect);
    assert!(close(angle(&s, rect), 90.0), "{l:?}");
    // 100 × 60 at (10, 10) turned about (60, 40): 60 wide, 100 tall, same center.
    assert!(close(b.width(), 60.0) && close(b.height(), 100.0) && close(b.center().x, 60.0) && close(b.center().y, 40.0), "{b:?}");
    let LiveShape::Rectangle { w, h, radii, .. } = &l else { panic!("{l:?}") };
    assert!(close(*w, 100.0) && close(*h, 60.0) && *radii == [8.0; 4], "{w} {h} {radii:?}");
    assert_eq!(steps(&s), n + 1, "one undo step");
    for same in [json!({"rectangleAngle": 90}), json!({"rectangleAngle": 450}), json!({"rectangleAngle": -270}), json!({"ellipseAngle": 10})] {
        run(&mut s, same.clone());
        assert_eq!(steps(&s), n + 1, "{same} records no undo step");
    }
    // A turn the field gives back rounded is the angle the shape has: typing it again records no step.
    run(&mut s, json!({"rectangleAngle": 33.3}));
    let now = angle(&s, rect);
    run(&mut s, json!({"rectangleAngle": now}));
    run(&mut s, json!({"rectangleAngle": 33.3}));
    assert_eq!(steps(&s), n + 2);
    for bad in [json!({"rectangleAngle": "left"}), json!({"ellipseAngle": "up"}), json!({"rectangleAngle": true})] {
        assert!(s.execute("object.setLiveShape", &bad).is_err(), "{bad}");
        assert_eq!(steps(&s), n + 2, "{bad} changes nothing");
    }

    // The ellipse turns about its center and keeps its pie.
    s.execute("select.set", &json!({"ids": [ellipse.0]})).unwrap();
    run(&mut s, json!({"ellipseAngle": 90}));
    let (l, b) = live(&s, ellipse);
    assert!(close(angle(&s, ellipse), 90.0), "{l:?}");
    let LiveShape::Ellipse { pie, .. } = &l else { panic!("{l:?}") };
    assert_eq!(*pie, (30.0, 120.0));
    let now = ellipse_center(&s);
    assert!(close(now.x, center.x) && close(now.y, center.y), "{center:?} → {now:?}");
    assert!(b.width() < b.height(), "120 × 80 turned a quarter turn: {b:?}");

    // Both selected: each key turns its own kind only, each about its own center.
    s.execute("select.set", &json!({"ids": [rect.0, ellipse.0]})).unwrap();
    run(&mut s, json!({"rectangleAngle": 45}));
    assert!(close(angle(&s, rect), 45.0) && close(angle(&s, ellipse), 90.0));
    let b = live(&s, rect).1;
    assert!(close(b.center().x, 60.0) && close(b.center().y, 40.0), "{b:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(close(angle(&s, rect), 33.3));

    // A reflected rectangle takes the angle too, about its center.
    s.execute("select.set", &json!({"ids": [rect.0]})).unwrap();
    s.execute("object.reflect", &json!({"axis": "vertical"})).unwrap();
    let before = live(&s, rect).1.center();
    run(&mut s, json!({"rectangleAngle": 20}));
    let after = live(&s, rect).1.center();
    assert!(close(angle(&s, rect), 20.0) && close(before.x, after.x) && close(before.y, after.y), "{before:?} → {after:?}");
}

/// A shape named in `ids` that a turn would take past the coordinate limit (±4,000,000 pt) is an
/// error that changes nothing, whether or not it is selected.
#[test]
fn a_turn_past_the_coordinate_limit_is_an_error() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let mut add = |cmd: &str, p: Value| s.execute(cmd, &p).unwrap()["id"].as_u64().unwrap();
    // Each reaches within 10 pt of the limit and past it when turned.
    let rect = add("shape.rectangle", json!({"x": 3_999_890, "y": 100, "width": 100, "height": 100}));
    let ellipse = add("shape.ellipse", json!({"x": 3_999_950, "y": 100, "width": 40, "height": 100}));
    let polygon = add("shape.polygon", json!({"cx": 3_999_955, "cy": 200, "radius": 50, "sides": 6}));
    for (id, key, angle) in [(rect, "rectangleAngle", 45), (ellipse, "ellipseAngle", 90), (polygon, "polygonAngle", 90)] {
        for selected in [false, true] {
            s.execute("select.set", &json!({"ids": if selected { vec![id] } else { vec![] }})).unwrap();
            let (doc, steps) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
            let e = s.execute("object.setLiveShape", &json!({"ids": [id], key: angle})).unwrap_err();
            assert!(e.to_string().contains("out of range"), "{key}, selected {selected}: {e}");
            assert!(*s.doc().unwrap().doc == *doc && s.doc().unwrap().history.undo.len() == steps, "{key}, selected {selected}: changed");
        }
    }
}

/// A live line's Line Properties: Line Length keeps its first end in place, Line Angle (degrees
/// counterclockwise, any finite value brought into 0 to 360) turns it about its midpoint, the
/// length applies before the angle, every selected line takes them in one undo step, and a value
/// that changes nothing records none.
#[test]
fn a_line_takes_its_length_and_angle() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let line = |s: &mut Session, a: (f64, f64), b: (f64, f64)| {
        NodeId(s.execute("shape.line", &json!({"x1": a.0, "y1": a.1, "x2": b.0, "y2": b.1})).unwrap()["id"].as_u64().unwrap())
    };
    let id = line(&mut s, (100.0, 100.0), (200.0, 100.0));
    let ends = |s: &Session, id: NodeId| match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Path { live: Some(l), path, .. } => {
            let LiveShape::Line { a, b } = **l else { panic!("{l:?}") };
            assert_eq!(*path, l.to_path(), "the path follows the line");
            (a.x, a.y, b.x, b.y)
        }
        k => panic!("{k:?}"),
    };
    let near = |got: (f64, f64, f64, f64), want: (f64, f64, f64, f64)| {
        assert!([got.0 - want.0, got.1 - want.1, got.2 - want.2, got.3 - want.3].iter().all(|d| d.abs() < 1e-9), "{got:?} ≠ {want:?}")
    };
    let steps = |s: &Session| s.doc().unwrap().history.undo.len();
    run(&mut s, json!({"lineLength": 50}));
    near(ends(&s, id), (100.0, 100.0, 150.0, 100.0));
    // 90° points up the page.
    run(&mut s, json!({"lineAngle": 90}));
    near(ends(&s, id), (125.0, 125.0, 125.0, 75.0));
    let n = steps(&s);
    for same in [json!({"lineAngle": 450}), json!({"lineAngle": -270}), json!({"lineLength": 50}), json!({"lineLength": 50, "lineAngle": 90})] {
        run(&mut s, same.clone());
        assert_eq!(steps(&s), n, "{same} records no undo step");
    }
    near(ends(&s, id), (125.0, 125.0, 125.0, 75.0));
    run(&mut s, json!({"lineLength": 20, "lineAngle": 0}));
    near(ends(&s, id), (115.0, 115.0, 135.0, 115.0));
    assert_eq!(steps(&s), n + 1, "both in one step");
    run(&mut s, json!({"lineAngle": 180}));
    near(ends(&s, id), (135.0, 115.0, 115.0, 115.0));
    run(&mut s, json!({"lineAngle": -90}));
    near(ends(&s, id), (125.0, 105.0, 125.0, 125.0));
    let LiveShape::Line { .. } = s.doc().unwrap().doc.node(id).unwrap().live_shape().unwrap() else { panic!("still live") };
    assert_eq!(s.doc().unwrap().doc.node(id).unwrap().live_shape().unwrap().line_angle(), Some(270.0));
    // Several lines: each keeps its own first end, in one undo step.
    let other = line(&mut s, (0.0, 0.0), (0.0, 40.0));
    let n = steps(&s);
    run(&mut s, json!({"ids": [id.0, other.0], "lineLength": 10}));
    near(ends(&s, id), (125.0, 105.0, 125.0, 115.0));
    near(ends(&s, other), (0.0, 0.0, 0.0, 10.0));
    assert_eq!(steps(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    near(ends(&s, id), (125.0, 105.0, 125.0, 125.0));
    near(ends(&s, other), (0.0, 0.0, 0.0, 40.0));
    // Bad values, and a line that would reach past the coordinate limit, change neither line.
    let far = line(&mut s, (3_900_000.0, 0.0), (3_900_100.0, 0.0));
    let n = steps(&s);
    for bad in [
        json!({"ids": [id.0], "lineLength": 0}),
        json!({"ids": [id.0], "lineLength": -3}),
        json!({"ids": [id.0], "lineLength": "long"}),
        json!({"ids": [id.0], "lineAngle": "up"}),
        json!({"ids": [id.0], "lineLength": 1e300}),
        json!({"ids": [id.0, far.0], "lineLength": 200_000}),
    ] {
        assert!(s.execute("object.setLiveShape", &bad).is_err(), "{bad}");
        assert_eq!(steps(&s), n, "{bad}");
    }
    near(ends(&s, id), (125.0, 105.0, 125.0, 125.0));
    near(ends(&s, far), (3_900_000.0, 0.0, 3_900_100.0, 0.0));
    // A length past the coordinate limit is fine while both ends stay inside it.
    let wide = line(&mut s, (-3_500_000.0, 0.0), (3_500_000.0, 0.0));
    run(&mut s, json!({"ids": [wide.0], "lineLength": 6_000_000}));
    near(ends(&s, wide), (-3_500_000.0, 0.0, 2_500_000.0, 0.0));
    // A line whose ends meet reads 0°: an angle leaves it as it is, and a length points it right.
    let dot = line(&mut s, (10.0, 10.0), (10.0, 10.0));
    assert_eq!(s.doc().unwrap().doc.node(dot).unwrap().live_shape().unwrap().line_angle(), Some(0.0));
    let n = steps(&s);
    run(&mut s, json!({"lineAngle": 45}));
    assert_eq!(steps(&s), n);
    run(&mut s, json!({"lineLength": 30}));
    near(ends(&s, dot), (10.0, 10.0, 40.0, 10.0));
    // A very short length is applied.
    let speck = line(&mut s, (20.0, 20.0), (20.0, 20.0));
    let n = steps(&s);
    run(&mut s, json!({"ids": [speck.0], "lineLength": 1e-12}));
    assert_eq!(steps(&s), n + 1, "a length of 1e-12 pt is a change");
    assert!(ends(&s, speck).2 > 20.0, "{:?}", ends(&s, speck));
    // A length so short that the rounded ends do not move records no undo step.
    let dust = line(&mut s, (20.0, 20.0), (20.0, 20.0));
    let n = steps(&s);
    run(&mut s, json!({"ids": [dust.0], "lineLength": 1e-20}));
    assert_eq!(steps(&s), n, "a length of 1e-20 pt changes nothing");
    near(ends(&s, dust), (20.0, 20.0, 20.0, 20.0));
    // Other live shapes ignore the line params.
    let rect = NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
    let n = steps(&s);
    run(&mut s, json!({"ids": [rect.0], "lineLength": 30, "lineAngle": 45}));
    assert_eq!(steps(&s), n);
}

#[test]
fn relative_corner_rounding_keeps_fraction_on_resize() {
    let (mut s, id) = session();
    run(&mut s, json!({"relativeRadii": true}));
    let n = s.doc().unwrap().doc.node(id).unwrap();
    let c = LiveCorners::of(n).unwrap();
    let max = c.corner(0).unwrap().max_radius();
    let want = 0.25;
    run(&mut s, json!({"radius": want * max}));
    let n = s.doc().unwrap().doc.node(id).unwrap();
    let LiveShape::Rectangle { radii, radii_relative, .. } = n.live_shape().unwrap() else { panic!() };
    assert!(radii_relative);
    assert!((radii[0] - want).abs() < 0.02, "stored fraction: {}", radii[0]);
    let drawn = LiveCorners::of(n).unwrap().drawn_radius(0);
    s.execute("object.scale", &json!({"sx": 200, "sy": 200, "corners": true})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap();
    let LiveShape::Rectangle { radii, radii_relative, .. } = n.live_shape().unwrap() else { panic!() };
    assert!(radii_relative);
    assert!((radii[0] - want).abs() < 0.02, "fraction unchanged after scale: {}", radii[0]);
    let c = LiveCorners::of(n).unwrap();
    assert!((c.drawn_radius(0) / drawn - 2.0).abs() < 0.05, "drawn radius doubled with the shape");
}

/// `relativeRadii` on a plain path or an ellipse, which have no Live Corners record, leaves them
/// as they are and adds no undo step.
#[test]
fn relative_rounding_without_live_corners_adds_no_undo_step() {
    let (mut s, id) = session();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.expandShape", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(id).unwrap().live_shape().is_none(), "a plain path");
    let steps = |s: &Session| s.doc().unwrap().history.undo.len();
    let n = steps(&s);
    run(&mut s, json!({"ids": [id.0], "relativeRadii": false}));
    run(&mut s, json!({"ids": [id.0], "relativeRadii": true}));
    assert_eq!(steps(&s), n, "a plain path: no change");
    assert!(s.doc().unwrap().doc.node(id).unwrap().live_shape().is_none(), "still a plain path");
    let e = s.execute("shape.ellipse", &json!({"x": 200, "y": 200, "width": 50, "height": 30})).unwrap()["id"].as_u64().unwrap();
    let n = steps(&s);
    run(&mut s, json!({"ids": [e], "relativeRadii": true}));
    assert_eq!(steps(&s), n, "an ellipse: no change");
}

/// With Relative rounding, a radius is compared as the fraction it would store: a radius past the
/// corner's maximum, typed again, and the maximum the field then shows add no undo step, while a
/// smaller one adds one.
#[test]
fn a_relative_radius_that_stores_the_same_fraction_adds_no_undo_step() {
    let (mut s, id) = session();
    run(&mut s, json!({"relativeRadii": true}));
    let steps = |s: &Session| s.doc().unwrap().history.undo.len();
    let max = LiveCorners::of(s.doc().unwrap().doc.node(id).unwrap()).unwrap().corner(0).unwrap().max_radius();
    let n = steps(&s);
    run(&mut s, json!({"corners": [0], "radius": max + 20.0}));
    assert_eq!(steps(&s), n + 1, "past the maximum: one undo step");
    let c = LiveCorners::of(s.doc().unwrap().doc.node(id).unwrap()).unwrap();
    assert!((c.radius(0) - 1.0).abs() < 1e-9, "stored as the whole maximum: {}", c.radius(0));
    run(&mut s, json!({"corners": [0], "radius": max + 20.0}));
    assert_eq!(steps(&s), n + 1, "the same radius again: no undo step");
    run(&mut s, json!({"corners": [0], "radius": max}));
    assert_eq!(steps(&s), n + 1, "the maximum, as shown: no undo step");
    run(&mut s, json!({"corners": [0], "radius": max / 2.0}));
    assert_eq!(steps(&s), n + 2, "a smaller radius: one undo step");
    // On a large shape a small radius is a fraction below 1e-9, and still a change.
    let big = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 3_000_000, "height": 3_000_000})).unwrap()["id"].as_u64().unwrap();
    run(&mut s, json!({"ids": [big], "relativeRadii": true}));
    let n = steps(&s);
    run(&mut s, json!({"ids": [big], "corners": [0], "radius": 0.001}));
    assert_eq!(steps(&s), n + 1, "a small radius on a large shape: one undo step");
}

/// #2898: while a corner widget is pressed or dragged, Up and Down step the corners' kind (round,
/// inverted round, chamfer, in turn) along with the radius, as one undo step with the drag.
#[test]
fn arrows_while_dragging_a_corner_widget_change_the_corner_kind() {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};
    let (mut s, id) = session();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let st = s.doc().unwrap();
    let w = vectorcraft_tools::corners::CornerWidgets::showing(&st.doc, &st.selection, v.zoom, false, true, 180.0).expect("corner widgets show");
    let at = w.visible().next().expect("a widget");
    let steps = |s: &Session| s.doc().unwrap().history.undo.len();
    let ptr = |s: &mut Session, kind: PointerKind, x: f64, y: f64| {
        s.pointer(&PointerEvent::new(kind, x, y).with_mods(Mods::default()), v).unwrap();
    };
    let n = steps(&s);
    // Press, Up twice (round → inverted round → chamfer), drag inward, release.
    ptr(&mut s, PointerKind::Down, at.x, at.y);
    s.tool_key(ToolKey::Up, Mods::default(), v).unwrap();
    s.tool_key(ToolKey::Up, Mods::default(), v).unwrap();
    ptr(&mut s, PointerKind::Drag, at.x + 8.0, at.y + 8.0);
    ptr(&mut s, PointerKind::Up, at.x + 8.0, at.y + 8.0);
    let (radii, kinds, _) = corners(&s, id);
    assert_eq!(kinds, [CornerKind::Chamfer; 4], "every shown corner takes the kind");
    assert!(radii.iter().all(|r| *r > 0.0), "and the radius: {radii:?}");
    assert_eq!(steps(&s), n + 1, "one undo step");
    // A press and Down without a drag: the previous kind, the radius kept, one more step. A
    // pointer move within the drag threshold after the press leaves the radii alone too.
    let st = s.doc().unwrap();
    let w = vectorcraft_tools::corners::CornerWidgets::showing(&st.doc, &st.selection, v.zoom, false, true, 180.0).unwrap();
    let at = w.visible().next().unwrap();
    ptr(&mut s, PointerKind::Down, at.x, at.y);
    s.tool_key(ToolKey::Down, Mods::default(), v).unwrap();
    ptr(&mut s, PointerKind::Drag, at.x + 1.0, at.y + 1.0);
    ptr(&mut s, PointerKind::Up, at.x + 1.0, at.y + 1.0);
    let (after, kinds, _) = corners(&s, id);
    assert_eq!(kinds, [CornerKind::InvertedRound; 4]);
    assert_eq!(after, radii);
    assert_eq!(steps(&s), n + 2);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(corners(&s, id).1, [CornerKind::Chamfer; 4]);
}

/// An arrow press without a drag changes only the corners' kind: corners with different radii
/// keep each their own (#2898 review).
#[test]
fn an_arrow_press_without_a_drag_keeps_mixed_radii() {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};
    let (mut s, id) = session();
    run(&mut s, json!({"id": id.0, "corners": [0], "radius": 5}));
    run(&mut s, json!({"id": id.0, "corners": [2], "radius": 15}));
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let before = corners(&s, id).0;
    let st = s.doc().unwrap();
    let w = vectorcraft_tools::corners::CornerWidgets::showing(&st.doc, &st.selection, v.zoom, false, true, 180.0).unwrap();
    let at = w.visible().next().unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, at.x, at.y).with_mods(Mods::default()), v).unwrap();
    s.tool_key(ToolKey::Up, Mods::default(), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, at.x, at.y).with_mods(Mods::default()), v).unwrap();
    let (radii, kinds, _) = corners(&s, id);
    assert_eq!(radii, before, "each corner keeps its radius");
    assert!(kinds.iter().all(|k| *k == CornerKind::InvertedRound), "{kinds:?}");
}

/// A double-click on a corner widget with the Rectangle tool asks the frontend for the Transform
/// panel, and leaves the document, its undo history, the selection and the tool as they were.
#[test]
fn a_double_click_on_a_corner_widget_with_a_shape_tool_asks_for_the_transform_panel() {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind};
    let (mut s, id) = session();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("rectangle", v).unwrap();
    let st = s.doc().unwrap();
    let w = vectorcraft_tools::corners::CornerWidgets::showing(&st.doc, &st.selection, v.zoom, false, true, 180.0).expect("corner widgets show");
    let at = w.visible().next().expect("a widget");
    let (doc, selection, steps) = (st.doc.clone(), st.selection.clone(), st.history.undo.len());
    let mut asked = vec![];
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick] {
        asked.extend(s.pointer(&PointerEvent::new(kind, at.x, at.y).with_mods(Mods::default()), v).unwrap());
    }
    assert_eq!(asked, vec![UiRequest::Panel("transform".into())]);
    let st = s.doc().unwrap();
    assert!(*st.doc == *doc, "the document is unchanged");
    assert_eq!(st.selection, selection);
    assert_eq!(st.history.undo.len(), steps, "no undo step");
    assert!(st.interaction.is_none());
    assert_eq!(s.tool_id(), "rectangle");
}

/// A double-click on a corner widget of a selected live rectangle with the Selection tool asks
/// the frontend for the Transform panel, and leaves the document, its undo history, the
/// selection and the tool as they were; with Direct Selection it still opens Corners.
#[test]
fn a_double_click_on_a_corner_widget_with_the_selection_tool_asks_for_the_transform_panel() {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind};
    let (mut s, id) = session();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let v = ViewInfo::default();
    let st = s.doc().unwrap();
    let w = vectorcraft_tools::corners::CornerWidgets::showing(&st.doc, &st.selection, v.zoom, false, true, 180.0).expect("corner widgets show");
    let at = w.visible().next().expect("a widget");
    let (doc, selection, steps) = (st.doc.clone(), st.selection.clone(), st.history.undo.len());
    let double_click = |s: &mut Session, tool: &str| {
        s.select_tool(tool, v).unwrap();
        let mut asked = vec![];
        for kind in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick] {
            asked.extend(s.pointer(&PointerEvent::new(kind, at.x, at.y).with_mods(Mods::default()), v).unwrap());
        }
        asked
    };
    assert_eq!(double_click(&mut s, "selection"), vec![UiRequest::Panel("transform".into())]);
    let st = s.doc().unwrap();
    assert!(*st.doc == *doc, "the document is unchanged");
    assert_eq!(st.selection, selection);
    assert_eq!(st.history.undo.len(), steps, "no undo step");
    assert!(st.interaction.is_none());
    assert_eq!(s.tool_id(), "selection");
    // Direct Selection opens Corners for the corner it picked with the first click.
    let asked = double_click(&mut s, "directSelection");
    assert!(matches!(asked.as_slice(), [UiRequest::Dialog(k, _)] if k == vectorcraft_tools::corners::DIALOG), "{asked:?}");
}
