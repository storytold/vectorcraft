//! Common Tangents: the lines for circles, turned ellipses and convex paths, the centerline,
//! the paint, one undo step, the cases with no tangent, and refused selections and params.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, NodeId, NodeKind};
use vectorcraft_geom::{Point, Vec2};

use crate::Session;

const C: &str = super::COMMON_TANGENTS;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn circle(s: &mut Session, cx: f64, cy: f64, r: f64) -> NodeId {
    id_of(&s.execute("shape.ellipse", &json!({"x": cx - r, "y": cy - r, "width": 2.0 * r, "height": 2.0 * r})).unwrap())
}

fn select(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// The ends of each new line, as `ids` lists them.
fn lines(s: &Session, r: &Value) -> Vec<(Point, Point)> {
    let d = &s.doc().unwrap().doc;
    r["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let n = d.node(NodeId(v.as_u64().unwrap())).unwrap();
            let NodeKind::Path { path, .. } = &n.kind else { panic!("not a path") };
            assert_eq!(path.subpaths.len(), 1);
            let sp = &path.subpaths[0];
            assert!(!sp.closed);
            assert_eq!(sp.anchors.len(), 2);
            (sp.anchors[0].p, sp.anchors[1].p)
        })
        .collect()
}

fn near(p: Point, q: Point) -> bool {
    p.distance(q) < 1e-6
}

fn has_line(found: &[(Point, Point)], p: Point, q: Point) -> bool {
    found.iter().any(|(a, b)| (near(*a, p) && near(*b, q)) || (near(*a, q) && near(*b, p)))
}

