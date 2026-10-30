//! Path Precision Tools' commands: each one undo step, bad params refused, and the tools' gestures
//! through the session.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::{AnchorKind, PathData, Point};
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::{Session, ViewInfo};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn line(s: &mut Session, pts: &[(f64, f64)], closed: bool) -> NodeId {
    let anchors: Vec<Value> = pts.iter().map(|(x, y)| json!({"x": x, "y": y})).collect();
    id_of(&s.execute("path.create", &json!({"anchors": anchors, "closed": closed})).unwrap())
}

/// A circle of 16 anchors about (100, 100), radius 50, selected.
fn circle(s: &mut Session) -> NodeId {
    let e = id_of(&s.execute("shape.ellipse", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap());
    s.execute("object.path.addAnchorPoints", &json!({})).unwrap();
    s.execute("object.path.addAnchorPoints", &json!({})).unwrap();
    e
}

fn path(s: &Session, id: NodeId) -> PathData {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().clone()
}

fn select(s: &mut Session, ids: &[NodeId]) {
    s.select(|_, sel| sel.set(ids.iter().copied())).unwrap();
}

/// Direct-select anchors `anchors` (of subpath 0) of `id`.
fn select_anchors(s: &mut Session, id: NodeId, anchors: &[usize]) {
    s.select(|_, sel| {
        sel.set([id]);
        sel.anchors.insert(id, anchors.iter().map(|a| (0, *a)).collect::<BTreeSet<_>>());
    })
    .unwrap();
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

#[test]
fn the_brush_removes_anchors_under_it_keeping_the_shape() {
    let mut s = session();
    let e = circle(&mut s);
    assert_eq!(path(&s, e).anchor_count(), 16);
    let bounds = path(&s, e).bounds().unwrap();
    let n = steps(&s);
    let r = s.execute("addon.pathtools.erase", &json!({"points": [[150, 40], [150, 160]], "diameter": 60, "tolerance": 100})).unwrap();
    let left = path(&s, e).anchor_count();
    assert!(left < 16 && left > 8, "{left}");
    assert_eq!(r["removed"], json!(16 - left));
    assert_eq!(steps(&s), n + 1, "one undo step");
    let b = path(&s, e).bounds().unwrap();
    assert!((b.x1 - bounds.x1).abs() < 2.5 && (b.y0 - bounds.y0).abs() < 0.01, "{b:?} vs {bounds:?}");
    // The anchors away from the brush stay: those on the left of the circle.
    assert!(path(&s, e).anchors().filter(|(_, _, a)| a.p.x < 100.0).count() >= 7);
    // A plain delete takes every anchor under it, whatever the shape.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("addon.pathtools.erase", &json!({"points": [[150, 40], [150, 160]], "diameter": 60, "plain": true})).unwrap();
    assert!(path(&s, e).anchors().all(|(_, _, a)| a.p.x < 121.0));
    // Nothing under the brush: no step.
    let n = steps(&s);
    s.execute("addon.pathtools.erase", &json!({"points": [[390, 290]], "diameter": 5})).unwrap();
    assert_eq!(steps(&s), n);
    refuses(
        &mut s,
        "addon.pathtools.erase",
        &[
            json!({}),
            json!({"points": "x"}),
            json!({"points": [[1, "y"]]}),
            json!({"points": [[100, 100]], "diameter": 0}),
            json!({"points": [[100, 100]], "diameter": 3000}),
            json!({"points": [[100, 100]], "tolerance": 0}),
            json!({"points": [[100, 100]], "plain": "yes"}),
        ],
    );
}

#[test]
fn the_brush_tool_is_one_undo_step_per_stroke() {
    let mut s = session();
    let e = circle(&mut s);
    let view = ViewInfo { smart_guides: false, ..ViewInfo::default() };
    s.select_tool("anchorEraserBrush", view).unwrap();
    assert_eq!(s.tool_id(), "anchorEraserBrush");
    s.set_tool_option("size", &json!(60));
    s.set_tool_option("tolerance", &json!(100));
    let n = steps(&s);
    for (k, y) in
        [(PointerKind::Down, 40.0), (PointerKind::Drag, 80.0), (PointerKind::Drag, 120.0), (PointerKind::Drag, 160.0), (PointerKind::Up, 160.0)]
    {
        s.pointer(&PointerEvent::new(k, 150.0, y), view).unwrap();
    }
    assert_eq!(steps(&s), n + 1);
    assert!(path(&s, e).anchor_count() < 16);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(path(&s, e).anchor_count(), 16);
}

#[test]
fn extend_lengthens_and_shortens_an_end() {
    let mut s = session();
    let l = line(&mut s, &[(0.0, 100.0), (100.0, 100.0)], false);
    let n = steps(&s);
    s.execute("addon.pathtools.extend", &json!({"id": l.0, "end": "end", "length": 50})).unwrap();
    assert_eq!(path(&s, l).subpaths[0].anchors.last().unwrap().p, Point::new(150.0, 100.0));
    assert_eq!(steps(&s), n + 1);
    s.execute("addon.pathtools.extend", &json!({"id": l.0, "end": "start", "to": [-30, 120], "mode": "bezier"})).unwrap();
    assert!((path(&s, l).subpaths[0].anchors[0].p.x + 30.0).abs() < 1e-6);
    let r = s.execute("addon.pathtools.extend", &json!({"id": l.0, "end": "end", "length": -60})).unwrap();
    assert_eq!(r["length"], json!(-60.0));
    assert!((path(&s, l).length() - 120.0).abs() < 1e-3, "{}", path(&s, l).length());
    assert_eq!(steps(&s), n + 3);
    let sq = id_of(&s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap());
    refuses(
        &mut s,
        "addon.pathtools.extend",
        &[
            json!({"id": l.0, "end": "end"}),
            json!({"id": l.0, "end": "middle", "length": 5}),
            json!({"id": l.0, "end": "end", "length": 5, "to": [1, 1]}),
            json!({"id": l.0, "end": "end", "length": 5, "mode": "spiral"}),
            json!({"id": l.0, "end": "end", "length": -1000}),
            json!({"id": l.0, "end": "end", "length": 1e9}),
            json!({"id": l.0, "end": "end", "to": [1]}),
            json!({"id": l.0, "subpath": 3, "end": "end", "length": 5}),
            json!({"id": sq.0, "end": "end", "length": 5}),
            json!({"id": 99999, "end": "end", "length": 5}),
        ],
    );
}

#[test]
fn inspect_reads_the_path_under_a_point() {
    let mut s = session();
    let r = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap());
    let v = s.execute("addon.pathtools.inspect", &json!({"id": r.0, "x": 99, "y": 1})).unwrap();
    assert_eq!((v["closed"].clone(), v["anchors"].clone()), (json!(true), json!(4)));
    assert!((v["length"].as_f64().unwrap() - 300.0).abs() < 1e-6 && (v["area"].as_f64().unwrap() - 5000.0).abs() < 1e-6);
    assert!((v["anchor"]["angle"].as_f64().unwrap() - 90.0).abs() < 1e-9);
    assert!(v["radius"].is_null());
    assert!(s.execute("addon.pathtools.inspect", &json!({"id": r.0, "x": 1})).is_err());
    assert!(s.execute("addon.pathtools.inspect", &json!({"id": r.0, "x": 1, "y": 1, "tolerance": -1})).is_err());
}

#[test]
fn smooth_corner_and_equalize_edit_the_selected_anchors_in_one_step_each() {
    let mut s = session();
    let r = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    select(&mut s, &[r]);
    let n = steps(&s);
    assert_eq!(s.execute("addon.pathtools.smooth", &json!({})).unwrap()["anchors"], json!(4));
    assert!(path(&s, r).anchors().all(|(_, _, a)| a.kind == AnchorKind::Smooth && (a.h_out - a.p).hypot() > 39.0));
    assert_eq!(steps(&s), n + 1);
    // Only the direct-selected anchor.
    select_anchors(&mut s, r, &[0]);
    s.execute("addon.pathtools.corner", &json!({})).unwrap();
    let p = path(&s, r);
    assert!(!p.subpaths[0].anchors[0].has_out() && p.subpaths[0].anchors[1].has_out());
    assert_eq!(steps(&s), n + 2);
    // Equalise: one handle made to match the other.
    select_anchors(&mut s, r, &[1]);
    s.execute("addon.pathtools.setAnchor", &json!({"outLength": 10})).unwrap();
    let a = path(&s, r).subpaths[0].anchors[1];
    assert!(((a.h_out - a.p).hypot() - 10.0).abs() < 1e-9 && (a.h_in - a.p).hypot() > 39.0);
    s.execute("addon.pathtools.equalize", &json!({})).unwrap();
    let a = path(&s, r).subpaths[0].anchors[1];
    assert!(((a.h_out - a.p).hypot() - (a.h_in - a.p).hypot()).abs() < 1e-9);
    // A selection with nothing to change makes no step.
    let n = steps(&s);
    assert_eq!(s.execute("addon.pathtools.equalize", &json!({})).unwrap()["anchors"], json!(0));
    assert_eq!(steps(&s), n);
    refuses(&mut s, "addon.pathtools.smooth", &[json!({"ratio": 0.9}), json!({"ratio": "x"})]);
    refuses(&mut s, "addon.pathtools.corner", &[json!({"retract": 1})]);
}

#[test]
fn set_anchor_moves_anchors_and_sets_handles() {
    let mut s = session();
    let l = line(&mut s, &[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)], false);
    select_anchors(&mut s, l, &[1]);
    let n = steps(&s);
    s.execute("addon.pathtools.setAnchor", &json!({"x": 60, "y": 20, "outLength": 30, "outAngle": 90})).unwrap();
    let a = path(&s, l).subpaths[0].anchors[1];
    assert_eq!(a.p, Point::new(60.0, 20.0));
    assert!(a.h_out.distance(Point::new(60.0, -10.0)) < 1e-9, "{a:?}");
    assert_eq!(steps(&s), n + 1);
    refuses(&mut s, "addon.pathtools.setAnchor", &[json!({}), json!({"x": "1"}), json!({"inLength": -2}), json!({"y": f64::MAX})]);
}

