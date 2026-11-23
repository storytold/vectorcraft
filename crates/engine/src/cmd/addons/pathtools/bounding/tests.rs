//! Add Bounding Rectangle and Add Bounding Circle: the box and the smallest circle, the current
//! paint and placement on top of the current layer or the isolated group, `each` and `skipped`,
//! shapes of no size, clip groups and clipping sublayers, paths of compound paths, huge curves and
//! small loops, and the errors that leave the document as it was.

use serde_json::{Value, json};
use vectorcraft_doc::{LiveShape, Node, NodeId, NodeKind};
use vectorcraft_geom::{CubicBez, Point, Rect, Vec2, shapes};

use super::{CIRCLE, COMPACT_AT, Gathered, RECTANGLE, Splits};
use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn ids_of(v: &Value) -> Vec<NodeId> {
    v["ids"].as_array().unwrap().iter().map(|i| NodeId(i.as_u64().unwrap())).collect()
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn object_count(s: &Session) -> usize {
    let mut n = 0;
    s.doc().unwrap().doc.walk(|_| n += 1);
    n
}

/// Whether two rectangles agree to within `tol` on every edge.
fn rect_near(b: Rect, want: Rect, tol: f64) -> bool {
    [(b.x0, want.x0), (b.y0, want.y0), (b.x1, want.x1), (b.y1, want.y1)].iter().all(|(a, w)| (a - w).abs() < tol)
}

/// The live shape of `id`.
fn live(s: &Session, id: NodeId) -> LiveShape {
    match &node(s, id).kind {
        NodeKind::Path { live: Some(live), .. } => live.as_ref().clone(),
        other => panic!("not a live shape: {other:?}"),
    }
}

/// The geometric bounds of `id`.
fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.bounds_of(&[id], false).unwrap()
}

/// The last child of `parent`, the top of its stacking order.
fn top_of(s: &Session, parent: NodeId) -> NodeId {
    node(s, parent).children().unwrap().last().unwrap().id
}

/// Runs `command` with `p` and checks that it fails and leaves the document as it was.
fn refused(s: &mut Session, command: &str, p: Value) -> String {
    let (undo, objects) = (undo_len(s), object_count(s));
    let e = s.execute(command, &p).unwrap_err().to_string();
    assert_eq!((undo_len(s), object_count(s)), (undo, objects), "{p} changed the document");
    e
}