#[test]
fn two_circles_get_four_tangents_at_the_exact_points_as_one_undo_step() {
    let mut s = session();
    let a = circle(&mut s, 100.0, 200.0, 20.0);
    let b = circle(&mut s, 300.0, 200.0, 40.0);
    select(&mut s, &[a, b]);
    let before = steps(&s);
    let r = s.execute(C, &json!({})).unwrap();
    assert_eq!((r["external"].as_u64(), r["internal"].as_u64(), r["centerline"].is_null()), (Some(2), Some(2), true), "{r}");
    assert_eq!(steps(&s), before + 1);
    let found = lines(&s, &r);
    assert_eq!(found.len(), 4);
    let (c1, c2) = (Point::new(100.0, 200.0), Point::new(300.0, 200.0));
    // External: (c1 - c2) · n = r2 - r1, n.x = -0.1; internal: (c1 - c2) · n = -(r1 + r2), n.x = 0.3.
    for (nx, external) in [(-0.1f64, true), (0.3, false)] {
        for sy in [1.0, -1.0] {
            let n = Vec2::new(nx, sy * (1.0 - nx * nx).sqrt());
            let far = if external { c2 + n * 40.0 } else { c2 - n * 40.0 };
            assert!(has_line(&found, c1 + n * 20.0, far), "{n:?} in {found:?}");
        }
    }
    // The lines are the selection, above the two circles.
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects.len(), 4);
    let ids: Vec<NodeId> = r["ids"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect();
    let order = st.doc.paint_order([a, b].into_iter().chain(ids.iter().copied()));
    assert_eq!(&order[..2], &[a, b]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(ids.iter().all(|id| s.doc().unwrap().doc.node(*id).is_none()), "one undo step removes them all");
}

#[test]
fn turned_ellipses_are_measured_exactly() {
    let mut s = session();
    let a = id_of(&s.execute("shape.ellipse", &json!({"x": 60, "y": 170, "width": 80, "height": 30})).unwrap());
    select(&mut s, &[a]);
    s.execute("object.rotate", &json!({"angle": 30, "origin": [100, 185]})).unwrap();
    let b = id_of(&s.execute("shape.ellipse", &json!({"x": 330, "y": 120, "width": 50, "height": 120})).unwrap());
    select(&mut s, &[b]);
    s.execute("object.rotate", &json!({"angle": -70, "origin": [355, 180]})).unwrap();
    select(&mut s, &[a, b]);
    let r = s.execute(C, &json!({"tangents": "all"})).unwrap();
    assert_eq!((r["external"].as_u64(), r["internal"].as_u64()), (Some(2), Some(2)), "{r}");
    // Each end lies on its ellipse, where the ellipse runs along the line: turned back about the
    // center, (x/rx)² + (y/ry)² = 1 and the gradient is square to the line.
    let on = |p: Point, center: Point, rx: f64, ry: f64, deg: f64, dir: Vec2| {
        let turn = vectorcraft_geom::kurbo::Affine::rotate(-deg.to_radians());
        // `object.rotate` turns counter-clockwise on the page (y down): undo it in page terms.
        let page = |v: Vec2| Vec2::new(v.x, -v.y);
        let q = page((turn * page(p - center).to_point()).to_vec2());
        let d = page((turn * page(dir).to_point()).to_vec2());
        let value = (q.x / rx).powi(2) + (q.y / ry).powi(2);
        let grad = Vec2::new(q.x / (rx * rx), q.y / (ry * ry));
        (value - 1.0).abs() < 1e-9 && (grad.dot(d) / (grad.hypot() * d.hypot())).abs() < 1e-7
    };
    for (p, q) in lines(&s, &r) {
        let dir = q - p;
        assert!(on(p, Point::new(100.0, 185.0), 40.0, 15.0, 30.0, dir), "{p:?} on the first ellipse");
        assert!(on(q, Point::new(355.0, 180.0), 25.0, 60.0, -70.0, dir), "{q:?} on the second ellipse");
    }
}

#[test]
fn a_convex_bezier_path_and_the_centerline() {
    let mut s = session();
    let anchors = json!([
        {"x": 0, "y": 40, "in": [0, 70], "out": [0, 10]},
        {"x": 50, "y": 0, "in": [20, 0], "out": [90, 0]},
        {"x": 110, "y": 50, "in": [110, 20], "out": [110, 90]},
        {"x": 40, "y": 100, "in": [80, 100], "out": [10, 100]}
    ]);
    let blob = id_of(&s.execute("path.create", &json!({"anchors": anchors, "closed": true})).unwrap());
    let round = circle(&mut s, 300.0, 200.0, 30.0);
    let r = s.execute(C, &json!({"ids": [blob.0, round.0], "centerline": true})).unwrap();
    assert_eq!((r["external"].as_u64(), r["internal"].as_u64()), (Some(2), Some(2)), "{r}");
    let found = lines(&s, &r);
    assert_eq!(found.len(), 5, "four tangents and the centerline");
    // The centerline is the last line and its id is given: from the blob's center of area to the
    // circle's center.
    assert_eq!(r["centerline"], r["ids"][4]);
    let (from, to) = found[4];
    assert!(near(to, Point::new(300.0, 200.0)), "{to:?}");
    assert!(from.x > 30.0 && from.x < 80.0 && from.y > 30.0 && from.y < 70.0, "{from:?}");
    // A tangent touches without crossing: every point of both paths is on one side of it.
    let d = &s.doc().unwrap().doc;
    let points = |id: NodeId| {
        let sp = d.node(id).unwrap().path_data().unwrap().subpaths[0].clone();
        let segments: Vec<_> = (0..sp.segment_count()).map(|k| sp.segment(k)).collect();
        segments.iter().flat_map(|seg| (0..200).map(|j| vectorcraft_geom::kurbo::ParamCurve::eval(seg, j as f64 / 200.0))).collect::<Vec<_>>()
    };
    let (pa, pb) = (points(blob), points(round));
    for (p, q) in &found[..4] {
        let n = Vec2::new(-(q.y - p.y), q.x - p.x).normalize();
        for pts in [&pa, &pb] {
            let side: Vec<f64> = pts.iter().map(|x| (*x - *p).dot(n)).collect();
            let (lo, hi) = side.iter().fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
            // The circle is measured as an exact circle, which its four curves follow to within
            // about 0.03% of the radius.
            assert!(lo > -1e-2 || hi < 1e-2, "crosses: {lo} .. {hi}");
        }
    }
}

#[test]
fn the_lines_take_the_current_stroke_and_no_fill() {
    let mut s = session();
    let a = circle(&mut s, 100.0, 100.0, 20.0);
    let b = circle(&mut s, 300.0, 100.0, 20.0);
    s.execute("select.none", &json!({})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let r = s.execute(C, &json!({"ids": [a.0, b.0], "tangents": "external"})).unwrap();
    assert_eq!((r["external"].as_u64(), r["internal"].as_u64()), (Some(2), Some(0)), "{r}");
    let d = &s.doc().unwrap().doc;
    for v in r["ids"].as_array().unwrap() {
        let n = d.node(NodeId(v.as_u64().unwrap())).unwrap();
        assert!(n.appearance.fill().is_none_or(|f| f.paint.is_none()), "no fill");
        let st = n.appearance.stroke().expect("stroked");
        assert!(!st.paint.is_none(), "black when the stroke is None");
    }
    // A new-art appearance with two fills and a stroke: the lines keep the stroke and neither fill.
    let mut look = vectorcraft_doc::Appearance::basic(Paint::solid(Color::BLACK), Paint::solid(Color::BLACK), 2.0);
    let fill = look.items.iter().find(|i| matches!(i, AppearanceItem::Fill(_))).cloned().unwrap();
    look.items.push(fill);
    s.paint.appearance = Some(look);
    s.paint.inherited = false;
    let r2 = s.execute(C, &json!({"ids": [a.0, b.0], "tangents": "external"})).unwrap();
    let d2 = &s.doc().unwrap().doc;
    for v in r2["ids"].as_array().unwrap() {
        let n = d2.node(NodeId(v.as_u64().unwrap())).unwrap();
        assert!(!n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Fill(_))), "no fill at all");
        assert!(n.appearance.stroke().is_some());
    }
    // Two equal circles: the external tangents run along their tops and bottoms.
    assert!(has_line(&lines(&s, &r), Point::new(100.0, 80.0), Point::new(300.0, 80.0)));
    assert!(has_line(&lines(&s, &r), Point::new(100.0, 120.0), Point::new(300.0, 120.0)));
}