#[test]
fn remove_keep_shape_takes_the_direct_selected_anchors() {
    let mut s = session();
    let e = circle(&mut s);
    // Selected whole: nothing to say which anchors go.
    refuses(&mut s, "addon.pathtools.removeKeepShape", &[json!({})]);
    select_anchors(&mut s, e, &[1, 5]);
    let n = steps(&s);
    assert_eq!(s.execute("addon.pathtools.removeKeepShape", &json!({})).unwrap()["removed"], json!(2));
    assert_eq!(path(&s, e).anchor_count(), 14);
    assert_eq!(steps(&s), n + 1);
}

#[test]
fn add_points_splits_selected_segments_evenly() {
    let mut s = session();
    let l = line(&mut s, &[(0.0, 0.0), (100.0, 0.0)], false);
    select(&mut s, &[l]);
    let n = steps(&s);
    assert_eq!(s.execute("addon.pathtools.addPoints", &json!({"count": 3})).unwrap()["added"], json!(3));
    let xs: Vec<f64> = path(&s, l).anchors().map(|(_, _, a)| a.p.x).collect();
    assert!(xs.iter().zip([0.0, 25.0, 50.0, 75.0, 100.0]).all(|(x, w)| (x - w).abs() < 1e-3), "{xs:?}");
    assert_eq!(steps(&s), n + 1);
    refuses(&mut s, "addon.pathtools.addPoints", &[json!({"count": 0}), json!({"count": 101}), json!({"count": 1.5})]);
    // An anchor alone selects no segment.
    select_anchors(&mut s, l, &[1]);
    refuses(&mut s, "addon.pathtools.addPoints", &[json!({})]);
}