/// A live rectangle on the selection's geometric bounding box, strokes left out, with the current
/// paint, on top of the current layer, selected; one undo removes it.
#[test]
fn a_bounding_rectangle_is_the_geometric_box_on_top_of_the_layer() {
    let mut s = session();
    s.execute("stroke.set", &json!({"weight": 10})).unwrap();
    let a = rect(&mut s, 10.0, 20.0, 30.0, 40.0);
    let b = id_of(&s.execute("shape.ellipse", &json!({"x": 100, "y": 50, "width": 60, "height": 80})).unwrap());
    assert_eq!(node(&s, a).appearance.stroke_width(), 10.0, "the strokes reach past the geometric box");
    s.execute("select.none", &json!({})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let (before, undo) = (object_count(&s), undo_len(&s));
    let out = s.execute(RECTANGLE, &json!({})).unwrap();
    let id = id_of(&out);
    assert_eq!(ids_of(&out), vec![id]);
    assert!(out.get("skipped").is_none(), "{out}");
    assert!(matches!(live(&s, id), LiveShape::Rectangle { .. }));
    assert!(rect_near(bounds(&s, id), Rect::new(10.0, 20.0, 160.0, 130.0), 1e-9), "{:?}", bounds(&s, id));
    let layer = s.doc().unwrap().current_layer().unwrap();
    assert_eq!(top_of(&s, layer), id);
    assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
    assert_eq!(undo_len(&s), undo + 1);
    assert_eq!(object_count(&s), before + 1);
    // The current paint, as a shape drawn now gets it.
    let reference = rect(&mut s, 300.0, 300.0, 5.0, 5.0);
    let (n, r) = (node(&s, id), node(&s, reference));
    assert_eq!((n.appearance.fill_paint(), n.appearance.stroke_paint()), (r.appearance.fill_paint(), r.appearance.stroke_paint()));
    assert_eq!(n.appearance.stroke_width(), r.appearance.stroke_width());
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(object_count(&s), before);
}

/// A bounding circle around a right triangle stands on its hypotenuse; around a circle it is that
/// circle, grown by at most the flattening allowance; `offset` grows the radius, and a rectangle's
/// sides, by that much.
#[test]
fn a_bounding_circle_is_the_smallest_circle_around_the_outlines() {
    let mut s = session();
    let tri =
        id_of(&s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 0}, {"x": 80, "y": 0}, {"x": 0, "y": 60}], "closed": true})).unwrap());
    s.execute("select.set", &json!({"ids": [tri.0]})).unwrap();
    let c = id_of(&s.execute(CIRCLE, &json!({})).unwrap());
    match live(&s, c) {
        LiveShape::Ellipse { w, h, pie, .. } => {
            assert!((w - 100.02).abs() < 1e-9 && (h - 100.02).abs() < 1e-9 && pie == (0.0, 360.0), "{w} {h}");
        }
        other => panic!("not a circle: {other:?}"),
    }
    assert!(rect_near(bounds(&s, c), Rect::new(-10.01, -20.01, 90.01, 80.01), 1e-9), "{:?}", bounds(&s, c));

    // A circle of radius 50 and a dot inside it: the bounding circle is the circle itself.
    let round = id_of(&s.execute("shape.ellipse", &json!({"x": 200, "y": 200, "width": 100, "height": 100})).unwrap());
    let dot = rect(&mut s, 240.0, 240.0, 2.0, 2.0);
    s.execute("select.set", &json!({"ids": [round.0, dot.0]})).unwrap();
    let c = id_of(&s.execute(CIRCLE, &json!({"offset": 5})).unwrap());
    let got = bounds(&s, c);
    let want = Rect::new(195.0, 195.0, 305.0, 305.0);
    assert!(rect_near(got, want, 0.03), "{got:?}");
    assert!(got.x0 <= want.x0 + 1e-9 && got.x1 >= want.x1 - 1e-9 && got.y0 <= want.y0 + 1e-9 && got.y1 >= want.y1 - 1e-9, "{got:?}");

    s.execute("select.set", &json!({"ids": [tri.0]})).unwrap();
    let r = id_of(&s.execute(RECTANGLE, &json!({"offset": 4})).unwrap());
    assert!(rect_near(bounds(&s, r), Rect::new(-4.0, -4.0, 84.0, 64.0), 1e-9));
}

/// A shape with no size is not drawn: a straight horizontal line gets no rectangle and a path of
/// one anchor no circle, unless `offset` gives them room; with `each` they are counted in
/// `skipped`, and alone they are an error.
#[test]
fn shapes_of_no_size_are_skipped_unless_offset_gives_them_room() {
    let mut s = session();
    let line = id_of(&s.execute("path.create", &json!({"anchors": [{"x": 10, "y": 50}, {"x": 90, "y": 50}]})).unwrap());
    let lone = id_of(&s.execute("path.create", &json!({"anchors": [{"x": 50, "y": 70}]})).unwrap());
    let box_ = rect(&mut s, 200.0, 200.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [line.0]})).unwrap();
    assert!(refused(&mut s, RECTANGLE, json!({})).contains("the shape would have no size"));
    let r = id_of(&s.execute(RECTANGLE, &json!({"offset": 2})).unwrap());
    assert!(rect_near(bounds(&s, r), Rect::new(8.0, 48.0, 92.0, 52.0), 1e-9), "{:?}", bounds(&s, r));
    // The line has length, so its circle has size.
    s.execute("select.set", &json!({"ids": [line.0]})).unwrap();
    let c = id_of(&s.execute(CIRCLE, &json!({})).unwrap());
    assert!(rect_near(bounds(&s, c), Rect::new(9.99, 9.99, 90.01, 90.01), 1e-9), "{:?}", bounds(&s, c));

    s.execute("select.set", &json!({"ids": [lone.0]})).unwrap();
    assert!(refused(&mut s, CIRCLE, json!({})).contains("the shape would have no size"));
    let c = id_of(&s.execute(CIRCLE, &json!({"offset": 3})).unwrap());
    assert!(rect_near(bounds(&s, c), Rect::new(46.99, 66.99, 53.01, 73.01), 1e-9), "{:?}", bounds(&s, c));

    s.execute("select.set", &json!({"ids": [line.0, lone.0, box_.0]})).unwrap();
    let out = s.execute(RECTANGLE, &json!({"each": true})).unwrap();
    assert_eq!((ids_of(&out).len(), out["skipped"].as_u64()), (1, Some(2)), "{out}");
    assert!(rect_near(bounds(&s, id_of(&out)), Rect::new(200.0, 200.0, 210.0, 210.0), 1e-9));
    s.execute("select.set", &json!({"ids": [line.0, lone.0, box_.0]})).unwrap();
    let out = s.execute(CIRCLE, &json!({"each": true})).unwrap();
    assert_eq!((ids_of(&out).len(), out["skipped"].as_u64()), (2, Some(1)), "{out}");
}

