//! Drawing / path-editing tools driven through the session (tools → actions → `path.*` commands).

use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{FillRule, PathData, Point};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use super::*;
use crate::tooling::{UiRequest, ViewInfo};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn view() -> ViewInfo {
    ViewInfo { smart_guides: false, ..Default::default() }
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn line(s: &mut Session, x1: f64, y1: f64, x2: f64, y2: f64) -> NodeId {
    let r = s.execute("shape.line", &json!({"x1": x1, "y1": y1, "x2": x2, "y2": y2})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn paths(s: &Session) -> Vec<(NodeId, PathData)> {
    let mut v = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if let NodeKind::Path { path, .. } = &n.kind {
            v.push((n.id, path.clone()));
        }
    });
    v
}

fn path(s: &Session, id: NodeId) -> PathData {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().clone()
}

fn drag(s: &mut Session, tool: &str, pts: &[(f64, f64)], up_mods: Mods) -> Vec<UiRequest> {
    let v = view();
    s.select_tool(tool, v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, pts[0].0, pts[0].1), v).unwrap();
    for &(x, y) in &pts[1..] {
        s.pointer(&PointerEvent::new(PointerKind::Drag, x, y), v).unwrap();
    }
    let l = pts[pts.len() - 1];
    s.pointer(&PointerEvent::new(PointerKind::Up, l.0, l.1).with_mods(up_mods), v).unwrap()
}

fn click(s: &mut Session, tool: &str, x: f64, y: f64) -> Vec<UiRequest> {
    drag(s, tool, &[(x, y)], Mods::default())
}

/// A gesture of the active tool, `m` held throughout: press at the first point, drag through the
/// others, release at the last. The undo steps it made.
fn gesture(s: &mut Session, pts: &[(f64, f64)], m: Mods) -> usize {
    let v = view();
    let undo = undo_steps(s);
    s.pointer(&PointerEvent::new(PointerKind::Down, pts[0].0, pts[0].1).with_mods(m), v).unwrap();
    for &(x, y) in &pts[1..] {
        s.pointer(&PointerEvent::new(PointerKind::Drag, x, y).with_mods(m), v).unwrap();
    }
    let l = pts[pts.len() - 1];
    s.pointer(&PointerEvent::new(PointerKind::Up, l.0, l.1).with_mods(m), v).unwrap();
    undo_steps(s) - undo
}

/// A wavy stroke sampled every 2 pt.
fn wave(x0: f64, x1: f64, y: f64) -> Vec<(f64, f64)> {
    let n = ((x1 - x0) / 2.0) as usize;
    (0..=n).map(|i| x0 + i as f64 * 2.0).map(|x| (x, y + 20.0 * ((x - x0) / 40.0).sin())).collect()
}

fn near(a: Point, b: Point) -> bool {
    a.distance(b) < 1e-6
}

fn area(pd: &PathData) -> f64 {
    vectorcraft_pathops::area(pd, FillRule::NonZero)
}

#[test]
fn pencil_fits_freehand_stroke_as_one_undo_step() {
    let mut s = session();
    let pts = wave(100.0, 400.0, 200.0);
    let undo_before = s.doc().unwrap().history.undo.len();
    drag(&mut s, "pencil", &pts, Mods::default());
    let ps = paths(&s);
    assert_eq!(ps.len(), 1);
    let p = &ps[0].1;
    assert!(!p.is_closed());
    assert!(p.anchor_count() >= 2 && p.anchor_count() < pts.len() / 4, "{} anchors", p.anchor_count());
    let first = p.subpaths[0].anchors[0].p;
    assert!(first.distance(Point::new(100.0, 200.0)) < 1e-6);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_before + 1);
    // Unfilled, stroked.
    let n = s.doc().unwrap().doc.node(ps[0].0).unwrap().clone();
    assert!(n.appearance.fill_paint().is_none());
    assert!(!n.appearance.stroke_paint().is_none());
}

#[test]
fn pencil_alt_closes_path() {
    let mut s = session();
    let mut pts: Vec<(f64, f64)> =
        (0..=36).map(|i| (i as f64 * 10.0f64).to_radians()).map(|a| (300.0 + 80.0 * a.cos(), 300.0 + 80.0 * a.sin())).collect();
    pts.pop();
    drag(&mut s, "pencil", &pts, Mods { alt: true, ..Default::default() });
    let ps = paths(&s);
    assert!(ps[0].1.is_closed());
}

#[test]
fn pencil_continues_selected_open_path() {
    let mut s = session();
    let id = line(&mut s, 100.0, 100.0, 200.0, 100.0);
    drag(&mut s, "pencil", &[(202.0, 101.0), (230.0, 120.0), (260.0, 150.0), (300.0, 160.0)], Mods::default());
    let ps = paths(&s);
    assert_eq!(ps.len(), 1, "no new object");
    let p = path(&s, id);
    assert!(p.anchor_count() >= 3);
    let last = p.subpaths[0].anchors.last().unwrap().p;
    assert!(last.distance(Point::new(300.0, 160.0)) < 1e-6);
    assert_eq!(p.subpaths[0].anchors[0].p, Point::new(100.0, 100.0));
}

#[test]
fn paintbrush_uses_stroke_blob_brush_fills() {
    let mut s = session();
    drag(&mut s, "paintbrush", &wave(50.0, 200.0, 100.0), Mods::default());
    let n = s.doc().unwrap().doc.node(paths(&s)[0].0).unwrap().clone();
    assert!(n.appearance.fill_paint().is_none());
    s.execute("select.none", &json!({})).unwrap();
    drag(&mut s, "blobBrush", &[(100.0, 400.0), (200.0, 400.0)], Mods::default());
    let ps = paths(&s);
    assert_eq!(ps.len(), 2);
    let blob = &ps[1].1;
    assert!(blob.is_closed());
    // 100 × 10 capsule ≈ 1000 + π·25.
    assert!((area(blob) - (1000.0 + std::f64::consts::PI * 25.0)).abs() < 10.0, "{}", area(blob));
    let bn = s.doc().unwrap().doc.node(ps[1].0).unwrap().clone();
    assert!(bn.appearance.stroke_paint().is_none() && !bn.appearance.fill_paint().is_none());
    // A second overlapping stroke of the same colour merges.
    drag(&mut s, "blobBrush", &[(150.0, 380.0), (150.0, 450.0)], Mods::default());
    let ps = paths(&s);
    assert_eq!(ps.len(), 2, "merged into the first blob");
    assert!(area(&ps[1].1) > 1500.0);
}

