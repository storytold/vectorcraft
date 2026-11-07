//! Editing envelopes on the canvas: Edit Contents hit-testing, mesh points and handles.

use serde_json::{Value, json};
use vectorcraft_doc::hit::{HitOptions, hit_test};
use vectorcraft_doc::live::{self, EnvelopeMap};
use vectorcraft_doc::{EnvelopeKind, Node, NodeKind};
use vectorcraft_geom::{Affine, Point, Vec2};
use vectorcraft_tools::{PointerEvent, PointerKind};

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
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

/// A 200×100 rectangle at (100, 100) enveloped by `cmd` → (rectangle, envelope).
fn enveloped(s: &mut Session, cmd: &str, params: Value) -> (NodeId, NodeId) {
    let a = id_of(&s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 200, "height": 100})).unwrap());
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    (a, id_of(&s.execute(cmd, &params).unwrap()))
}

fn mesh(s: &Session, e: NodeId) -> (u32, u32, Vec<Point>, Vec<[Vec2; 4]>) {
    match node(s, e).kind {
        NodeKind::Envelope { kind, .. } => match *kind {
            EnvelopeKind::Mesh { rows, cols, points, handles } => (rows, cols, points, handles),
            k => panic!("{k:?}"),
        },
        k => panic!("{k:?}"),
    }
}

#[test]
fn edit_contents_clicks_reach_the_content_where_it_is() {
    let mut s = session();
    let (a, e) = enveloped(&mut s, "object.envelope.makeWithWarp", json!({"style": "arch", "bend": 50}));
    let doc = |s: &Session| s.doc().unwrap().doc.clone();
    // Inside the rectangle near its bottom middle, which the arch lifted away.
    let below = Point::new(200.0, 195.0);
    let h = hit_test(&doc(&s), below, HitOptions::default()).unwrap();
    assert_eq!((h.leaf, h.top_object(None), h.contents_of), (e, e, None), "the envelope is one object");
    s.execute("object.envelope.editContents", &json!({"editing": true})).unwrap();
    let h = hit_test(&doc(&s), below, HitOptions::default()).unwrap();
    assert_eq!((h.leaf, h.top_object(None), h.contents_of), (a, a, Some(e)), "Edit Contents: the content is the object clicked");
    // The Selection tool picks it.
    s.execute("select.none", &json!({})).unwrap();
    s.select_tool("selection", ViewInfo::default()).unwrap();
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, 200.0, 195.0), ViewInfo::default()).unwrap();
    }
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
}

#[test]
fn mesh_handles_start_from_the_smooth_mesh() {
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithMesh", json!({"rows": 2, "cols": 2}));
    s.execute("object.envelope.setMeshPoint", &json!({"id": e.0, "index": 4, "x": 210, "y": 130})).unwrap();
    assert!(mesh(&s, e).3.is_empty(), "moving a point keeps the smooth mesh");
    let smooth = |s: &Session, u: f64, v: f64| EnvelopeMap::of(&node(s, e)).unwrap().at(u, v);
    let before: Vec<Point> = (0..=8).flat_map(|i| [(i as f64 / 8.0, 0.5), (0.5, i as f64 / 8.0)]).map(|(u, v)| smooth(&s, u, v)).collect();
    // Placing a handle where it already is fills in the handles: the grid lines don't move.
    let (rows, cols, points, _) = mesh(&s, e);
    let h = live::smooth_handles(rows, cols, &points)[4][live::H_RIGHT];
    let end = points[4] + h;
    s.execute("object.envelope.setMeshPoint", &json!({"id": e.0, "index": 4, "x": end.x, "y": end.y, "handle": 0})).unwrap();
    assert_eq!(mesh(&s, e).3.len(), 9);
    let after: Vec<Point> = (0..=8).flat_map(|i| [(i as f64 / 8.0, 0.5), (0.5, i as f64 / 8.0)]).map(|(u, v)| smooth(&s, u, v)).collect();
    for (p, q) in before.iter().zip(&after) {
        assert!(p.distance(*q) < 1e-6, "{p:?} vs {q:?}");
    }
    // Now the handle bends the row line up right of the point.
    let y0 = smooth(&s, 0.62, 0.5).y;
    s.execute("object.envelope.setMeshPoint", &json!({"id": e.0, "index": 4, "x": end.x, "y": end.y - 40.0, "handle": 0})).unwrap();
    assert!(smooth(&s, 0.62, 0.5).y < y0 - 1.0, "{} vs {y0}", smooth(&s, 0.62, 0.5).y);
    assert!(s.execute("object.envelope.setMeshPoint", &json!({"id": e.0, "index": 4, "x": 0, "y": 0, "handle": 7})).is_err());
}