/// With `each`, every selected object gets a shape of its own, all in one undo step, on top of the
/// layer in the selection's stacking order, and the new shapes are selected.
#[test]
fn each_draws_one_shape_around_each_object() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 100.0, 0.0, 20.0, 30.0);
    s.execute("select.set", &json!({"ids": [b.0, a.0]})).unwrap();
    let undo = undo_len(&s);
    let out = s.execute(RECTANGLE, &json!({"each": true})).unwrap();
    let ids = ids_of(&out);
    assert_eq!(ids.len(), 2);
    assert!(rect_near(bounds(&s, ids[0]), Rect::new(0.0, 0.0, 10.0, 10.0), 1e-9));
    assert!(rect_near(bounds(&s, ids[1]), Rect::new(100.0, 0.0, 120.0, 30.0), 1e-9));
    let layer = s.doc().unwrap().current_layer().unwrap();
    let order: Vec<NodeId> = node(&s, layer).children().unwrap().iter().map(|n| n.id).collect();
    assert_eq!(order, vec![a, b, ids[0], ids[1]]);
    assert_eq!(s.doc().unwrap().selection.objects.len(), 2);
    assert_eq!(undo_len(&s), undo + 1);
}

/// An object with no outline (an empty group) gets no shape and the result counts it; with only
/// such objects selected the command draws nothing.
#[test]
fn each_counts_the_objects_without_an_outline() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let empty = s
        .edit("Group", |d, _| {
            let id = d.alloc_id();
            d.insert(d.default_layer(), 0, Node::group(id, vec![]))?;
            Ok(id)
        })
        .unwrap();
    s.execute("select.set", &json!({"ids": [a.0, empty.0]})).unwrap();
    let out = s.execute(CIRCLE, &json!({"each": true})).unwrap();
    assert_eq!((ids_of(&out).len(), out["skipped"].as_u64()), (1, Some(1)), "{out}");
    s.execute("select.set", &json!({"ids": [a.0, empty.0]})).unwrap();
    let out = s.execute(CIRCLE, &json!({})).unwrap();
    assert!(out.get("skipped").is_none(), "{out}");
    s.execute("select.set", &json!({"ids": [empty.0]})).unwrap();
    assert!(refused(&mut s, RECTANGLE, json!({"each": true})).contains("nothing to enclose"));
}

/// Curves over a hundred thousand points across are held by their circle: every point along the
/// curves is inside the circle, whose radius is the 0.01 pt allowance larger than the circle around
/// the flattened points. The farthest point of the lopsided arc lies between two flattened points,
/// about 0.001 pt outside the circle around them, so it needs that allowance.
#[test]
fn the_circle_holds_huge_curves() {
    let mut s = session();
    let d = "M 0 0 C 0 -90000 120000 -90000 120000 0 C 60000 40000 30000 70000 0 0 Z";
    let blob = id_of(&s.execute("path.create", &json!({"d": d})).unwrap());
    let arc = id_of(&s.execute("path.create", &json!({"d": "M 0 0 C 20 400 380 500 400 0"})).unwrap());
    for big in [blob, arc] {
        s.execute("select.set", &json!({"ids": [big.0]})).unwrap();
        let c = id_of(&s.execute(CIRCLE, &json!({})).unwrap());
        let LiveShape::Ellipse { w, xf, .. } = live(&s, c) else { panic!("not a circle") };
        let (center, r) = (xf * Point::new(w / 2.0, w / 2.0), w / 2.0);
        let path = node(&s, big).path_data().unwrap().to_bezpath();
        let mut farthest: f64 = 0.0;
        for seg in path.segments() {
            for i in 0..=4000 {
                let p = vectorcraft_geom::ParamCurve::eval(&seg, f64::from(i) / 4000.0);
                farthest = farthest.max(p.distance(center));
            }
        }
        assert!(farthest <= r + 1e-6, "a point {farthest} from the center is outside the circle of radius {r}");
    }
}