#[test]
fn curvature_clicks_build_smooth_path() {
    let mut s = session();
    let v = view();
    s.select_tool("curvature", v).unwrap();
    for (x, y) in [(100.0, 300.0), (200.0, 200.0), (300.0, 300.0)] {
        s.pointer(&PointerEvent::new(PointerKind::Down, x, y), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, x, y), v).unwrap();
    }
    let ps = paths(&s);
    assert_eq!(ps.len(), 1);
    let sp = &ps[0].1.subpaths[0];
    assert_eq!(sp.anchors.len(), 3);
    assert_eq!(sp.anchors[1].kind, vectorcraft_geom::AnchorKind::Smooth);
    // Alt-click the middle point → corner.
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 200.0).with_mods(Mods { alt: true, ..Default::default() }), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 200.0, 200.0), v).unwrap();
    assert!(!path(&s, ps[0].0).subpaths[0].anchors[1].has_out());
    // Drag the middle point.
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 200.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 150.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 200.0, 150.0), v).unwrap();
    assert_eq!(path(&s, ps[0].0).subpaths[0].anchors[1].p, Point::new(200.0, 150.0));
    // Esc ends; the next click starts a new path.
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 500.0, 500.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 500.0, 500.0), v).unwrap();
    assert_eq!(paths(&s).len(), 2);
}

#[test]
fn curvature_click_first_point_closes() {
    let mut s = session();
    let v = view();
    s.select_tool("curvature", v).unwrap();
    for (x, y) in [(100.0, 100.0), (200.0, 100.0), (150.0, 200.0), (100.0, 100.0)] {
        s.pointer(&PointerEvent::new(PointerKind::Down, x, y), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, x, y), v).unwrap();
    }
    let p = &paths(&s)[0].1;
    assert!(p.is_closed());
    assert_eq!(p.anchor_count(), 3);
}

/// The Curvature tool edits a path another tool drew (#798), one undo step per edit, keeping its
/// shape except where edited.
#[test]
fn curvature_edits_any_selected_path() {
    let mut s = session();
    let v = view();
    let anchors = json!([
        {"x": 100, "y": 300},
        {"x": 200, "y": 200, "in": [160, 210], "out": [260, 185]},
        {"x": 350, "y": 280, "in": [320, 230], "out": [380, 330]},
        {"x": 450, "y": 350, "in": [420, 360], "out": [480, 300]},
        {"x": 550, "y": 260, "in": [520, 290]}
    ]);
    let id = NodeId(s.execute("path.create", &json!({ "anchors": anchors })).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.select_tool("curvature", v).unwrap();
    let before = path(&s, id).subpaths[0].clone();
    // Drag a point: only its two segments change.
    assert_eq!(gesture(&mut s, &[(350.0, 280.0), (360.0, 320.0), (370.0, 340.0)], Mods::default()), 1);
    let sp = path(&s, id).subpaths[0].clone();
    assert_eq!(sp.anchors[2].p, Point::new(370.0, 340.0));
    assert_eq!((sp.segment(0), sp.segment(3)), (before.segment(0), before.segment(3)));
    assert_eq!(s.doc().unwrap().selection.anchors.get(&id).map(|a| a.len()), Some(1), "the point is current");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(path(&s, id).subpaths[0], before);
    // Alt-click toggles it, Delete removes the current point: one step each.
    let (alt, steps) = (Mods { alt: true, ..Default::default() }, undo_steps(&s));
    assert_eq!(gesture(&mut s, &[(200.0, 200.0)], alt), 1);
    assert!(!path(&s, id).subpaths[0].anchors[1].has_out());
    s.tool_key(ToolKey::Delete, Mods::default(), v).unwrap();
    assert_eq!(path(&s, id).anchor_count(), 4);
    assert_eq!(undo_steps(&s), steps + 2);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(path(&s, id).subpaths[0], before);
    // A press on a segment adds a point, the drag moving it: one step.
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let on = kurbo::ParamCurve::eval(&before.segment(3), 0.5);
    assert_eq!(gesture(&mut s, &[(on.x, on.y), (on.x, on.y + 20.0), (on.x, on.y + 30.0)], Mods::default()), 1);
    let sp = path(&s, id).subpaths[0].clone();
    assert_eq!(sp.anchors.len(), 6);
    assert_eq!(sp.anchors[4].p, Point::new(on.x, on.y + 30.0));
    assert_eq!((sp.segment(0), sp.segment(1), sp.segment(2)), (before.segment(0), before.segment(1), before.segment(2)));
    s.execute("edit.undo", &json!({})).unwrap();
    // A click on an end, then two more: the path goes on, its old segments unchanged.
    let steps = undo_steps(&s);
    for (x, y) in [(550.0, 260.0), (650.0, 300.0), (720.0, 220.0)] {
        gesture(&mut s, &[(x, y)], Mods::default());
    }
    assert_eq!(undo_steps(&s), steps + 2, "selecting the end is no step");
    let sp = path(&s, id).subpaths[0].clone();
    assert_eq!(sp.anchors.len(), 7);
    assert!((0..4).all(|i| sp.segment(i) == before.segment(i)));
    assert_eq!(sp.anchors[6].p, Point::new(720.0, 220.0));
    // The other end closes it; Esc and a click away start a new path.
    gesture(&mut s, &[(100.0, 300.0)], Mods::default());
    assert!(path(&s, id).is_closed());
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    gesture(&mut s, &[(700.0, 550.0)], Mods::default());
    assert_eq!(paths(&s).len(), 2);
}

#[test]
fn add_and_delete_anchor_tools() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    click(&mut s, "addAnchor", 150.0, 100.0);
    let p = path(&s, id);
    assert_eq!(p.anchor_count(), 5);
    assert!(near(p.subpaths[0].anchors[1].p, Point::new(150.0, 100.0)));
    click(&mut s, "deleteAnchor", 150.0, 100.0);
    click(&mut s, "deleteAnchor", 200.0, 200.0);
    let p = path(&s, id);
    assert_eq!(p.anchor_count(), 3);
    assert!(p.is_closed());
}