#[test]
fn nested_and_overlapping_paths_change_nothing_and_say_why() {
    let mut s = session();
    let big = circle(&mut s, 200.0, 200.0, 100.0);
    let small = circle(&mut s, 220.0, 200.0, 20.0);
    let before = (steps(&s), s.doc().unwrap().doc.node_count());
    let r = s.execute(C, &json!({"ids": [big.0, small.0]})).unwrap();
    assert_eq!(r["ids"], json!([]));
    assert!(r["warning"].as_str().unwrap().contains("inside the other"), "{r}");
    assert_eq!((steps(&s), s.doc().unwrap().doc.node_count()), before, "nothing changes");
    // Overlapping circles have external tangents only.
    let a = circle(&mut s, 100.0, 100.0, 30.0);
    let b = circle(&mut s, 140.0, 100.0, 30.0);
    let before = (steps(&s), s.doc().unwrap().doc.node_count());
    let r = s.execute(C, &json!({"ids": [a.0, b.0], "tangents": "internal"})).unwrap();
    assert!(r["warning"].as_str().unwrap().contains("overlap"), "{r}");
    assert_eq!((steps(&s), s.doc().unwrap().doc.node_count()), before);
    let r = s.execute(C, &json!({"ids": [a.0, b.0]})).unwrap();
    assert_eq!((r["external"].as_u64(), r["internal"].as_u64()), (Some(2), Some(0)), "{r}");
    // Circles that touch share the tangent where they touch.
    let c = circle(&mut s, 100.0, 300.0, 10.0);
    let e = circle(&mut s, 130.0, 300.0, 20.0);
    let r = s.execute(C, &json!({"ids": [c.0, e.0], "tangents": "internal"})).unwrap();
    assert_eq!(r["internal"].as_u64(), Some(1), "{r}");
    let (p, q) = lines(&s, &r)[0];
    assert!((p.x - 110.0).abs() < 1e-5 && (q.x - 110.0).abs() < 1e-5 && (p.distance(q) - 20.0).abs() < 1e-5, "{p:?} {q:?}");
    // A circle touching the inside of a larger one: no internal tangent, and the message says the
    // one lies inside the other rather than that they overlap.
    let outer = circle(&mut s, 400.0, 200.0, 50.0);
    let inner = circle(&mut s, 430.0, 200.0, 20.0);
    let before = (steps(&s), s.doc().unwrap().doc.node_count());
    let r = s.execute(C, &json!({"ids": [outer.0, inner.0], "tangents": "internal"})).unwrap();
    assert_eq!(r["ids"], json!([]));
    assert!(r["warning"].as_str().unwrap().contains("inside the other, so they have no internal"), "{r}");
    assert_eq!((steps(&s), s.doc().unwrap().doc.node_count()), before);
    let r = s.execute(C, &json!({"ids": [outer.0, inner.0]})).unwrap();
    assert_eq!((r["external"].as_u64(), r["internal"].as_u64()), (Some(1), Some(0)), "{r}");
}