/// Clip groups count by their clipping paths; for art inside one, the shape goes on top of the
/// current layer, never into the clip group.
#[test]
fn clip_groups_count_by_their_clipping_paths() {
    let mut s = session();
    let art = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let clip = id_of(&s.execute("shape.ellipse", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap());
    s.execute("select.set", &json!({"ids": [art.0, clip.0]})).unwrap();
    let group = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    s.execute("select.set", &json!({"ids": [group.0]})).unwrap();
    let id = id_of(&s.execute(CIRCLE, &json!({})).unwrap());
    let b = bounds(&s, id);
    assert!((b.width() / 2.0 - 25.01).abs() < 0.03 && b.center().distance(Point::new(35.0, 35.0)) < 0.03, "{b:?}");
    let layer = s.doc().unwrap().current_layer().unwrap();
    assert_eq!(s.doc().unwrap().doc.parent_of(id), Some(layer));
    s.execute("select.set", &json!({"ids": [clip.0]})).unwrap();
    let id = id_of(&s.execute(RECTANGLE, &json!({})).unwrap());
    assert_eq!(s.doc().unwrap().doc.parent_of(id), Some(layer));
    assert!(rect_near(bounds(&s, id), Rect::new(10.0, 10.0, 60.0, 60.0), 1e-6));
}

/// A sublayer that clips its contents counts by its clipping path, and the hidden objects of a
/// sublayer are left out; a sublayer inside a selected group (as an opened file can put one there)
/// counts the same way.
#[test]
fn sublayers_count_by_their_clipping_path_and_shown_art() {
    let mut s = session();
    let top = s.doc().unwrap().current_layer().unwrap();
    let sub = id_of(&s.execute("layer.newSublayer", &json!({"parent": top.0})).unwrap());
    s.execute("layer.setCurrent", &json!({"id": sub.0})).unwrap();
    let art = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let hidden = rect(&mut s, 300.0, 300.0, 10.0, 10.0);
    assert_eq!(s.doc().unwrap().doc.parent_of(art), Some(sub), "the art is in the sublayer");
    s.execute("select.set", &json!({"ids": [hidden.0]})).unwrap();
    s.execute("object.hide", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    let (sub_node, symbols) = (d.node(sub).unwrap().clone(), d.symbol_boxes());
    let mut g = Gathered::new();
    super::outline_points(&sub_node, &symbols, &mut g, 0);
    let (center, r) = shapes::enclosing_circle(&g.points).unwrap();
    assert!(center.distance(Point::new(50.0, 50.0)) < 1e-9 && (r - 50.0 * 2f64.sqrt()).abs() < 1e-9, "{center:?} {r}");

    // The top object of the sublayer becomes its clipping path.
    s.execute("select.none", &json!({})).unwrap();
    let clip = rect(&mut s, 20.0, 20.0, 10.0, 10.0);
    assert_eq!(s.doc().unwrap().doc.parent_of(clip), Some(sub));
    s.execute("select.none", &json!({})).unwrap();
    assert!(s.execute("layer.clippingMask.toggle", &json!({"id": sub.0})).unwrap()["clip"].as_bool().unwrap());
    let d = &s.doc().unwrap().doc;
    let sub_node = d.node(sub).unwrap().clone();
    let mut g = Gathered::new();
    super::outline_points(&sub_node, &d.symbol_boxes(), &mut g, 0);
    let (center, r) = shapes::enclosing_circle(&g.points).unwrap();
    assert!(center.distance(Point::new(25.0, 25.0)) < 1e-9 && (r - 5.0 * 2f64.sqrt()).abs() < 1e-9, "{center:?} {r}");

    let group = s
        .edit("Group", |d, _| {
            let layer = d.remove(sub)?;
            let id = d.alloc_id();
            d.insert(Some(top), usize::MAX, Node::group(id, vec![layer]))?;
            Ok(id)
        })
        .unwrap();
    s.execute("select.set", &json!({"ids": [group.0]})).unwrap();
    let c = id_of(&s.execute(CIRCLE, &json!({})).unwrap());
    let b = bounds(&s, c);
    assert!(b.center().distance(Point::new(25.0, 25.0)) < 1e-9 && (b.width() / 2.0 - (5.0 * 2f64.sqrt() + 0.01)).abs() < 1e-9, "{b:?}");
}

/// A path of a compound path selected on its own is an object, as `object.transform` moves it:
/// the shape goes around that path only, and with `each` each selected path gets a shape. The
/// compound path selected whole is one object.
#[test]
fn a_path_of_a_compound_path_selected_on_its_own_is_an_object() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 10.0, 10.0);
    let b = rect(&mut s, 120.0, 100.0, 10.0, 20.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let compound = id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap());
    let members: Vec<NodeId> = node(&s, compound).children().unwrap().iter().map(|n| n.id).collect();
    assert_eq!(members.len(), 2);
    for kind in [RECTANGLE, CIRCLE] {
        s.execute("select.set", &json!({"ids": [members[0].0]})).unwrap();
        let id = id_of(&s.execute(kind, &json!({})).unwrap());
        let (got, want) = (bounds(&s, id), bounds(&s, members[0]));
        if kind == RECTANGLE {
            assert!(rect_near(got, want, 1e-9), "{got:?} on {want:?}");
        } else {
            let d = want.width().hypot(want.height()) + 0.02;
            assert!(got.center().distance(want.center()) < 1e-9 && (got.width() - d).abs() < 1e-9, "{got:?} around {want:?}");
        }
        s.execute("select.set", &json!({"ids": [members[0].0, members[1].0]})).unwrap();
        assert_eq!(ids_of(&s.execute(kind, &json!({"each": true})).unwrap()).len(), 2, "{kind}");
        s.execute("select.set", &json!({"ids": [compound.0]})).unwrap();
        assert_eq!(ids_of(&s.execute(kind, &json!({"each": true})).unwrap()).len(), 1, "{kind}");
    }
}