#[test]
fn delete_anchor_on_circle_keeps_curve() {
    let mut s = session();
    let r = s.execute("shape.ellipse", &json!({"x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    let before = path(&s, id);
    let a = before.subpaths[0].anchors[1].p;
    click(&mut s, "deleteAnchor", a.x, a.y);
    let p = path(&s, id);
    assert_eq!(p.anchor_count(), before.anchor_count() - 1);
    // The remaining curve still bulges out towards the removed anchor.
    let b = p.bounds().unwrap();
    assert!(b.width() > 150.0 && b.height() > 150.0, "{b:?}");
}

#[test]
fn anchor_point_tool_converts() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    drag(&mut s, "anchorPoint", &[(100.0, 100.0), (130.0, 80.0)], Mods::default());
    let a = path(&s, id).subpaths[0].anchors[0];
    assert_eq!(a.kind, vectorcraft_geom::AnchorKind::Smooth);
    assert_eq!(a.h_out, Point::new(130.0, 80.0));
    assert_eq!(a.h_in, Point::new(70.0, 120.0));
    // Click the smooth anchor → corner again.
    click(&mut s, "anchorPoint", 100.0, 100.0);
    let a = path(&s, id).subpaths[0].anchors[0];
    assert!(!a.has_in() && !a.has_out());
    // Drag a segment → it bends so the grabbed point follows.
    drag(&mut s, "anchorPoint", &[(150.0, 200.0), (150.0, 240.0)], Mods::default());
    let p = path(&s, id);
    let (_, _, _, q, d) = p.nearest(Point::new(150.0, 240.0)).unwrap();
    assert!(d < 0.5, "{q:?}");
}

#[test]
fn scissors_opens_closed_path() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    click(&mut s, "scissors", 200.0, 150.0);
    let p = path(&s, id);
    assert!(!p.is_closed());
    assert_eq!(p.anchor_count(), 6);
    assert!(near(p.subpaths[0].anchors[0].p, Point::new(200.0, 150.0)));
    assert!(near(p.subpaths[0].anchors[5].p, Point::new(200.0, 150.0)));
}

#[test]
fn scissors_splits_open_path_in_two() {
    let mut s = session();
    let id = line(&mut s, 100.0, 100.0, 300.0, 100.0);
    click(&mut s, "scissors", 150.0, 100.0);
    let ps = paths(&s);
    assert_eq!(ps.len(), 2);
    assert!(near(path(&s, id).subpaths[0].anchors[1].p, Point::new(150.0, 100.0)));
    assert!(near(ps[1].1.subpaths[0].anchors[0].p, Point::new(150.0, 100.0)));
    assert_eq!(ps[1].1.subpaths[0].anchors[1].p, Point::new(300.0, 100.0));
    assert_eq!(s.doc().unwrap().selection.objects.len(), 2);
    // Clicking an end point is refused without panicking.
    assert!(s.execute("path.split", &json!({"id": id.0, "subpath": 0, "anchor": 0})).is_err());
}

#[test]
fn knife_cuts_rect_into_two_pieces() {
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("select.none", &json!({})).unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    drag(&mut s, "knife", &[(200.0, 50.0), (210.0, 150.0), (200.0, 250.0)], Mods::default());
    let ps = paths(&s);
    assert_eq!(ps.len(), 2);
    let total: f64 = ps.iter().map(|(_, p)| area(p)).sum();
    assert!((total - 20000.0).abs() < 50.0, "{total}");
    assert!(ps.iter().all(|(_, p)| p.is_closed() && area(p) > 9000.0));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(paths(&s).len(), 1);
}

#[test]
fn knife_partial_cut_does_nothing() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("select.none", &json!({})).unwrap();
    let r = s.execute("path.knife", &json!({"points": [[200, 50], [200, 150]]})).unwrap();
    assert_eq!(r["ids"], json!([]));
    assert_eq!(path(&s, id).anchor_count(), 4);
}

#[test]
fn eraser_splits_shapes_and_lines() {
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    line(&mut s, 100.0, 300.0, 300.0, 300.0);
    s.execute("select.none", &json!({})).unwrap();
    drag(&mut s, "eraser", &[(200.0, 50.0), (200.0, 350.0)], Mods::default());
    let ps = paths(&s);
    assert_eq!(ps.len(), 4, "two rect halves + two line pieces");
    let closed: Vec<&PathData> = ps.iter().map(|p| &p.1).filter(|p| p.is_closed()).collect();
    assert_eq!(closed.len(), 2);
    let total: f64 = closed.iter().map(|p| area(p)).sum();
    assert!((total - (20000.0 - 1000.0)).abs() < 50.0, "{total}");
    let open: Vec<&PathData> = ps.iter().map(|p| &p.1).filter(|p| !p.is_closed()).collect();
    let len: f64 = open.iter().map(|p| p.length()).sum();
    assert!((len - 190.0).abs() < 0.1, "{len}");
}

#[test]
fn eraser_removes_small_shape_entirely() {
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 4.0, 4.0);
    s.execute("select.none", &json!({})).unwrap();
    s.execute("path.eraseRegion", &json!({"points": [[102, 102]], "size": 20})).unwrap();
    assert!(paths(&s).is_empty());
}

#[test]
fn path_eraser_splits_selected_path() {
    let mut s = session();
    let id = line(&mut s, 100.0, 100.0, 300.0, 100.0);
    drag(&mut s, "pathEraser", &[(180.0, 100.0), (220.0, 100.0)], Mods::default());
    let ps = paths(&s);
    assert_eq!(ps.len(), 2);
    assert!(path(&s, id).subpaths[0].anchors[1].p.x < 180.0);
}

