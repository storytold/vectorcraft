//! Smart Pen's commands: each one undo step, bad params refused, and the tool's gestures through
//! the session.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::{AnchorKind, PathData, Point, Vec2};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::{Session, ViewInfo};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn line(s: &mut Session, pts: &[(f64, f64)]) -> NodeId {
    let anchors: Vec<Value> = pts.iter().map(|(x, y)| json!({"x": x, "y": y})).collect();
    NodeId(s.execute("path.create", &json!({"anchors": anchors, "closed": false})).unwrap()["id"].as_u64().unwrap())
}

fn path(s: &Session, id: NodeId) -> PathData {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().clone()
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

fn parallel(a: Vec2, b: Vec2) -> bool {
    a.cross(b).abs() < 1e-9 && a.dot(b) > 0.0
}

#[test]
fn append_adds_a_straight_or_curved_segment() {
    let mut s = session();
    let id = line(&mut s, &[(0.0, 100.0), (100.0, 100.0)]);
    let n = steps(&s);
    let r = s.execute("addon.pen.append", &json!({"id": id.0, "x": 150, "y": 150, "segment": "curved"})).unwrap();
    assert_eq!(r["anchor"], json!(2));
    assert_eq!(steps(&s), n + 1, "one undo step");
    let sp = &path(&s, id).subpaths[0];
    // The curve leaves (100, 100) along the straight segment before it.
    let a = sp.anchors[1];
    assert!(a.has_out() && !a.has_in() && parallel(a.h_out - a.p, Vec2::new(1.0, 0.0)), "{a:?}");
    assert!(sp.segment_is_line(0) && !sp.segment_is_line(1));
    // The new anchor is the one selected.
    let sel = &s.doc().unwrap().selection;
    assert_eq!(sel.anchors.get(&id), Some(&BTreeSet::from([(0, 2)])));
    // Straight: the previous anchor's handle on it goes, and the new anchor's incoming one.
    s.execute("addon.pen.append", &json!({"id": id.0, "subpath": 0, "x": 250, "y": 150, "in": [200, 100], "out": [300, 200], "segment": "straight"}))
        .unwrap();
    let sp = &path(&s, id).subpaths[0];
    assert!(sp.segment_is_line(2) && sp.anchors[3].has_out() && !sp.anchors[3].has_in());
    assert_eq!(sp.anchors[3].kind, AnchorKind::Corner);
    // Without a segment, the handles as given.
    s.execute("addon.pen.append", &json!({"id": id.0, "x": 350, "y": 150, "in": [330, 140], "out": [370, 160]})).unwrap();
    assert_eq!(path(&s, id).subpaths[0].anchors[4].kind, AnchorKind::Smooth);
    let closed = line(&mut s, &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]);
    s.execute("path.close", &json!({"id": closed.0})).unwrap();
    refuses(
        &mut s,
        "addon.pen.append",
        &[
            json!({}),
            json!({"id": id.0}),
            json!({"id": id.0, "x": 1, "y": f64::MAX}),
            json!({"id": id.0, "x": 1, "y": 1, "segment": "wavy"}),
            json!({"id": id.0, "x": 1, "y": 1, "in": [1]}),
            json!({"id": id.0, "x": 1, "y": 1, "subpath": 3}),
            json!({"id": closed.0, "x": 1, "y": 1}),
            json!({"id": closed.0, "x": 1, "y": 1, "subpath": 0}),
            json!({"id": 999_999, "x": 1, "y": 1}),
        ],
    );
}

#[test]
fn close_makes_the_closing_segment_straight_or_curved() {
    let mut s = session();
    let id = line(&mut s, &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)]);
    let n = steps(&s);
    s.execute("addon.pen.close", &json!({"id": id.0, "segment": "curved"})).unwrap();
    let sp = &path(&s, id).subpaths[0];
    assert!(sp.closed && !sp.segment_is_line(2));
    assert_eq!(steps(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("addon.pen.close", &json!({"id": id.0, "segment": "straight"})).unwrap();
    assert!(path(&s, id).subpaths[0].segment_is_line(2));
    let dot = line(&mut s, &[(5.0, 5.0)]);
    refuses(&mut s, "addon.pen.close", &[json!({}), json!({"id": id.0}), json!({"id": dot.0}), json!({"id": id.0, "segment": 1})]);
}

#[test]
fn segment_switches_between_straight_and_curved() {
    let mut s = session();
    let id = line(&mut s, &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)]);
    let n = steps(&s);
    let r = s.execute("addon.pen.segment", &json!({"id": id.0, "segment": 1, "curved": true})).unwrap();
    assert_eq!(r["changed"], json!(true));
    assert!(!path(&s, id).subpaths[0].segment_is_line(1));
    s.execute("addon.pen.segment", &json!({"id": id.0, "segment": 1, "curved": false})).unwrap();
    assert!(path(&s, id).subpaths[0].segment_is_line(1));
    assert_eq!(steps(&s), n + 2);
    // Already straight: no step.
    let r = s.execute("addon.pen.segment", &json!({"id": id.0, "segment": 1, "curved": false})).unwrap();
    assert_eq!((r["changed"].clone(), steps(&s)), (json!(false), n + 2));
    refuses(
        &mut s,
        "addon.pen.segment",
        &[
            json!({"id": id.0, "segment": 1}),
            json!({"id": id.0, "segment": 2, "curved": true}),
            json!({"id": id.0, "curved": true}),
            json!({"id": id.0, "segment": -1, "curved": true}),
        ],
    );
}