/// A small closed curve whose handles lie within the 0.01 pt allowance of its one anchor is not a
/// single point: it gets a circle.
#[test]
fn a_small_loop_gets_a_circle() {
    let mut s = session();
    let lp = id_of(&s.execute("path.create", &json!({"d": "M 0 0 C 0.005 0 0 0.005 0 0"})).unwrap());
    s.execute("select.set", &json!({"ids": [lp.0]})).unwrap();
    let c = id_of(&s.execute(CIRCLE, &json!({})).unwrap());
    let LiveShape::Ellipse { w, .. } = live(&s, c) else { panic!("not a circle") };
    assert!(w > 0.02, "{w}");
}

/// Once the command has used up both kinds of halvings, a curve counts by its control points' hull
/// cut down to its bounding box: the points stay inside both and reach no farther than the curve
/// along either axis, while its handles may.
#[test]
fn a_curve_past_the_halvings_counts_by_its_hull_inside_its_bounding_box() {
    // With nothing left, `curve_points` takes no halving, so `none` stays as it is.
    let mut none = Splits { any: 0, early: 0 };
    let c = CubicBez::new((0.0, 0.0), (20.0, 400.0), (380.0, 500.0), (400.0, 0.0));
    let b = vectorcraft_geom::kurbo::ParamCurveExtrema::bounding_box(&c);
    let mut out = vec![];
    super::curve_points(c, 0, &mut out, &mut none);
    assert!(out.len() >= 4 && out.iter().all(|p| b.inflate(1e-9, 1e-9).contains(*p)), "{out:?} in {b:?}");
    assert!(out.iter().any(|p| (p.y - b.y1).abs() < 1e-9), "the curve's lowest point is reached: {out:?}");
    let (center, r) = shapes::enclosing_circle(&out).unwrap();
    for i in 0..=1000 {
        let p = vectorcraft_geom::ParamCurve::eval(&c, f64::from(i) / 1000.0);
        assert!(p.distance(center) <= r + 1e-9, "{p:?} outside");
    }
    // The handle at x = 4,000,000 stays out: the curve reaches x = 16,000,000 / 9.
    let reach = CubicBez::new((0.0, 0.0), (4.0e6, 0.0), (0.0, 0.0), (0.0, 0.0));
    out.clear();
    super::curve_points(reach, 0, &mut out, &mut none);
    let far = out.iter().map(|p| p.x).fold(0.0, f64::max);
    assert!((far - 16.0e6 / 9.0).abs() < 1e-6, "{far}");
    out.clear();
    let mut splits = Splits { any: 3, early: 0 };
    super::curve_points(c, 0, &mut out, &mut splits);
    assert_eq!(splits, Splits { any: 0, early: 0 });
    assert!(out.len() > 4, "{out:?}");
}