#[test]
fn bad_selections_and_params_are_refused() {
    let mut s = session();
    let a = circle(&mut s, 100.0, 100.0, 20.0);
    let b = circle(&mut s, 300.0, 100.0, 20.0);
    let star = id_of(&s.execute("shape.star", &json!({"cx": 200, "cy": 300, "radius1": 40, "radius2": 15, "points": 5})).unwrap());
    let open = id_of(&s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 0}, {"x": 50, "y": 0}, {"x": 50, "y": 50}]})).unwrap());
    let group_of = |s: &mut Session, ids: &[NodeId]| {
        select(s, ids);
        id_of(&s.execute("object.group", &json!({})).unwrap())
    };
    let before = (steps(&s), s.doc().unwrap().doc.node_count());
    let err = |s: &mut Session, p: Value| s.execute(C, &p).unwrap_err().to_string();
    assert!(err(&mut s, json!({"ids": [a.0, b.0], "tangents": "both"})).contains("tangents"));
    assert!(err(&mut s, json!({"ids": [a.0, b.0], "tangents": 3})).contains("tangents"));
    assert!(err(&mut s, json!({"ids": [a.0, b.0], "centerline": "yes"})).contains("centerline"));
    assert!(err(&mut s, json!({"ids": [a.0, b.0], "which": "all"})).contains("which"), "unknown keys are refused");
    assert!(err(&mut s, json!({"ids": [a.0]})).contains("select two closed paths"));
    assert!(err(&mut s, json!({"ids": [a.0, b.0, star.0]})).contains("select two closed paths"));
    assert!(err(&mut s, json!({"ids": [a.0, star.0]})).contains("not convex"));
    assert!(err(&mut s, json!({"ids": [a.0, open.0]})).contains("open"));
    assert!(s.execute(C, &json!({"ids": [a.0, a.0]})).is_err(), "the same path twice");
    assert_eq!((steps(&s), s.doc().unwrap().doc.node_count()), before, "nothing changes");
    let copy = id_of(&s.execute("shape.ellipse", &json!({"x": 80, "y": 80, "width": 40, "height": 40})).unwrap());
    assert!(err(&mut s, json!({"ids": [a.0, copy.0]})).contains("same outline"));
    let g = group_of(&mut s, &[b]);
    assert!(err(&mut s, json!({"ids": [a.0, g.0]})).contains("select two closed paths"));
}