#[test]
fn smooth_tool_reduces_jagged_path() {
    let mut s = session();
    let anchors: Vec<_> = (0..=40).map(|i| json!({"x": 100.0 + i as f64 * 5.0, "y": 200.0 + if i % 2 == 0 { 0.0 } else { 1.5 }})).collect();
    let r = s.execute("path.create", &json!({"anchors": anchors})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    drag(&mut s, "smooth", &[(90.0, 200.0), (200.0, 200.0), (310.0, 200.0)], Mods::default());
    let p = path(&s, id);
    assert!(p.anchor_count() < 20, "{}", p.anchor_count());
    assert_eq!(p.subpaths[0].anchors[0].p, Point::new(100.0, 200.0));
}

#[test]
fn join_tool_joins_two_lines() {
    let mut s = session();
    let a = line(&mut s, 100.0, 100.0, 200.0, 100.0);
    line(&mut s, 205.0, 100.0, 300.0, 150.0);
    s.execute("select.none", &json!({})).unwrap();
    drag(&mut s, "join", &[(202.0, 90.0), (202.0, 110.0)], Mods::default());
    let ps = paths(&s);
    assert_eq!(ps.len(), 1);
    let p = path(&s, a);
    assert_eq!(p.anchor_count(), 3);
    assert_eq!(p.subpaths[0].anchors[1].p, Point::new(202.5, 100.0));
    assert_eq!(p.subpaths[0].anchors[2].p, Point::new(300.0, 150.0));
}

#[test]
fn line_family_tools_draw_and_click_asks_dialog() {
    let mut s = session();
    drag(&mut s, "arc", &[(100.0, 100.0), (200.0, 150.0)], Mods::default());
    drag(&mut s, "spiral", &[(400.0, 300.0), (450.0, 300.0)], Mods::default());
    drag(&mut s, "rectangularGrid", &[(100.0, 300.0), (200.0, 400.0)], Mods::default());
    drag(&mut s, "polarGrid", &[(500.0, 100.0), (600.0, 200.0)], Mods::default());
    let top = s.doc().unwrap().doc.layers[0].children().unwrap().len();
    assert_eq!(top, 4);
    let b = paths(&s)[0].1.bounds().unwrap();
    assert!((b.width() - 100.0).abs() < 1e-6 && (b.height() - 50.0).abs() < 1e-6, "{b:?}");
    let ui = click(&mut s, "polarGrid", 10.0, 10.0);
    assert_eq!(ui, vec![UiRequest::Dialog("polarGrid".into(), json!({"x": 10.0, "y": 10.0}))]);
}

#[test]
fn draw2_commands_reject_bad_params() {
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    for (cmd, p) in [
        ("path.freehand", json!({})),
        ("path.freehand", json!({"points": [[1, 2]]})),
        ("path.curvature", json!({"points": []})),
        ("path.removeAnchor", json!({"id": id.0, "anchor": 99})),
        ("path.removeAnchors", json!({})),
        ("path.convertAnchor", json!({"id": 9999, "anchor": 0, "to": "corner"})),
        ("path.reshapeSegment", json!({"id": id.0, "segment": 42})),
        ("path.split", json!({"id": id.0})),
        ("path.split", json!({"id": id.0, "segment": 0, "t": "x"})),
        ("path.knife", json!({"points": [[0, 0]]})),
        ("path.eraseRegion", json!({"points": [[0, 0]], "size": -1})),
        ("path.blob", json!({"points": "nope"})),
        ("path.joinScrub", json!({"points": []})),
    ] {
        let before = s.doc().unwrap().history.undo.len();
        assert!(s.execute(cmd, &p).is_err(), "{cmd} {p}");
        assert_eq!(s.doc().unwrap().history.undo.len(), before, "{cmd} left an undo step");
    }
}

/// A curve from (100, 100) to (300, 100) whose first anchor has an out handle at (150, 100).
fn curve(s: &mut Session) -> NodeId {
    let anchors = json!([{"x": 100, "y": 100, "out": [150, 100]}, {"x": 300, "y": 100, "in": [250, 100]}]);
    NodeId(s.execute("path.create", &json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap())
}

/// Drag the out handle of path `id`'s first anchor with `tool` to `to`, holding `mods`. Direct
/// Selection clicks the anchor first, so its handles show.
fn drag_handle(s: &mut Session, tool: &str, id: NodeId, to: (f64, f64), mods: Mods, v: ViewInfo) {
    let from = path(s, id).subpaths[0].anchors[0].h_out;
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.select_tool(tool, v).unwrap();
    if tool == "directSelection" {
        s.execute("select.none", &json!({})).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 100.0), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, 100.0, 100.0), v).unwrap();
    }
    s.pointer(&PointerEvent::new(PointerKind::Down, from.x, from.y).with_mods(mods), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, to.0, to.1).with_mods(mods), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, to.0, to.1).with_mods(mods), v).unwrap();
    assert!(!s.in_interaction());
}

/// The out handle of path `id`'s first anchor.
fn out_handle(s: &Session, id: NodeId) -> Point {
    path(s, id).subpaths[0].anchors[0].h_out
}

#[test]
fn shift_keeps_a_dragged_handle_at_45_degree_steps() {
    // #322.
    let shift = Mods { shift: true, ..Mods::default() };
    for tool in ["directSelection", "anchorPoint"] {
        let mut s = session();
        let id = curve(&mut s);
        drag_handle(&mut s, tool, id, (190.0, 130.0), shift, view());
        let h = out_handle(&s, id);
        assert!(near(h, Point::new(100.0 + 90.0f64.hypot(30.0), 100.0)), "{tool}: {h:?}");
        drag_handle(&mut s, tool, id, (160.0, 155.0), shift, view());
        let h = out_handle(&s, id);
        assert!((h.x - h.y).abs() < 1e-6 && h.x > 100.0, "{tool}: 45°: {h:?}");
        // Without Shift the handle goes where it is dragged.
        drag_handle(&mut s, tool, id, (190.0, 130.0), Mods::default(), view());
        assert_eq!(out_handle(&s, id), Point::new(190.0, 130.0), "{tool}");
    }
}

#[test]
fn a_dragged_handle_snaps_to_smart_guides() {
    // #322: in line with the other anchor, with smart guides on.
    for tool in ["directSelection", "anchorPoint"] {
        let mut s = session();
        let id = curve(&mut s);
        drag_handle(&mut s, tool, id, (298.0, 160.0), Mods::default(), ViewInfo::default());
        assert_eq!(out_handle(&s, id), Point::new(300.0, 160.0), "{tool}");
        drag_handle(&mut s, tool, id, (298.0, 170.0), Mods::default(), view());
        assert_eq!(out_handle(&s, id), Point::new(298.0, 170.0), "{tool}: smart guides off");
    }
}

