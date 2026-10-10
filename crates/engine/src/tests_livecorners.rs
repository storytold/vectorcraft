//! Live Corners: `object.setLiveShape` rounds the given corners of a live rectangle, the
//! Direct-Selected ones or all four, sets corner kinds, and keeps selected corners selected; and
//! does the same on any path's corners: a star, a live polygon, a pen path (#511).

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_doc::{AnchorRef, LiveCorners, LiveShape, NodeKind};
use vectorcraft_geom::Affine;
use vectorcraft_geom::shapes::CornerKind;

use super::*;

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
    let NodeKind::Path { live: Some(LiveShape::Rectangle { radii, kinds, .. }), path, .. } = &n.kind else { panic!("not a live rectangle") };
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
        *live = LiveShape::Rectangle { w: 50.0, h: 60.0, radii: [10.0; 4], kinds: Default::default(), xf };
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
    (path.clone(), live.clone())
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
        NodeKind::Path { live: Some(LiveShape::Ellipse { pie, .. }), path, .. } => (*pie, path.bounds().unwrap()),
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

/// #812: a polygon's Polygon Properties: its angle, radius and side length, each one undo step;
/// Make Sides Equal after an uneven scale; bad values are errors.
#[test]
fn a_polygon_takes_its_angle_radius_side_length_and_equal_sides() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = NodeId(s.execute("shape.polygon", &json!({"cx": 200, "cy": 200, "radius": 50, "sides": 6})).unwrap()["id"].as_u64().unwrap());
    let live = |s: &Session| match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Path { live: Some(l @ LiveShape::Polygon { .. }), path, .. } => (l.clone(), path.bounds().unwrap()),
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