#[test]
fn remove_point_keeps_the_shape_or_deletes_plainly() {
    let mut s = session();
    let e = NodeId(s.execute("shape.ellipse", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.path.addAnchorPoints", &json!({})).unwrap();
    let before = path(&s, e);
    let n = steps(&s);
    s.execute("addon.pen.removePoint", &json!({"id": e.0, "anchor": 1})).unwrap();
    let after = path(&s, e);
    assert_eq!(after.anchor_count(), 7);
    assert_eq!(steps(&s), n + 1);
    // The outline barely moved: its bounds are the circle's.
    let (b0, b1) = (before.bounds().unwrap(), after.bounds().unwrap());
    assert!((b0.width() - b1.width()).abs() < 2.0 && (b0.height() - b1.height()).abs() < 2.0, "{b0:?} {b1:?}");
    // Plain: the handles beside it stay as they were.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("addon.pen.removePoint", &json!({"id": e.0, "anchor": 1, "plain": true})).unwrap();
    let plain = path(&s, e);
    assert_eq!(
        (plain.subpaths[0].anchors[0].h_out, plain.subpaths[0].anchors[1].h_in),
        (before.subpaths[0].anchors[0].h_out, before.subpaths[0].anchors[2].h_in)
    );
    // An open end goes with its segment; two anchors stay.
    let l = line(&mut s, &[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)]);
    s.execute("addon.pen.removePoint", &json!({"id": l.0, "anchor": 2})).unwrap();
    assert_eq!(path(&s, l).subpaths[0].anchors.len(), 2);
    refuses(
        &mut s,
        "addon.pen.removePoint",
        &[
            json!({"id": l.0, "anchor": 0}),
            json!({"id": l.0, "anchor": 0, "plain": true}),
            json!({"id": l.0, "anchor": 5}),
            json!({"id": l.0}),
            json!({"id": l.0, "anchor": 1, "plain": 1}),
        ],
    );
}

#[test]
fn toggle_point_and_reshape() {
    let mut s = session();
    let id = line(&mut s, &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)]);
    let n = steps(&s);
    assert_eq!(s.execute("addon.pen.togglePoint", &json!({"id": id.0, "anchor": 1})).unwrap()["smooth"], json!(true));
    let smooth = path(&s, id).subpaths[0].anchors[1];
    assert_eq!(s.execute("addon.pen.togglePoint", &json!({"id": id.0, "anchor": 1})).unwrap()["smooth"], json!(false));
    let corner = path(&s, id).subpaths[0].anchors[1];
    assert_eq!((corner.h_in, corner.h_out), (smooth.h_in, smooth.h_out), "nothing moved");
    assert_eq!(steps(&s), n + 2);
    refuses(&mut s, "addon.pen.togglePoint", &[json!({"id": id.0, "anchor": 9}), json!({"id": id.0, "anchor": 1, "ratio": 2})]);
    // The grabbed point follows the drag.
    let l = line(&mut s, &[(0.0, 200.0), (100.0, 200.0)]);
    for mode in ["free", "keepAngles", "symmetric"] {
        s.execute("addon.pen.reshape", &json!({"id": l.0, "segment": 0, "t": 0.5, "dx": 0, "dy": -40, "mode": mode})).unwrap();
        let p = kurbo::ParamCurve::eval(&path(&s, l).subpaths[0].segment(0), 0.5);
        assert!(p.distance(Point::new(50.0, 160.0)) < 1e-6, "{mode}: {p:?}");
        s.execute("edit.undo", &json!({})).unwrap();
    }
    refuses(
        &mut s,
        "addon.pen.reshape",
        &[
            json!({"id": l.0, "segment": 0}),
            json!({"id": l.0, "segment": 1, "dx": 1, "dy": 1}),
            json!({"id": l.0, "segment": 0, "dx": 1, "dy": 1, "t": 2}),
            json!({"id": l.0, "segment": 0, "dx": 1e300, "dy": 1}),
            json!({"id": l.0, "segment": 0, "dx": 1, "dy": 1, "mode": "twist"}),
        ],
    );
}