#[test]
fn close_joins_open_ends() {
    let mut s = session();
    let l = line(&mut s, &[(0.0, 0.0), (100.0, 0.0), (50.0, 80.0)], false);
    select(&mut s, &[l]);
    let n = steps(&s);
    assert_eq!(s.execute("addon.pathtools.close", &json!({"mode": "smooth"})).unwrap()["closed"], json!(1));
    assert!(path(&s, l).subpaths[0].closed);
    assert_eq!(steps(&s), n + 1);
    refuses(&mut s, "addon.pathtools.close", &[json!({}), json!({"mode": "round"})]);
}

#[test]
fn crossings_add_anchors_or_cut_in_one_step() {
    let mut s = session();
    let a = line(&mut s, &[(0.0, 0.0), (100.0, 100.0)], false);
    let b = line(&mut s, &[(0.0, 100.0), (100.0, 0.0)], false);
    select(&mut s, &[a, b]);
    let n = steps(&s);
    assert_eq!(s.execute("addon.pathtools.crossings", &json!({})).unwrap()["crossings"], json!(2));
    assert_eq!((path(&s, a).anchor_count(), path(&s, b).anchor_count()), (3, 3));
    assert!(path(&s, a).subpaths[0].anchors[1].p.distance(Point::new(50.0, 50.0)) < 1e-6);
    assert_eq!(steps(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    let r = s.execute("addon.pathtools.crossings", &json!({"cut": true})).unwrap();
    assert_eq!(r["ids"].as_array().unwrap().len(), 4, "{r}");
    assert_eq!(steps(&s), n + 1);
    refuses(&mut s, "addon.pathtools.crossings", &[json!({"cut": "yes"}), json!({"ids": [123456]})]);
}

// ---------- Parametric Shape Tool ----------

/// The parametric shape object `id` is now, as the tool reads it.
fn shape_of(s: &Session, id: NodeId) -> Option<vectorcraft_pathops::addons::pathtools::shapes::ParametricShape> {
    vectorcraft_tools::addons::parametric_shape::read(s.doc().unwrap().doc.node(id).unwrap())
}

fn star(s: &mut Session) -> NodeId {
    let r = s.execute("addon.pathtools.shape.create", &json!({"kind": "star", "center": [200, 150], "width": 100, "height": 100})).unwrap();
    id_of(&r)
}

#[test]
fn create_draws_a_selected_parametric_shape_in_one_step() {
    let mut s = session();
    let n = steps(&s);
    let r = s.execute("addon.pathtools.shape.create", &json!({"kind": "gear", "center": [200, 150], "count": 10, "a": 0.3, "radius": 2})).unwrap();
    let id = id_of(&r);
    assert_eq!(steps(&s), n + 1, "one undo step");
    assert_eq!((r["shape"]["kind"].clone(), r["shape"]["count"].clone(), r["shape"]["width"].clone()), (json!("gear"), json!(10), json!(100.0)));
    assert_eq!(s.doc().unwrap().selection.objects, [id]);
    let g = shape_of(&s, id).unwrap();
    assert_eq!((g.count, g.a, g.radius), (10, 0.3, 2.0));
    assert!(path(&s, id).anchor_count() > 40, "40 tooth corners, rounded");
    // Every kind, with its defaults.
    for kind in ["rectangle", "ellipse", "polygon", "star", "ring", "arrow", "cross", "heart", "gear", "bubble"] {
        let id = id_of(&s.execute("addon.pathtools.shape.create", &json!({"kind": kind, "center": [50, 50]})).unwrap());
        assert_eq!(shape_of(&s, id).map(|s| s.kind.id()), Some(kind));
    }
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(steps(&s), n + 10);
}

#[test]
fn set_changes_the_numbers_in_one_step_and_nothing_without_a_change() {
    let mut s = session();
    let id = star(&mut s);
    assert_eq!(path(&s, id).anchor_count(), 10);
    let n = steps(&s);
    let r = s.execute("addon.pathtools.shape.set", &json!({"id": id.0, "count": 8, "a": 0.3, "rotation": 15})).unwrap();
    assert_eq!((r["shape"]["count"].clone(), r["shape"]["rotation"].clone()), (json!(8), json!(15.0)));
    assert_eq!(path(&s, id).anchor_count(), 16);
    assert_eq!(steps(&s), n + 1);
    // The same numbers again, or none: no step.
    s.execute("addon.pathtools.shape.set", &json!({"id": id.0, "count": 8})).unwrap();
    s.execute("addon.pathtools.shape.set", &json!({"id": id.0})).unwrap();
    assert_eq!(steps(&s), n + 1);
    // Another kind takes its own defaults.
    let r = s.execute("addon.pathtools.shape.set", &json!({"id": id.0, "kind": "arrow"})).unwrap();
    assert_eq!((r["shape"]["a"].clone(), r["shape"]["rotation"].clone()), (json!(0.4), json!(15.0)));
    assert_eq!(path(&s, id).anchor_count(), 7);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(path(&s, id).anchor_count(), 16);
}

#[test]
fn bad_shape_params_are_refused() {
    let mut s = session();
    let id = star(&mut s);
    let rect = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    let c = json!([10, 10]);
    refuses(
        &mut s,
        "addon.pathtools.shape.create",
        &[
            json!({}),
            json!({"center": null}),
            json!({"center": [1]}),
            json!({"center": "here"}),
            json!({"center": [1e300, 0]}),
            json!({"center": c, "kind": "cloud"}),
            json!({"center": c, "kind": 3}),
            json!({"center": c, "count": 2}),
            json!({"center": c, "count": 1001}),
            json!({"center": c, "count": 4.5}),
            json!({"center": c, "width": 0}),
            json!({"center": c, "height": 16385}),
            json!({"center": c, "width": "40"}),
            json!({"center": c, "kind": "star", "a": 2}),
            json!({"center": c, "kind": "bubble", "b": -1.5}),
            json!({"center": c, "radius": -1}),
            json!({"center": c, "rotation": "up"}),
        ],
    );
    refuses(
        &mut s,
        "addon.pathtools.shape.set",
        &[json!({}), json!({"id": rect.0, "count": 6}), json!({"id": 999_999}), json!({"id": id.0, "count": 0}), json!({"id": id.0, "width": -5})],
    );
    s.execute("object.setProps", &json!({"ids": [id.0], "locked": true})).unwrap();
    refuses(&mut s, "addon.pathtools.shape.set", &[json!({"id": id.0, "count": 6})]);
}

#[test]
fn an_anchor_edit_makes_it_a_plain_path_and_transforms_keep_it_editable() {
    let mut s = session();
    let id = star(&mut s);
    // Moved, turned and scaled with the ordinary commands: still the shape, at its new place.
    s.execute("object.move", &json!({"dx": 20, "dy": -10})).unwrap();
    assert!(shape_of(&s, id).is_some_and(|s| s.center.distance(Point::new(220.0, 140.0)) < 1e-6));
    s.execute("object.rotate", &json!({"angle": 30, "origin": [220, 140]})).unwrap();
    let r = shape_of(&s, id).unwrap();
    assert!((r.rotation + 30.0).abs() < 1e-6, "30° counter-clockwise is −30 here: {r:?}");
    s.execute("object.scale", &json!({"sx": 200, "origin": [220, 140]})).unwrap();
    let r = shape_of(&s, id).unwrap();
    assert!((r.width - 200.0).abs() < 1e-6 && r.center.distance(Point::new(220.0, 140.0)) < 1e-6, "{r:?}");
    let n = steps(&s);
    s.execute("addon.pathtools.shape.set", &json!({"id": id.0, "count": 6})).unwrap();
    assert_eq!((path(&s, id).anchor_count(), steps(&s)), (12, n + 1));
    assert!((shape_of(&s, id).unwrap().rotation + 30.0).abs() < 1e-6, "the turn stays");
    // Stretched across its own axes (it is turned): skewed, so no longer.
    s.execute("object.scale", &json!({"sx": 150, "sy": 100, "origin": [220, 140]})).unwrap();
    assert_eq!(shape_of(&s, id), None);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(shape_of(&s, id).is_some(), "undo brings it back");
    // An anchor moved by hand: a plain path, which `set` refuses.
    select_anchors(&mut s, id, &[2]);
    s.execute("addon.pathtools.setAnchor", &json!({"x": 0})).unwrap();
    assert_eq!(shape_of(&s, id), None);
    refuses(&mut s, "addon.pathtools.shape.set", &[json!({"id": id.0, "count": 7})]);
}

#[test]
fn convert_makes_plain_shapes_parametric_in_one_step() {
    let mut s = session();
    let r = id_of(&s.execute("shape.rectangle", &json!({"x": 10, "y": 20, "width": 100, "height": 50})).unwrap());
    let p = id_of(&s.execute("shape.polygon", &json!({"cx": 200, "cy": 150, "radius": 40, "sides": 7})).unwrap());
    let st = id_of(&s.execute("shape.star", &json!({"cx": 300, "cy": 150, "radius1": 40, "radius2": 15})).unwrap());
    select(&mut s, &[r, p, st]);
    let n = steps(&s);
    let out = s.execute("addon.pathtools.shape.convert", &json!({})).unwrap();
    assert_eq!(out["ids"], json!([r.0, p.0]));
    assert_eq!(steps(&s), n + 1);
    assert_eq!(shape_of(&s, r).map(|s| s.kind.id()), Some("rectangle"));
    assert_eq!(shape_of(&s, p).map(|s| (s.kind.id(), s.count)), Some(("polygon", 7)));
    assert_eq!(shape_of(&s, st), None);
    assert_eq!(path(&s, r).bounds().map(|b| b.center()), Some(Point::new(60.0, 45.0)));
    // Nothing left to convert.
    select(&mut s, &[r, st]);
    refuses(&mut s, "addon.pathtools.shape.convert", &[json!({}), json!({"ids": [st.0]}), json!({"ids": [123456]})]);
}

#[test]
fn the_shape_data_stays_out_of_svg_export() {
    let mut s = session();
    let id = star(&mut s);
    s.execute("object.setProps", &json!({"ids": [id.0], "data": {"pivot": "1,2"}})).unwrap();
    let key = vectorcraft_tools::addons::parametric_shape::KEY;
    let node = s.doc().unwrap().doc.node(id).unwrap().clone();
    assert!(node.extras().attrs.as_deref().is_some_and(|a| a.data.iter().any(|(k, _)| k == key)), "kept with the object");
    assert!(shape_of(&s, id).is_some(), "setting other data keeps it a shape");
    let svg = vectorcraft_svg::export(&s.doc().unwrap().doc, &Default::default());
    assert!(svg.contains("data-pivot=\"1,2\""), "the object's own data still goes out: {svg}");
    assert!(!svg.contains(key) && !svg.contains("fp&quot;") && !svg.contains("\"fp\""), "{svg}");
}

#[test]
fn the_tool_draws_one_step_per_drag_and_arrows_change_the_count() {
    let mut s = session();
    let view = ViewInfo { smart_guides: false, ..ViewInfo::default() };
    s.select_tool("parametricShape", view).unwrap();
    s.set_tool_option("kind", &json!("star"));
    let n = steps(&s);
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 150.0), view).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 100.0), view).unwrap();
    s.tool_key(vectorcraft_tools::ToolKey::Up, vectorcraft_tools::Mods::default(), view).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 90.0), view).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 200.0, 90.0), view).unwrap();
    assert_eq!(steps(&s), n + 1);
    let id = *s.doc().unwrap().selection.objects.first().unwrap();
    let shape = shape_of(&s, id).unwrap();
    assert_eq!((shape.kind.id(), shape.count, shape.height, shape.rotation), ("star", 6, 120.0, 0.0));
    // Its size handle (lower right of its box) drags into one more step.
    let corner = shape.frame() * Point::new(shape.width / 2.0, shape.height / 2.0);
    for (k, x) in [(PointerKind::Down, corner.x), (PointerKind::Drag, corner.x + 30.0), (PointerKind::Up, corner.x + 30.0)] {
        s.pointer(&PointerEvent::new(k, x, corner.y), view).unwrap();
    }
    assert_eq!(steps(&s), n + 2);
    assert!((shape_of(&s, id).unwrap().width - 180.0).abs() < 1e-9);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(shape_of(&s, id).map(|s| s.width), Some(120.0));
}
