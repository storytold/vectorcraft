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

/// The kind of envelope `e`.
fn kind(s: &Session, e: NodeId) -> EnvelopeKind {
    match node(s, e).kind {
        NodeKind::Envelope { kind, .. } => *kind,
        k => panic!("{k:?}"),
    }
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// Moving a point of a warp envelope makes it a mesh envelope on the warp's grid with that point
/// moved, as one undo step: the other points and every handle stay where the warp had them, and
/// the shape away from the moved point stays the warp's. Undo gives back the warp, Redo the edit.
#[test]
fn moving_a_warp_point_makes_it_a_mesh_envelope_in_one_step() {
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithWarp", json!({"style": "arc", "bend": 50}));
    let warp = node(&s, e);
    let grid = live::warp_grid(&warp).unwrap();
    assert_eq!((grid.rows, grid.cols), (4, 4));
    let before = EnvelopeMap::of(&warp).unwrap();
    let n = undo_len(&s);
    // An interior point.
    s.execute("object.mesh.movePoint", &json!({"id": e.0, "index": 6, "x": 160, "y": 120})).unwrap();
    assert_eq!(undo_len(&s), n + 1, "one undo step");
    let (rows, cols, points, handles) = mesh(&s, e);
    assert_eq!((rows, cols, points.len(), handles.len()), (4, 4, 25, 25));
    for (i, q) in grid.points.iter().enumerate() {
        let want = if i == 6 { Point::new(160.0, 120.0) } else { q.p };
        assert!(points[i].distance(want) < 1e-9, "point {i}: {:?} vs {want:?}", points[i]);
        assert_eq!(handles[i], q.handles, "handles of point {i}");
    }
    let moved = node(&s, e);
    let after = EnvelopeMap::of(&moved).unwrap();
    let b = before.src();
    let tol = b.width().hypot(b.height()) * 1e-3;
    for (u, v) in [(0.9, 0.9), (0.6, 0.875), (1.0, 0.5), (0.0, 1.0)] {
        assert!(after.at(u, v).distance(before.at(u, v)) <= tol, "({u}, {v}): {:?} vs {:?}", after.at(u, v), before.at(u, v));
    }
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, e), warp, "Undo gives back the warp, style and bend included");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(mesh(&s, e).2[6], Point::new(160.0, 120.0));
    // A corner of a warp, and a handle with object.envelope.setMeshPoint.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("object.mesh.movePoints", &json!({"id": e.0, "points": [0], "dx": -10, "dy": -5})).unwrap();
    assert!(mesh(&s, e).2[0].distance(grid.points[0].p + Vec2::new(-10.0, -5.0)) < 1e-9);
    s.execute("edit.undo", &json!({})).unwrap();
    let end = grid.points[12].p + Vec2::new(30.0, -30.0);
    s.execute("object.envelope.setMeshPoint", &json!({"id": e.0, "index": 12, "x": end.x, "y": end.y, "handle": 0})).unwrap();
    let (_, _, points, handles) = mesh(&s, e);
    assert!((points[12] + handles[12][live::H_RIGHT]).distance(end) < 1e-9);
    assert_eq!(handles[12][live::H_LEFT], grid.points[12].handles[live::H_LEFT]);
    assert_eq!(undo_len(&s), n + 1);
}