#[test]
fn mesh_commands_edit_mesh_envelopes() {
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithMesh", json!({"rows": 1, "cols": 1}));
    let r = s.execute("object.mesh.addLine", &json!({"id": e.0, "x": 150, "y": 175})).unwrap();
    let (rows, cols, points, handles) = mesh(&s, e);
    assert_eq!((rows, cols, handles.len()), (2, 2, 9));
    let i = r["index"].as_u64().unwrap() as usize;
    assert!(points[i].distance(Point::new(150.0, 175.0)) < 0.5);
    s.execute("object.mesh.movePoint", &json!({"id": e.0, "index": i, "x": 160, "y": 160})).unwrap();
    assert!(mesh(&s, e).2[i].distance(Point::new(160.0, 160.0)) < 1e-9);
    s.execute("object.mesh.movePoint", &json!({"id": e.0, "index": i, "x": 200, "y": 160, "handle": 0})).unwrap();
    assert!((mesh(&s, e).3[i][0] - Vec2::new(40.0, 0.0)).hypot() < 1e-9);
    s.execute("object.mesh.deletePoint", &json!({"id": e.0, "index": i})).unwrap();
    assert_eq!(mesh(&s, e).0, 1);
    assert!(s.execute("object.mesh.setPointColor", &json!({"id": e.0, "index": 0, "color": "#ff0000"})).is_err(), "envelope points have no colour");
    // One undo step per edit.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(mesh(&s, e).0, 2);
}

/// `object.envelope.setMeshPoint` refuses an `index` that is negative or not a whole number, and
/// Make and Reset with Mesh refuse `rows`, `cols` or `columns` that are not whole numbers; the
/// envelope stays as it was, and 4.0 names point 4.
#[test]
fn mesh_envelopes_refuse_an_index_or_count_that_is_not_a_whole_number() {
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithMesh", json!({"rows": 2, "cols": 2}));
    let before = node(&s, e);
    for (index, why) in [(json!(4.5), "`index` must be a whole number"), (json!(-1), "index must be ≥ 0"), (json!(-0.5), "index must be ≥ 0")] {
        let r = s.execute("object.envelope.setMeshPoint", &json!({"id": e.0, "index": index, "x": 0, "y": 0}));
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg == why), "{index}: {r:?}");
    }
    for (key, p) in [("rows", json!({"rows": 2.5})), ("cols", json!({"cols": 1.5})), ("columns", json!({"columns": 3.25}))] {
        let r = s.execute("object.envelope.resetWithMesh", &p);
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == format!("`{key}` must be a whole number")), "{p}: {r:?}");
    }
    assert_eq!(node(&s, e), before, "nothing changed");
    let b = id_of(&s.execute("shape.rectangle", &json!({"x": 400, "y": 100, "width": 100, "height": 100})).unwrap());
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    assert!(s.execute("object.envelope.makeWithMesh", &json!({"rows": 2, "cols": 2.5})).is_err());
    assert!(matches!(node(&s, b).kind, NodeKind::Path { .. }), "not enveloped");
    s.execute("object.envelope.setMeshPoint", &json!({"id": e.0, "index": 4.0, "x": 210, "y": 130})).unwrap();
    assert_eq!(mesh(&s, e).2[4], Point::new(210.0, 130.0));
}

#[test]
fn mesh_handles_turn_with_the_envelope_and_round_trip() {
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithMesh", json!({"rows": 1, "cols": 1}));
    s.execute("object.envelope.setMeshPoint", &json!({"id": e.0, "index": 0, "x": 150, "y": 100, "handle": 0})).unwrap();
    s.execute("object.rotate", &json!({"angle": 90, "origin": [200, 150]})).unwrap();
    let h = mesh(&s, e).3[0][0];
    let want = Affine::rotate(-std::f64::consts::FRAC_PI_2) * Point::new(50.0, 0.0);
    assert!((h - want.to_vec2()).hypot() < 1e-9, "{h:?}");
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(d, false)).unwrap();
    assert_eq!(back.node(e), d.node(e));
    // Mesh envelopes saved before handles existed load as smooth meshes.
    let mut old = serde_json::to_value(node(&s, e)).unwrap();
    old["kind"]["kind"].as_object_mut().unwrap().remove("handles");
    let n: Node = serde_json::from_value(old).unwrap();
    assert!(matches!(n.kind, NodeKind::Envelope { kind, .. } if matches!(&*kind, EnvelopeKind::Mesh { handles, .. } if handles.is_empty())));
}

#[test]
fn reset_with_mesh_keeps_the_warp_shape_along_the_grid() {
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithWarp", json!({"style": "arc", "bend": 60}));
    let warp = EnvelopeMap::of(&node(&s, e)).unwrap().surface_mesh(2, 2, vectorcraft_color::Color::BLACK);
    s.execute("object.envelope.resetWithMesh", &json!({"rows": 2, "cols": 2})).unwrap();
    let (_, _, points, handles) = mesh(&s, e);
    for (q, (p, h)) in warp.points.iter().zip(points.iter().zip(&handles)) {
        assert!(q.p.distance(*p) < 1e-9 && q.handles == *h);
    }
}