/// The circle fitted to a circle of four curves: less than 0.3% larger with only the early halvings
/// left, and up to about 14% larger by the curves' hulls once those are used up too.
#[test]
fn a_circle_measured_past_the_halvings_stays_close() {
    let fitted = |mut splits: Splits| {
        let circle = shapes::ellipse(Rect::new(-100.0, -100.0, 100.0, 100.0)).to_bezpath();
        let mut out = vec![];
        for seg in circle.segments() {
            super::curve_points(seg.to_cubic(), 0, &mut out, &mut splits);
        }
        shapes::enclosing_circle(&out).unwrap()
    };
    let (center, r) = fitted(Splits { any: 0, early: 1000 });
    assert!(center.to_vec2().hypot() < 0.1 && (100.0..100.3).contains(&r), "{center:?} {r}");
    let (_, r) = fitted(Splits { any: 0, early: 0 });
    assert!((110.0..115.0).contains(&r), "{r}");
}

/// Past the halvings at any depth, only the first [`super::MIN_SPLITS`] halvings of a curve take
/// from the early ones; with `each`, every shape gets an equal share of both.
#[test]
fn early_halvings_are_taken_near_the_start_of_a_curve_and_shared() {
    let mut s = Splits { any: 1, early: 1 };
    assert!(s.take(super::MIN_SPLITS + 1));
    assert!(!s.take(super::MIN_SPLITS));
    assert!(s.take(0));
    assert!(!s.take(0));
    let whole = Splits { any: super::TOTAL_SPLITS, early: super::EARLY_SPLITS };
    assert_eq!(super::split_share(0), whole);
    assert_eq!(super::split_share(1), whole);
    assert_eq!(super::split_share(4), Splits { any: super::TOTAL_SPLITS / 4, early: super::EARLY_SPLITS / 4 });
}

/// In an isolated group the shape goes on top of the group's art; in an isolated compound path it
/// goes beside the compound path, never into it.
#[test]
fn shapes_go_into_an_isolated_group_but_not_an_isolated_compound_path() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let group = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.execute("object.isolate", &json!({"id": group.0})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let id = id_of(&s.execute(RECTANGLE, &json!({})).unwrap());
    assert_eq!(s.doc().unwrap().doc.parent_of(id), Some(group));
    assert_eq!(top_of(&s, group), id);
    s.execute("object.exitIsolation", &json!({})).unwrap();

    let c = rect(&mut s, 100.0, 100.0, 10.0, 10.0);
    let e = rect(&mut s, 120.0, 100.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [c.0, e.0]})).unwrap();
    let compound = id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap());
    s.execute("object.isolate", &json!({"id": compound.0})).unwrap();
    let members: Vec<u64> = node(&s, compound).children().unwrap().iter().map(|n| n.id.0).collect();
    s.execute("select.set", &json!({"ids": members})).unwrap();
    let id = id_of(&s.execute(CIRCLE, &json!({})).unwrap());
    let d = &s.doc().unwrap().doc;
    assert_ne!(d.parent_of(id), Some(compound), "the circle does not join the compound path");
    assert_eq!(d.parent_of(id), d.parent_of(compound));
    assert_eq!(node(&s, compound).children().unwrap().len(), 2);
}

/// An offset or each that is not one the commands take is an error, and nothing is drawn; with
/// nothing selected the commands are not available.
#[test]
fn bad_params_and_an_empty_selection_draw_nothing() {
    let mut s = session();
    assert!(s.execute(RECTANGLE, &json!({})).is_err(), "nothing selected");
    assert!(s.execute(CIRCLE, &json!({})).is_err(), "nothing selected");
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    for command in [RECTANGLE, CIRCLE] {
        for p in [json!({"offset": -1}), json!({"offset": "5"}), json!({"each": "yes"})] {
            refused(&mut s, command, p);
        }
        assert!(refused(&mut s, command, json!({"offset": -1})).contains("`offset` must be 0 or more"));
    }
}