#[test]
fn shift_drag_keeps_a_smooth_anchor_smooth_and_alt_breaks_it() {
    // #322: the opposite handle of a smooth anchor turns with the constrained one; Alt still
    // moves the dragged handle alone.
    let shift = Mods { shift: true, ..Mods::default() };
    let shift_alt = Mods { shift: true, alt: true, ..Mods::default() };
    for (mods, opposite_follows) in [(shift, true), (shift_alt, false)] {
        let mut s = session();
        let anchors = json!([{"x": 100, "y": 200}, {"x": 200, "y": 100, "in": [150, 100], "out": [250, 100]}, {"x": 300, "y": 200}]);
        let id = NodeId(s.execute("path.create", &json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap());
        s.select_tool("directSelection", view()).unwrap();
        for (kind, x, y, m) in [
            (PointerKind::Down, 200.0, 100.0, Mods::default()),
            (PointerKind::Up, 200.0, 100.0, Mods::default()),
            (PointerKind::Down, 250.0, 100.0, mods),
            (PointerKind::Drag, 275.0, 165.0, mods),
            (PointerKind::Up, 275.0, 165.0, mods),
        ] {
            s.pointer(&PointerEvent::new(kind, x, y).with_mods(m), view()).unwrap();
        }
        let a = path(&s, id).subpaths[0].anchors[1];
        let (o, i) = (a.h_out - a.p, a.h_in - a.p);
        assert!((o.x - o.y).abs() < 1e-6 && o.x > 0.0, "{mods:?}: 45°: {o:?}");
        if opposite_follows {
            assert!(near(a.h_in, a.p - o.normalize() * 50.0), "{mods:?}: {i:?}");
        } else {
            assert_eq!(a.h_in, Point::new(150.0, 100.0), "{mods:?}");
        }
    }
}

#[test]
fn removing_an_anchor_refits_a_split_curve_and_undo_restores_it() {
    let mut s = session();
    let made = s
        .execute(
            "path.create",
            &json!({"anchors": [
                {"x": 0, "y": 0, "out": [30, 80]},
                {"x": 100, "y": 0, "in": [70, 80]}
            ]}),
        )
        .unwrap();
    let id = NodeId(made["id"].as_u64().unwrap());
    let before = path(&s, id);
    let inserted = s.execute("path.insertAnchor", &json!({"id": id.0, "segment": 0, "t": 0.4})).unwrap();
    let ai = inserted["anchor"].as_u64().unwrap();
    s.execute("path.removeAnchor", &json!({"id": id.0, "anchor": ai})).unwrap();
    let after = path(&s, id);
    assert_eq!(after.subpaths[0].anchors.len(), 2);
    let (got, orig) = (&after.subpaths[0].anchors, &before.subpaths[0].anchors);
    assert!(got[0].h_out.distance(orig[0].h_out) < 0.5, "{:?}", got[0].h_out);
    assert!(got[1].h_in.distance(orig[1].h_in) < 0.5, "{:?}", got[1].h_in);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(path(&s, id).subpaths[0].anchors.len(), 3);
}

#[test]
fn remove_anchor_points_keeps_the_paths_closed() {
    // Object › Path › Remove Anchor Points, unlike the Delete key: a rectangle loses a corner and
    // stays closed, a straight-sided triangle; a curve keeps its shape round a removed point.
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 80.0);
    let made = s.execute("path.create", &json!({"anchors": [{"x": 300, "y": 0, "out": [330, 80]}, {"x": 400, "y": 0, "in": [370, 80]}]})).unwrap();
    let c = NodeId(made["id"].as_u64().unwrap());
    let before = path(&s, c).subpaths[0].segment(0);
    s.execute("path.insertAnchor", &json!({"id": c.0, "segment": 0, "t": 0.5})).unwrap();
    s.execute("select.anchors", &json!({"id": r.0, "anchors": [[0, 0]]})).unwrap();
    s.execute("select.anchors", &json!({"id": c.0, "anchors": [[0, 1]], "mode": "add"})).unwrap();
    assert_eq!(s.execute("path.removeAnchors", &json!({})).unwrap()["removedObjects"], 0);
    let sp = &path(&s, r).subpaths[0];
    assert!(sp.closed && sp.anchors.len() == 3, "{sp:?}");
    assert!(!sp.anchors.iter().any(|a| a.has_in() || a.has_out()), "straight sides stay straight: {sp:?}");
    let after = path(&s, c).subpaths[0].segment(0);
    assert_eq!(path(&s, c).subpaths[0].anchors.len(), 2);
    assert!(after.p1.distance(before.p1) < 0.5 && after.p2.distance(before.p2) < 0.5, "{after:?}");
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Remove Anchor Points");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(path(&s, r).subpaths[0].anchors.len(), 4);
    assert_eq!(path(&s, c).subpaths[0].anchors.len(), 3);
    let spec = find_command("path.removeAnchors").unwrap();
    assert_eq!(spec.menu, &["Object", "Path"][..]);
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn convert_anchors_turns_the_selected_anchors_corner_or_smooth() {
    // The Control bar's convert buttons: one undo step each, other anchors left alone.
    let mut s = session();
    let made = s.execute("path.create", &json!({"anchors": [{"x": 100, "y": 200}, {"x": 200, "y": 100}, {"x": 300, "y": 200}]})).unwrap();
    let id = NodeId(made["id"].as_u64().unwrap());
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 1]]})).unwrap();
    let undo = undo_steps(&s);
    s.execute("path.convertAnchors", &json!({"to": "smooth"})).unwrap();
    let a = path(&s, id).subpaths[0].anchors.clone();
    assert_eq!(undo_steps(&s), undo + 1);
    assert_eq!(a[1].kind, vectorcraft_geom::AnchorKind::Smooth);
    let r = 100.0 * 2f64.sqrt() / 3.0;
    assert!(near(a[1].h_in, Point::new(200.0 - r, 100.0)) && near(a[1].h_out, Point::new(200.0 + r, 100.0)), "{:?}", a[1]);
    assert!(!a[0].has_out() && !a[2].has_in(), "the neighbours keep theirs");
    // Smooth again keeps the handles; corner retracts them.
    s.execute("path.setHandle", &json!({"id": id.0, "anchor": 1, "which": "out", "x": 260, "y": 100})).unwrap();
    let h = path(&s, id).subpaths[0].anchors[1];
    s.execute("path.convertAnchors", &json!({"to": "smooth"})).unwrap();
    assert_eq!(path(&s, id).subpaths[0].anchors[1], h);
    s.execute("path.convertAnchors", &json!({"to": "corner"})).unwrap();
    let a = path(&s, id).subpaths[0].anchors[1];
    assert!(!a.has_in() && !a.has_out(), "{a:?}");
    // Anchors that are gone (a stale selection) are skipped; a bad `to` is refused.
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 99], [7, 0]]})).unwrap();
    s.execute("path.convertAnchors", &json!({"to": "smooth"})).unwrap();
    let undo = undo_steps(&s);
    assert!(s.execute("path.convertAnchors", &json!({"to": "round"})).is_err());
    assert_eq!(undo_steps(&s), undo);
}

