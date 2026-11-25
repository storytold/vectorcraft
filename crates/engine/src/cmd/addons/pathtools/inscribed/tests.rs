//! Add Inscribed Circle: the largest circle in a square, a concave shape and a ring compound path
//! under its fill rule, one circle per object as one undo step, the drawn circle inside where its
//! curves bulge, and the errors that leave the document as it was.

use serde_json::{Value, json};
use vectorcraft_doc::{LiveShape, NodeId, NodeKind};
use vectorcraft_geom::Point;

use super::COMMAND;
use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn ids_of(v: &Value) -> Vec<NodeId> {
    v["ids"].as_array().unwrap().iter().map(|i| NodeId(i.as_u64().unwrap())).collect()
}

fn select(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn object_count(s: &Session) -> usize {
    let mut n = 0;
    s.doc().unwrap().doc.walk(|_| n += 1);
    n
}

/// The center and radius of the live circle `id`.
fn circle(s: &Session, id: NodeId) -> (Point, f64) {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Path { live: Some(live), .. } => match live.as_ref() {
            LiveShape::Ellipse { w, h, xf, .. } => {
                assert!((w - h).abs() < 1e-9, "a circle, not an ellipse");
                (*xf * Point::new(w / 2.0, h / 2.0), w / 2.0)
            }
            other => panic!("not a live ellipse: {other:?}"),
        },
        other => panic!("not a live shape: {other:?}"),
    }
}

/// The smallest drawn radius for a largest circle of radius `largest`: smaller by the curves'
/// bulge, and by the search's 0.02 pt.
fn least(largest: f64) -> f64 {
    largest / (1.0 + super::CIRCLE_BULGE) - 0.02
}

/// The command on the selection fails with a message holding `want`, and the document is as it was.
fn refused(s: &mut Session, want: &str) {
    let (undo, objects) = (undo_len(s), object_count(s));
    let e = s.execute(COMMAND, &json!({})).unwrap_err().to_string();
    assert!(e.contains(want), "{e}");
    assert_eq!((undo_len(s), object_count(s)), (undo, objects));
}

/// The circle in a square sits at its center and touches its sides, gets the current paint, goes
/// on top of the layer and is selected; one undo removes it.
#[test]
fn a_square_gets_the_circle_that_touches_its_sides() {
    let mut s = session();
    let sq = id_of(&s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap());
    let (undo, objects) = (undo_len(&s), object_count(&s));
    let out = s.execute(COMMAND, &json!({})).unwrap();
    let id = id_of(&out);
    let (center, r) = circle(&s, id);
    assert!((center - Point::new(150.0, 150.0)).hypot() < 0.02 && r <= 50.0 + 1e-6 && r > least(50.0), "{center:?} {r}");
    assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
    let d = &s.doc().unwrap().doc;
    let parent = d.parent_of(sq).unwrap();
    assert_eq!(d.parent_of(id), Some(parent));
    assert_eq!(d.node(parent).unwrap().children().unwrap().last().map(|n| n.id), Some(id), "on top of the layer");
    // The current paint, as the rectangle drawn before got it.
    let (n, r) = (d.node(id).unwrap(), d.node(sq).unwrap());
    assert_eq!((n.appearance.fill_paint(), n.appearance.stroke_paint()), (r.appearance.fill_paint(), r.appearance.stroke_paint()));
    assert_eq!(undo_len(&s), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(object_count(&s), objects);
}

/// The circle in an L sits in its corner, inside the shape, where the box's center is outside.
#[test]
fn a_concave_shape_gets_the_circle_in_its_corner() {
    let mut s = session();
    let anchors = [(0, 0), (100, 0), (100, 30), (30, 30), (30, 100), (0, 100)].map(|(x, y)| json!({"x": x, "y": y}));
    s.execute("path.create", &json!({"anchors": anchors, "closed": true})).unwrap();
    let id = id_of(&s.execute(COMMAND, &json!({})).unwrap());
    let (center, r) = circle(&s, id);
    let largest = 30.0 * std::f64::consts::SQRT_2 / (1.0 + std::f64::consts::SQRT_2);
    assert!(r <= largest + 1e-6 && r > least(largest), "{r}");
    assert!(center.x < 30.0 && center.y < 30.0, "{center:?}");
}

/// A compound path of two concentric circles: under even-odd the inner one is a hole, and the
/// circle lies in the ring beside it. Under nonzero it is a hole while the two circles turn
/// opposite ways, and with both turning the same way the circle fills the outer one.
#[test]
fn a_ring_compound_path_gets_a_circle_by_its_fill_rule() {
    let mut s = session();
    let outer = id_of(&s.execute("shape.ellipse", &json!({"x": 100, "y": 100, "width": 200, "height": 200})).unwrap());
    let inner = id_of(&s.execute("shape.ellipse", &json!({"x": 160, "y": 160, "width": 80, "height": 80})).unwrap());
    select(&mut s, &[outer, inner]);
    let compound = id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap());
    let ring = Point::new(200.0, 200.0);
    let cases = [("evenOdd", false, 70.0, 30.0), ("nonZero", false, 70.0, 30.0), ("nonZero", true, 0.0, 100.0), ("evenOdd", true, 70.0, 30.0)];
    for (rule, same_way, around, largest) in cases {
        select(&mut s, &[compound]);
        let both = s.doc().unwrap().doc.node(compound).unwrap().children().unwrap().iter().map(|n| n.id.0).collect::<Vec<_>>();
        s.execute("path.reverse", &json!({"ids": both, "reversed": true})).unwrap();
        if !same_way {
            s.execute("path.reverse", &json!({"ids": [both[1]], "reversed": false})).unwrap();
        }
        s.execute("path.setFillRule", &json!({"rule": rule, "ids": [compound.0]})).unwrap();
        select(&mut s, &[compound]);
        let id = id_of(&s.execute(COMMAND, &json!({})).unwrap());
        let (center, r) = circle(&s, id);
        assert!(r > least(largest) && r < largest + 0.0003 * 140.0, "{rule} {same_way}: {r}");
        assert!(((center - ring).hypot() - around).abs() < 0.1, "{rule} {same_way}: {center:?}");
    }
}