#[test]
fn the_panel_commands_set_the_next_segment_and_close_or_join() {
    let mut s = session();
    assert_eq!(s.execute("addon.pen.nextSegment", &json!({"curved": true})).unwrap()["next"], json!("curved"));
    assert_eq!(s.tool_options_of("smartPen")["next"], json!("curved"));
    assert!(s.execute("addon.pen.nextSegment", &json!({})).is_err());
    // Two open ends direct-selected: joined.
    let a = line(&mut s, &[(0.0, 0.0), (100.0, 0.0)]);
    let b = line(&mut s, &[(110.0, 0.0), (200.0, 0.0)]);
    s.select(|_, sel| {
        sel.set([a, b]);
        sel.anchors.insert(a, BTreeSet::from([(0, 1)]));
        sel.anchors.insert(b, BTreeSet::from([(0, 0)]));
    })
    .unwrap();
    let n = steps(&s);
    s.execute("addon.pen.closeJoin", &json!({})).unwrap();
    assert_eq!(steps(&s), n + 1);
    assert!(s.doc().unwrap().doc.node(b).is_none() || s.doc().unwrap().doc.node(a).is_none(), "one path is left");
    // Otherwise the selected open paths close.
    let c = line(&mut s, &[(0.0, 100.0), (100.0, 100.0), (100.0, 200.0)]);
    s.execute("addon.pen.closeJoin", &json!({})).unwrap();
    assert!(path(&s, c).subpaths[0].closed);
}

fn view() -> ViewInfo {
    ViewInfo { smart_guides: false, ..ViewInfo::default() }
}

/// A gesture of the active tool, `m` held throughout: press at the first point, drag through the
/// others, release at the last.
fn gesture(s: &mut Session, pts: &[(f64, f64)], m: Mods) {
    let mut kinds = vec![PointerKind::Down];
    kinds.extend(pts.iter().skip(1).map(|_| PointerKind::Drag));
    for (k, (x, y)) in kinds.into_iter().zip(pts.iter().copied()) {
        s.pointer(&PointerEvent::new(k, x, y).with_mods(m), view()).unwrap();
    }
    let (x, y) = pts.last().copied().unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, x, y).with_mods(m), view()).unwrap();
}

/// The one path of the session's document.
fn only_path(s: &Session) -> (NodeId, PathData) {
    let mut found = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if let Some(p) = n.path_data() {
            found.push((n.id, p.clone()));
        }
    });
    assert_eq!(found.len(), 1, "{found:?}");
    found.remove(0)
}

#[test]
fn the_tool_draws_a_path_one_step_per_anchor_and_closes_it() {
    let mut s = session();
    s.select_tool("smartPen", view()).unwrap();
    let n = steps(&s);
    gesture(&mut s, &[(50.0, 50.0)], Mods::default());
    gesture(&mut s, &[(150.0, 50.0)], Mods::default());
    s.execute("addon.pen.nextSegment", &json!({"curved": true})).unwrap();
    gesture(&mut s, &[(150.0, 150.0)], Mods::default());
    let (id, p) = only_path(&s);
    let sp = &p.subpaths[0];
    assert_eq!(sp.anchors.len(), 3);
    assert!(sp.segment_is_line(0) && !sp.segment_is_line(1), "a straight segment, then a curved one leaving it tangent");
    assert!(parallel(sp.anchors[1].h_out - sp.anchors[1].p, Vec2::new(1.0, 0.0)));
    // A drag makes a smooth anchor.
    gesture(&mut s, &[(50.0, 150.0), (30.0, 170.0)], Mods::default());
    assert_eq!(path(&s, id).subpaths[0].anchors[3].kind, AnchorKind::Smooth);
    // Tab between anchors: the last segment straight again.
    s.tool_key(ToolKey::Tab, Mods::default(), view()).unwrap();
    assert!(path(&s, id).subpaths[0].segment_is_line(2));
    gesture(&mut s, &[(50.0, 50.0)], Mods::default());
    assert!(path(&s, id).subpaths[0].closed);
    assert_eq!(steps(&s), n + 6, "one step per anchor, the Tab and the close");
    // Edits with the same tool: a drag on an anchor moves it, Alt-click removes one keeping the shape.
    s.select(|_, sel| sel.set([id])).unwrap();
    gesture(&mut s, &[(150.0, 50.0), (160.0, 40.0)], Mods::default());
    assert_eq!(path(&s, id).subpaths[0].anchors[1].p, Point::new(160.0, 40.0));
    gesture(&mut s, &[(160.0, 40.0)], Mods { alt: true, ..Mods::default() });
    assert_eq!(path(&s, id).subpaths[0].anchors.len(), 3);
    assert_eq!(steps(&s), n + 8);
}