/// After a warp's point is edited, the envelope keeps its content, options and fidelity; it saves
/// and opens again as it is; Reset with Mesh with Maintain Shape keeps the edited shape and Reset
/// with Warp gives the chosen style; Release gives back the edited mesh as its shape and Expand
/// keeps the edited distortion.
#[test]
fn an_edited_warp_saves_resets_releases_and_expands_as_edited() {
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithWarp", json!({"style": "flag", "bend": 40}));
    s.execute("object.envelope.options", &json!({"fidelity": 80, "distortAppearance": false})).unwrap();
    let warp = node(&s, e);
    // The middle of the bottom edge, 30 down.
    s.execute("object.mesh.movePoints", &json!({"id": e.0, "points": [22], "dx": 0, "dy": 30})).unwrap();
    let edited = node(&s, e);
    let (NodeKind::Envelope { content: c0, options: o0, fidelity: f0, .. }, NodeKind::Envelope { content: c1, options: o1, fidelity: f1, .. }) =
        (&warp.kind, &edited.kind)
    else {
        panic!("envelopes")
    };
    assert_eq!((c0, o0, f0), (c1, o1, f1), "content, options and fidelity stay");
    let d = s.doc().unwrap().doc.clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&d, false)).unwrap();
    assert_eq!(back.node(e), d.node(e), "a save opens again as edited");
    // Reset with Mesh, keeping the shape: the edited points.
    let (_, _, points, _) = mesh(&s, e);
    s.execute("object.envelope.resetWithMesh", &json!({"maintainShape": true})).unwrap();
    let (rows, cols, kept, _) = mesh(&s, e);
    assert_eq!((rows, cols), (4, 4));
    for (p, q) in points.iter().zip(&kept) {
        assert!(p.distance(*q) < 1e-6, "{p:?} vs {q:?}");
    }
    // Reset with Warp: the chosen style replaces the edit.
    s.execute("object.envelope.resetWithWarp", &json!({"style": "bulge", "bend": 30})).unwrap();
    assert!(matches!(kind(&s, e), EnvelopeKind::Warp { style, bend, .. } if style == "bulge" && bend == 30.0));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, e), edited);
    // Release: the content and the edited mesh as the envelope's shape.
    s.execute("select.set", &json!({"ids": [e.0]})).unwrap();
    let ids = s.execute("object.envelope.release", &json!({})).unwrap();
    let ids: Vec<NodeId> = ids["ids"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect();
    let shape = ids.iter().find_map(|id| match node(&s, *id).kind {
        NodeKind::Mesh(m) => Some(m),
        _ => None,
    });
    let shape = shape.expect("the envelope's shape");
    assert_eq!(shape.points.iter().map(|q| q.p).collect::<Vec<_>>(), points);
    s.execute("edit.undo", &json!({})).unwrap();
    // Expand: the distorted content follows the moved point, unlike the warp's.
    let expanded = |s: &mut Session| {
        s.execute("select.set", &json!({"ids": [e.0]})).unwrap();
        s.execute("object.envelope.expand", &json!({})).unwrap();
        let b = node(s, e).geometric_bounds().unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        b
    };
    let edited_bounds = expanded(&mut s);
    s.execute("edit.undo", &json!({})).unwrap();
    let warp_bounds = expanded(&mut s);
    assert!(edited_bounds.y1 > warp_bounds.y1 + 5.0, "{edited_bounds:?} vs {warp_bounds:?}");
}

/// An edit of a warp's points that fails (an index out of range, here on the second of two
/// warps) changes neither warp and adds no undo step; a warp that no grid follows closely enough
/// shows no points and is refused, and so is a top object envelope.
#[test]
fn a_warp_point_edit_that_fails_changes_nothing() {
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithWarp", json!({"style": "arc", "bend": 50}));
    let (_, f) = enveloped(&mut s, "object.envelope.makeWithWarp", json!({"style": "wave", "bend": 50}));
    let (we, wf, n) = (node(&s, e), node(&s, f), undo_len(&s));
    let r = s.execute("object.mesh.movePoints", &json!({"items": [{"id": e.0, "points": [0]}, {"id": f.0, "points": [3, 99999]}], "dx": 5, "dy": 5}));
    assert!(r.is_err(), "{r:?}");
    let r = s.execute("object.envelope.setMeshPoint", &json!({"id": e.0, "index": 25, "x": 0, "y": 0}));
    assert!(r.is_err(), "{r:?}");
    assert_eq!((node(&s, e), node(&s, f), undo_len(&s)), (we, wf, n), "nothing changed");
    // The old point map's fisheye: no grid of up to 16×16 patches follows it.
    let (_, g) = enveloped(&mut s, "object.envelope.makeWithWarp", json!({"style": "fisheye", "bend": 100, "version": 0}));
    let old = node(&s, g);
    assert!(live::warp_grid(&old).is_none());
    assert!(live::envelope_overlay(&old).unwrap().1.is_none(), "no points show");
    let n = undo_len(&s);
    for (cmd, p) in [
        ("object.mesh.movePoint", json!({"id": g.0, "index": 0, "x": 0, "y": 0})),
        ("object.envelope.setMeshPoint", json!({"id": g.0, "index": 0, "x": 0, "y": 0})),
        ("object.mesh.addLine", json!({"id": g.0, "x": 200, "y": 150})),
    ] {
        let r = s.execute(cmd, &p);
        assert!(
            matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg == "this warp's points can't be edited: use Reset with Mesh first"),
            "{cmd}: {r:?}"
        );
    }
    assert_eq!((node(&s, g), undo_len(&s)), (old, n));
}