#[test]
fn cut_at_anchors_splits_paths_and_leaves_one_end_selected() {
    // Cut Path at Selected Anchor Points: an open path becomes one path per piece, and one anchor
    // of each cut stays selected, so dragging it pulls the path apart there.
    let mut s = session();
    let anchors = json!([{"x": 100, "y": 100}, {"x": 200, "y": 100, "in": [170, 80], "out": [230, 120]}, {"x": 300, "y": 100}, {"x": 400, "y": 100}]);
    let id = NodeId(s.execute("path.create", &json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 1], [0, 2]]})).unwrap();
    let undo = undo_steps(&s);
    let made = s.execute("path.cutAtAnchors", &json!({})).unwrap();
    let ids: Vec<NodeId> = made["ids"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect();
    assert_eq!(undo_steps(&s), undo + 1);
    assert_eq!((ids.len(), ids[0]), (3, id));
    let pieces: Vec<Vec<Point>> = ids.iter().map(|i| path(&s, *i).subpaths[0].anchors.iter().map(|a| a.p).collect()).collect();
    let p = |x: f64| Point::new(x, 100.0);
    assert_eq!(pieces, vec![vec![p(100.0), p(200.0)], vec![p(200.0), p(300.0)], vec![p(300.0), p(400.0)]]);
    // The curve keeps its shape: each end keeps the handle on its own side only.
    let (l, r) = (path(&s, ids[0]).subpaths[0].anchors[1], path(&s, ids[1]).subpaths[0].anchors[0]);
    assert!(l.h_in == Point::new(170.0, 80.0) && !l.has_out() && r.h_out == Point::new(230.0, 120.0) && !r.has_in(), "{l:?} {r:?}");
    let sel = s.doc().unwrap().selection.clone();
    assert_eq!(sel.objects, vec![ids[1], ids[2]]);
    assert!(sel.anchors.values().all(|a| a.iter().eq([&(0, 0)])), "{sel:?}");
    s.execute("path.moveAnchors", &json!({"dx": 0, "dy": 50})).unwrap();
    assert_eq!(path(&s, ids[0]).subpaths[0].anchors[1].p, p(200.0));
    assert_eq!(path(&s, ids[1]).subpaths[0].anchors[0].p, Point::new(200.0, 150.0));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(paths(&s).len(), 1);
    // A closed path opens at the cut, its ends on top of each other.
    let r = rect(&mut s, 500.0, 100.0, 100.0, 100.0);
    s.execute("select.anchors", &json!({"id": r.0, "anchors": [[0, 2]]})).unwrap();
    s.execute("path.cutAtAnchors", &json!({})).unwrap();
    let sp = &path(&s, r).subpaths[0];
    assert!(!sp.closed && sp.anchors.len() == 5 && sp.anchors[0].p == sp.anchors[4].p, "{sp:?}");
    assert_eq!(s.doc().unwrap().selection.anchors[&r].iter().collect::<Vec<_>>(), [&(0, 0)]);
    // Only the end points of an open path selected: nothing to cut.
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 0], [0, 3]]})).unwrap();
    let undo = undo_steps(&s);
    assert!(s.execute("path.cutAtAnchors", &json!({})).is_err());
    assert_eq!(undo_steps(&s), undo);
}