/// An offset that would put an edge of the shape more than 4,000,000 pt from the origin along
/// either axis is an error that names the canvas edge, and nothing is drawn; the largest offsets
/// that stay inside are drawn, for the rectangle and for the circle.
#[test]
fn an_offset_past_the_canvas_edge_draws_nothing() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    for command in [RECTANGLE, CIRCLE] {
        for offset in [1e308, 4.0e6] {
            let e = refused(&mut s, command, json!({ "offset": offset }));
            assert!(e.contains("the shape would reach past the edge of the canvas"), "{command} {offset}: {e}");
        }
    }
    let r = id_of(&s.execute(RECTANGLE, &json!({"offset": 3.99e6})).unwrap());
    assert!(rect_near(bounds(&s, r), Rect::new(-3.99e6, -3.99e6, 3.99e6 + 10.0, 3.99e6 + 10.0), 1e-3));
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let c = id_of(&s.execute(CIRCLE, &json!({"offset": 3.99e6})).unwrap());
    // The circle around the square from (0, 0) to (10, 10), 0.01 pt larger, then the offset.
    let radius = 50.0f64.sqrt() + 0.01 + 3.99e6;
    assert!(rect_near(bounds(&s, c), Rect::new(5.0 - radius, 5.0 - radius, 5.0 + radius, 5.0 + radius), 1e-3));
}

/// The limit holds along each axis on its own: a shape whose right edge would pass 4,000,000 pt is
/// refused although its other edges, and its distance along the other axis, are small.
#[test]
fn one_edge_past_the_canvas_edge_is_enough_to_refuse() {
    let mut s = session();
    let a = rect(&mut s, 3_999_980.0, 0.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    for command in [RECTANGLE, CIRCLE] {
        let e = refused(&mut s, command, json!({"offset": 20}));
        assert!(e.contains("the shape would reach past the edge of the canvas"), "{command}: {e}");
    }
    assert!(s.execute(RECTANGLE, &json!({"offset": 5})).is_ok(), "the right edge at 3,999,995 pt is inside");
}

/// A large circle close to the canvas edge, whose arcs stay curved after the most halvings, gets a
/// circle within 0.05 pt of the farthest point of its curves, which stays inside the edge.
#[test]
fn a_large_circle_near_the_canvas_edge_gets_a_close_circle() {
    let mut s = session();
    let e = id_of(&s.execute("shape.ellipse", &json!({"x": 3_799_950.0, "y": 0.0, "width": 200_000.0, "height": 200_000.0})).unwrap());
    s.execute("select.set", &json!({"ids": [e.0]})).unwrap();
    let c = id_of(&s.execute(CIRCLE, &json!({})).unwrap());
    let LiveShape::Ellipse { w, xf, .. } = live(&s, c) else { panic!("not a circle") };
    let center = xf * Point::new(w / 2.0, w / 2.0);
    let mut farthest: f64 = 0.0;
    for seg in node(&s, e).path_data().unwrap().to_bezpath().segments() {
        for i in 0..=4000 {
            farthest = farthest.max(vectorcraft_geom::ParamCurve::eval(&seg, f64::from(i) / 4000.0).distance(center));
        }
    }
    assert!(w / 2.0 >= farthest && w / 2.0 < farthest + 0.05, "radius {} for curves reaching {farthest}", w / 2.0);
}

/// The commands are dimmed while an object of a symbol instance's art is selected, which they
/// refuse, and available again once it is not.
#[test]
fn a_selected_symbol_part_dims_the_commands() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let enabled = |s: &Session, id: &str| (crate::find_command(id).unwrap().enabled)(s);
    for command in [RECTANGLE, CIRCLE] {
        assert!(enabled(&s, command).is_ok(), "{command}");
    }
    s.doc_mut().unwrap().selection.symbol_part = Some(vectorcraft_doc::SymbolPart { instance: a, path: vec![0] });
    for command in [RECTANGLE, CIRCLE] {
        assert!(enabled(&s, command).is_err(), "{command} with a symbol part selected");
    }
    s.doc_mut().unwrap().selection.symbol_part = None;
    assert!(enabled(&s, RECTANGLE).is_ok());
}

/// Many points keep only their hull's corners, and the circle around them is the one around all
/// of the points.
#[test]
fn gathered_points_keep_their_hull_and_circle() {
    let mut g = Gathered::new();
    let c = Point::new(500.0, 300.0);
    for i in 0..200_000 {
        let angle = f64::from(i) * 0.618;
        let r = if i % 1000 == 0 { 250.0 } else { f64::from(i % 200) };
        g.points.push(c + Vec2::from_angle(angle) * r);
        g.compact();
    }
    assert!(g.points.len() <= COMPACT_AT + 1, "{} points kept", g.points.len());
    let (center, radius) = shapes::enclosing_circle(&g.points).unwrap();
    assert!(center.distance(c) < 1e-6 && (radius - 250.0).abs() < 1e-6, "{center:?} {radius}");
}