/// An edit that moves no point of a warp (no points named, a move by nothing, a point placed where
/// it is, a line added through one of its points) leaves it a warp and records no undo step.
#[test]
fn a_warp_edit_that_moves_nothing_leaves_the_warp() {
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithWarp", json!({"style": "arc", "bend": 50}));
    let (warp, n) = (node(&s, e), undo_len(&s));
    let grid = live::warp_grid(&warp).unwrap();
    s.execute("object.mesh.movePoints", &json!({"id": e.0, "points": [], "dx": 10, "dy": 0})).unwrap();
    s.execute("object.mesh.movePoints", &json!({"id": e.0, "points": [3, 6], "dx": 0, "dy": 0})).unwrap();
    assert_eq!((node(&s, e), undo_len(&s)), (warp.clone(), n));
    let q = grid.points[6].clone();
    s.execute("object.mesh.movePoint", &json!({"id": e.0, "index": 6, "x": q.p.x, "y": q.p.y})).unwrap();
    s.execute("object.envelope.setMeshPoint", &json!({"id": e.0, "index": 6, "x": q.p.x, "y": q.p.y})).unwrap();
    // A line added through a point of the warp's grid adds none.
    let r = s.execute("object.mesh.addLine", &json!({"id": e.0, "x": q.p.x, "y": q.p.y})).unwrap();
    assert_eq!(r["index"], json!(6));
    assert_eq!((node(&s, e), undo_len(&s)), (warp, n));
    // An unknown mesh still fails.
    assert!(s.execute("object.mesh.movePoints", &json!({"id": 99999, "points": [], "dx": 1, "dy": 0})).is_err());
}

/// A handle placed at its own end, sent as point plus handle, leaves a warp a warp, even where
/// `(p + v) - p` rounds to a value other than `v`.
#[test]
fn a_warp_handle_placed_at_its_end_leaves_the_warp() {
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithWarp", json!({"style": "arc", "bend": 50}));
    let (warp, n) = (node(&s, e), undo_len(&s));
    let grid = live::warp_grid(&warp).unwrap();
    let mut rounds = 0;
    for (index, pt) in grid.points.iter().enumerate() {
        for (h, v) in pt.handles.iter().enumerate() {
            let end = pt.p + *v;
            if end - pt.p != *v {
                rounds += 1;
            }
            for cmd in ["object.mesh.movePoint", "object.envelope.setMeshPoint"] {
                s.execute(cmd, &json!({"id": e.0, "index": index, "x": end.x, "y": end.y, "handle": h})).unwrap();
                assert_eq!((node(&s, e), undo_len(&s)), (warp.clone(), n), "{cmd}: point {index}, handle {h}");
            }
        }
    }
    assert!(rounds > 0, "the grid has a handle whose end does not give the handle back");
}

/// With Direct Selection, a selected warp's point drags as `object.mesh.movePoint`: the preview
/// makes it a mesh envelope, Escape gives back the warp with no undo step, and a finished drag is
/// one undo step. While its contents are edited, its points don't take the press.
#[test]
fn direct_selection_drags_a_warps_point_and_escape_gives_it_back() {
    let mut s = session();
    let (_, e) = enveloped(&mut s, "object.envelope.makeWithWarp", json!({"style": "arc", "bend": 50}));
    let warp = node(&s, e);
    let corner = live::warp_grid(&warp).unwrap().points[24].p;
    s.execute("select.set", &json!({"ids": [e.0]})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("directSelection", v).unwrap();
    let n = undo_len(&s);
    s.pointer(&PointerEvent::new(PointerKind::Down, corner.x, corner.y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, corner.x + 20.0, corner.y + 10.0), v).unwrap();
    assert!(mesh(&s, e).2[24].distance(corner + Vec2::new(20.0, 10.0)) < 1e-9, "the preview moves the point");
    s.tool_key(vectorcraft_tools::ToolKey::Escape, vectorcraft_tools::Mods::default(), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, corner.x + 20.0, corner.y + 10.0), v).unwrap();
    assert_eq!((node(&s, e), undo_len(&s)), (warp.clone(), n), "Escape gives back the warp");
    s.pointer(&PointerEvent::new(PointerKind::Down, corner.x, corner.y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, corner.x + 20.0, corner.y + 10.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, corner.x + 20.0, corner.y + 10.0), v).unwrap();
    assert_eq!(undo_len(&s), n + 1);
    assert!(mesh(&s, e).2[24].distance(corner + Vec2::new(20.0, 10.0)) < 1e-9);
    // Edit Contents: the press doesn't take the envelope's point.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("object.envelope.editContents", &json!({"editing": true})).unwrap();
    s.execute("select.set", &json!({"ids": [e.0]})).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, corner.x, corner.y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, corner.x + 20.0, corner.y + 10.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, corner.x + 20.0, corner.y + 10.0), v).unwrap();
    assert!(matches!(kind(&s, e), EnvelopeKind::Warp { .. }), "{:?}", kind(&s, e));
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