#[test]
fn pen_with_alt_moves_one_handle_and_converts_anchors() {
    // Discord feedback: Alt held with the Pen moves a direction handle on its own (and works as
    // the Anchor Point tool on a selected path's anchors), while a path is being drawn too.
    let alt = Mods { alt: true, ..Mods::default() };
    let v = view();
    let mut s = session();
    let anchors = json!([{"x": 100, "y": 200}, {"x": 200, "y": 100, "in": [150, 100], "out": [250, 100]}, {"x": 300, "y": 200}]);
    let id = NodeId(s.execute("path.create", &json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap());
    s.select_tool("pen", v).unwrap();
    // Alt-drag the out handle: it moves alone, the in handle stays.
    assert_eq!(gesture(&mut s, &[(250.0, 100.0), (260.0, 130.0), (275.0, 165.0)], alt), 1);
    assert!(!s.in_interaction());
    let a = path(&s, id).subpaths[0].anchors[1];
    assert_eq!((a.h_in, a.h_out), (Point::new(150.0, 100.0), Point::new(275.0, 165.0)));
    assert_eq!(paths(&s).len(), 1, "no new path or anchor");
    // Alt-click the anchor: a corner. Alt-drag it: new symmetric handles.
    assert_eq!(gesture(&mut s, &[(200.0, 100.0)], alt), 1);
    let a = path(&s, id).subpaths[0].anchors[1];
    assert!(!a.has_in() && !a.has_out(), "{a:?}");
    gesture(&mut s, &[(200.0, 100.0), (220.0, 100.0), (240.0, 110.0)], alt);
    let a = path(&s, id).subpaths[0].anchors[1];
    assert_eq!((a.h_in, a.h_out), (Point::new(160.0, 90.0), Point::new(240.0, 110.0)));
    assert_eq!(path(&s, id).subpaths[0].anchors.len(), 3);
    // Drawing: place a smooth point, then Alt-drag its outgoing handle; the next click goes on
    // drawing the same path.
    s.execute("select.none", &json!({})).unwrap();
    gesture(&mut s, &[(100.0, 400.0)], Mods::default());
    gesture(&mut s, &[(200.0, 400.0), (250.0, 400.0)], Mods::default());
    let new = *s.doc().unwrap().selection.objects.first().unwrap();
    gesture(&mut s, &[(250.0, 400.0), (230.0, 350.0)], alt);
    let a = path(&s, new).subpaths[0].anchors[1];
    assert_eq!((a.h_in, a.h_out), (Point::new(150.0, 400.0), Point::new(230.0, 350.0)));
    gesture(&mut s, &[(300.0, 450.0)], Mods::default());
    assert_eq!(path(&s, new).subpaths[0].anchors.len(), 3);
    assert_eq!(paths(&s).len(), 2);
}

/// The anchor's handles point opposite ways.
fn smooth(a: vectorcraft_geom::Anchor) -> bool {
    let (i, o) = (a.h_in - a.p, a.h_out - a.p);
    (i.x * o.y - i.y * o.x).abs() < 1e-6 && i.x * o.x + i.y * o.y < 0.0
}

#[test]
fn pen_with_cmd_borrows_direct_selection_and_goes_on_drawing() {
    // #494: Cmd (Ctrl) held with the Pen drags with the selection tool used last (Direct Selection
    // until one is chosen): handles, segments and anchors of the path being drawn; released, the
    // Pen goes on drawing that path. Through pointer events, so agents reach it too.
    let (none, cmd) = (Mods::default(), Mods { cmd: true, ..Mods::default() });
    let v = view();
    let mut s = session();
    s.select_tool("pen", v).unwrap();
    gesture(&mut s, &[(100.0, 300.0)], none);
    gesture(&mut s, &[(200.0, 300.0), (250.0, 300.0)], none);
    let id = s.doc().unwrap().selection.objects[0];
    assert_eq!(s.cursor(Point::new(250.0, 300.0), none, v), vectorcraft_tools::Cursor::Pen);
    assert_eq!(s.cursor(Point::new(250.0, 300.0), cmd, v), vectorcraft_tools::Cursor::ArrowHollow, "Direct Selection's");
    // The new anchor's outgoing handle: it moves, its incoming one turning with it.
    s.pointer(&PointerEvent::new(PointerKind::Down, 250.0, 300.0).with_mods(cmd), v).unwrap();
    assert_eq!(s.tool_id(), "directSelection");
    s.pointer(&PointerEvent::new(PointerKind::Drag, 250.0, 260.0).with_mods(cmd), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 250.0, 260.0).with_mods(cmd), v).unwrap();
    assert_eq!(s.tool_id(), "pen");
    let a = path(&s, id).subpaths[0].anchors[1];
    assert!(a.h_out == Point::new(250.0, 260.0) && smooth(a), "{a:?}");
    // The segment between the two anchors bends, the smooth anchor staying smooth.
    let before = path(&s, id).subpaths[0].anchors[1].h_in;
    gesture(&mut s, &[(120.0, 300.0), (120.0, 270.0)], cmd);
    let a = path(&s, id).subpaths[0].anchors[1];
    assert!(a.h_in != before && smooth(a), "{a:?}");
    assert_eq!(s.tool_id(), "pen");
    // A plain click goes on drawing the same path.
    gesture(&mut s, &[(300.0, 350.0)], none);
    assert_eq!((paths(&s).len(), path(&s, id).subpaths[0].anchors.len()), (1, 3));
    // A Cmd-click away from the art deselects: the path is done, the next click starts another.
    gesture(&mut s, &[(600.0, 100.0)], cmd);
    gesture(&mut s, &[(400.0, 500.0)], none);
    assert_eq!(paths(&s).len(), 2);
    assert_eq!(path(&s, id).subpaths[0].anchors.len(), 3);
    // With the Selection tool chosen last, Cmd lends it.
    s.select_tool("selection", v).unwrap();
    s.select_tool("pen", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 600.0, 100.0).with_mods(cmd), v).unwrap();
    assert_eq!(s.tool_id(), "selection");
    s.pointer(&PointerEvent::new(PointerKind::Up, 600.0, 100.0).with_mods(cmd), v).unwrap();
    assert_eq!(s.tool_id(), "pen");
    // Choosing a tool mid-gesture keeps it: the Pen doesn't come back over it.
    s.pointer(&PointerEvent::new(PointerKind::Down, 600.0, 100.0).with_mods(cmd), v).unwrap();
    s.select_tool("rectangle", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 600.0, 100.0), v).unwrap();
    assert_eq!(s.tool_id(), "rectangle");
}

#[test]
fn reshaping_a_segment_keeps_smooth_anchors_smooth() {
    // #494: the anchors at the ends of a dragged segment keep their kind: a smooth one's other
    // handle turns with the moved one (at its length), a corner's stays.
    let mut s = session();
    let anchors = json!([{"x": 100, "y": 300}, {"x": 200, "y": 300, "in": [160, 300], "out": [240, 300]}, {"x": 300, "y": 300}]);
    let id = NodeId(s.execute("path.create", &json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap());
    s.execute("path.reshapeSegment", &json!({"id": id.0, "segment": 0, "t": 0.5, "dx": 0, "dy": -30})).unwrap();
    let sp = &path(&s, id).subpaths[0];
    let a = sp.anchors[1];
    assert!(near(a.h_in, Point::new(160.0, 260.0)) && smooth(a), "{a:?}");
    assert!((a.h_out.distance(a.p) - 40.0).abs() < 1e-9, "its length kept");
    assert!(near(sp.anchors[0].h_out, Point::new(100.0, 260.0)) && !sp.anchors[0].has_in(), "the corner end: {:?}", sp.anchors[0]);
}

#[test]
fn pen_drag_from_the_first_anchor_closes_the_path() {
    // #501: closing a path by dragging out of its first anchor flickered between a close and a new
    // path, and released at the wrong moment left the shape open. The press decides: the drag
    // shapes the closing curve, symmetric, and the release closes the path wherever it ends.
    let none = Mods::default();
    let v = view();
    let mut s = session();
    s.select_tool("pen", v).unwrap();
    for (x, y) in [(100.0, 300.0), (200.0, 300.0), (200.0, 400.0)] {
        gesture(&mut s, &[(x, y)], none);
    }
    let id = s.doc().unwrap().selection.objects[0];
    let pts: Vec<(f64, f64)> = (0..=12).map(|i| (100.0 + 5.0 * f64::from(i), 300.0 - 4.0 * f64::from(i))).collect();
    s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 300.0), v).unwrap();
    for &(x, y) in &pts[1..] {
        s.pointer(&PointerEvent::new(PointerKind::Drag, x, y), v).unwrap();
        let p = path(&s, id);
        assert!(p.subpaths[0].closed && paths(&s).len() == 1, "closed throughout the drag, at ({x}, {y})");
    }
    s.pointer(&PointerEvent::new(PointerKind::Up, 160.0, 252.0), v).unwrap();
    assert!(!s.in_interaction());
    let sp = &path(&s, id).subpaths[0];
    assert!(sp.closed && sp.anchors.len() == 3, "{sp:?}");
    let a = sp.anchors[0];
    assert_eq!((a.h_in, a.h_out), (Point::new(40.0, 348.0), Point::new(160.0, 252.0)), "symmetric");
    assert_eq!(paths(&s).len(), 1);
    // The next click starts a new path.
    gesture(&mut s, &[(500.0, 500.0)], none);
    assert_eq!(paths(&s).len(), 2);
}