/// Each selected path gets its own circle, all as one undo step; a path of a compound path
/// selected on its own gets a circle of its own outline.
#[test]
fn each_selected_path_gets_a_circle_in_one_step() {
    let mut s = session();
    let a = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap());
    let b = id_of(&s.execute("shape.rectangle", &json!({"x": 100, "y": 0, "width": 40, "height": 80})).unwrap());
    select(&mut s, &[a, b]);
    let undo = undo_len(&s);
    let ids = ids_of(&s.execute(COMMAND, &json!({})).unwrap());
    assert_eq!(ids.len(), 2);
    assert_eq!(undo_len(&s), undo + 1);
    let centers: Vec<Point> = ids.iter().map(|id| circle(&s, *id).0).collect();
    assert!(centers.iter().any(|c| c.x < 40.0) && centers.iter().any(|c| c.x > 100.0), "{centers:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    select(&mut s, &[a, b]);
    let compound = id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap());
    let member = s.doc().unwrap().doc.node(compound).unwrap().children().unwrap()[0].id;
    select(&mut s, &[member]);
    let id = id_of(&s.execute(COMMAND, &json!({})).unwrap());
    let r = circle(&s, id).1;
    assert!(r <= 20.0 + 1e-6 && r > least(20.0), "the member's own square: {r}");
    // Inside the isolated compound path, the circle joins it.
    s.execute("object.isolate", &json!({"id": compound.0})).unwrap();
    select(&mut s, &[member]);
    let id = id_of(&s.execute(COMMAND, &json!({})).unwrap());
    assert_eq!(s.doc().unwrap().doc.parent_of(id), Some(compound));
}

/// The drawn circle stays inside a large square turned so that it touches the square's sides
/// where its curves bulge most past a true circle, about 20° from each axis.
#[test]
fn the_drawn_circle_stays_inside_where_its_curves_bulge() {
    let mut s = session();
    let (center, half) = (Point::new(1200.0, 1200.0), 1000.0);
    for degrees in [15.0_f64, 19.4, 25.0] {
        let (sin, cos) = degrees.to_radians().sin_cos();
        let corner = |u: f64, v: f64| json!({"x": center.x + u * cos - v * sin, "y": center.y + u * sin + v * cos});
        let anchors = [(-half, -half), (half, -half), (half, half), (-half, half)].map(|(u, v)| corner(u, v));
        s.execute("path.create", &json!({"anchors": anchors, "closed": true})).unwrap();
        let id = id_of(&s.execute(COMMAND, &json!({})).unwrap());
        let NodeKind::Path { path, .. } = &s.doc().unwrap().doc.node(id).unwrap().kind else {
            panic!("not a path");
        };
        for seg in path.to_bezpath().segments() {
            for i in 0..=200 {
                let p = vectorcraft_geom::ParamCurve::eval(&seg, f64::from(i) / 200.0) - center;
                let (u, v) = (p.x * cos + p.y * sin, -p.x * sin + p.y * cos);
                assert!(u.abs().max(v.abs()) <= half, "{degrees}°: ({u}, {v}) is outside the square");
            }
        }
    }
}

/// An open path, an object that is not a path or compound path, a path that fills no area and an
/// empty selection are errors, and nothing is drawn.
#[test]
fn open_paths_other_objects_and_no_selection_are_refused() {
    let mut s = session();
    assert!(s.execute(COMMAND, &json!({})).is_err(), "nothing selected");
    let open = id_of(&s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 0}, {"x": 80, "y": 0}, {"x": 0, "y": 60}]})).unwrap());
    select(&mut s, &[open]);
    refused(&mut s, "is not a closed path or compound path");
    let sq = id_of(&s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 50, "height": 50})).unwrap());
    select(&mut s, &[sq]);
    let group = id_of(&s.execute("object.group", &json!({})).unwrap());
    select(&mut s, &[group]);
    refused(&mut s, "is not a closed path or compound path");
    // One refused object leaves the other selected objects without a circle too.
    select(&mut s, &[group, open]);
    refused(&mut s, "is not a closed path or compound path");
    let flat = [(0, 300), (100, 300), (50, 300)].map(|(x, y)| json!({"x": x, "y": y}));
    let flat = id_of(&s.execute("path.create", &json!({"anchors": flat, "closed": true})).unwrap());
    select(&mut s, &[flat]);
    refused(&mut s, "fills no area");
    // A selected layer is refused too, alone or with a path.
    let mut layer = None;
    s.doc().unwrap().doc.walk(|n| {
        if n.is_layer() {
            layer.get_or_insert(n.id);
        }
    });
    let layer = layer.unwrap();
    let path = id_of(&s.execute("shape.rectangle", &json!({"x": 200, "y": 100, "width": 50, "height": 50})).unwrap());
    for ids in [vec![layer], vec![layer, path]] {
        select(&mut s, &ids);
        refused(&mut s, "is not a closed path or compound path");
    }
}