#[test]
fn pen_with_alt_converts_the_anchors_of_the_path_being_drawn() {
    // #504: while drawing, Alt-click on a smooth anchor of the path makes it a corner and Alt-drag
    // on a corner one pulls out smooth handles; the next click goes on drawing the path.
    let (none, alt) = (Mods::default(), Mods { alt: true, ..Mods::default() });
    let v = view();
    let mut s = session();
    s.select_tool("pen", v).unwrap();
    gesture(&mut s, &[(100.0, 300.0)], none);
    gesture(&mut s, &[(200.0, 300.0), (240.0, 300.0)], none);
    gesture(&mut s, &[(300.0, 400.0)], none);
    gesture(&mut s, &[(400.0, 300.0)], none);
    let id = s.doc().unwrap().selection.objects[0];
    assert!(smooth(path(&s, id).subpaths[0].anchors[1]));
    assert_eq!(gesture(&mut s, &[(200.0, 300.0)], alt), 1);
    let a = path(&s, id).subpaths[0].anchors[1];
    assert!(!a.has_in() && !a.has_out(), "a corner: {a:?}");
    assert_eq!(gesture(&mut s, &[(300.0, 400.0), (320.0, 400.0), (340.0, 400.0)], alt), 1);
    let a = path(&s, id).subpaths[0].anchors[2];
    assert!(smooth(a) && a.h_out == Point::new(340.0, 400.0), "smooth: {a:?}");
    gesture(&mut s, &[(500.0, 400.0)], none);
    assert_eq!((paths(&s).len(), path(&s, id).subpaths[0].anchors.len()), (1, 5));
}

/// #776: the Pen continues any open path from the end clicked, selected or not, and while drawing a
/// click on an end of another open path joins the two into one path, in one undo step, which
/// finishes it.
#[test]
fn the_pen_continues_any_open_path_and_joins_another() {
    use vectorcraft_tools::Cursor;
    let (v, none) = (view(), Mods::default());
    let mut s = session();
    let a = line(&mut s, 100.0, 100.0, 200.0, 100.0);
    let b = line(&mut s, 300.0, 200.0, 400.0, 200.0);
    s.execute("select.none", &json!({})).unwrap();
    s.select_tool("pen", v).unwrap();
    assert_eq!(s.cursor(Point::new(200.0, 100.0), none, v), Cursor::PenContinue, "an end of a path that isn't selected");
    gesture(&mut s, &[(200.0, 100.0)], none);
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    assert_eq!(s.cursor(Point::new(300.0, 200.0), none, v), Cursor::PenJoin);
    assert_eq!(s.cursor(Point::new(350.0, 200.0), none, v), Cursor::Pen, "the middle of the other path");
    assert_eq!(gesture(&mut s, &[(300.0, 200.0)], none), 1);
    let pts = |s: &Session, id| path(s, id).subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|x| (x.p.x, x.p.y))).collect::<Vec<_>>();
    assert_eq!(pts(&s, a), [(100.0, 100.0), (200.0, 100.0), (300.0, 200.0), (400.0, 200.0)]);
    assert!(s.doc().unwrap().doc.node(b).is_none() && paths(&s).len() == 1, "one path");
    // The join finished it: the next click starts a new path.
    gesture(&mut s, &[(500.0, 500.0)], none);
    assert_eq!(paths(&s).len(), 2);
    // From a path's first end, drawing goes on from there: the path is reversed.
    s.execute("select.none", &json!({})).unwrap();
    s.select_tool("selection", v).unwrap();
    s.select_tool("pen", v).unwrap();
    gesture(&mut s, &[(100.0, 100.0)], none);
    gesture(&mut s, &[(50.0, 50.0)], none);
    assert_eq!(pts(&s, a), [(400.0, 200.0), (300.0, 200.0), (200.0, 100.0), (100.0, 100.0), (50.0, 50.0)]);
}

/// `path.join {ids, ends}` joins the ends asked for, not the nearest pair; bad requests are errors.
#[test]
fn join_takes_the_paths_and_the_ends_to_join() {
    let mut s = session();
    let a = line(&mut s, 0.0, 0.0, 100.0, 0.0);
    let b = line(&mut s, 110.0, 0.0, 200.0, 0.0);
    // The far ends: a's first to b's last (the nearest pair would be a's last and b's first).
    s.execute("path.join", &json!({"ids": [a.0, b.0], "ends": ["first", "last"]})).unwrap();
    let pts: Vec<(f64, f64)> = path(&s, a).subpaths[0].anchors.iter().map(|x| (x.p.x, x.p.y)).collect();
    assert_eq!(pts, [(100.0, 0.0), (0.0, 0.0), (200.0, 0.0), (110.0, 0.0)]);
    let r = rect(&mut s, 0.0, 50.0, 10.0, 10.0);
    for bad in [
        json!({"ids": [a.0, a.0]}),
        json!({"ids": [a.0, 999]}),
        json!({"ids": [a.0, r.0], "ends": ["first"]}),
        json!({"ids": [a.0, b.0], "ends": ["first", "middle"]}),
    ] {
        assert!(s.execute("path.join", &bad).is_err(), "{bad}");
    }
}
