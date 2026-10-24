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

/// #3799: with Shift held, a wavy Pencil drag draws one straight segment at 45° steps from the
/// Constrain Angle (here 30°: a drag at about 60° comes out at 75°).
#[test]
fn pencil_with_shift_draws_a_straight_line() {
    let mut s = session();
    s.execute("prefs.set", &json!({"key": "constrainAngle", "value": 30})).unwrap();
    let v = view();
    let shift = Mods { shift: true, ..Default::default() };
    s.select_tool("pencil", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 400.0).with_mods(shift), v).unwrap();
    for (x, y) in [(120.0, 380.0), (125.0, 330.0), (160.0, 320.0), (150.0, 313.4)] {
        s.pointer(&PointerEvent::new(PointerKind::Drag, x, y).with_mods(shift), v).unwrap();
    }
    s.pointer(&PointerEvent::new(PointerKind::Up, 150.0, 313.4).with_mods(shift), v).unwrap();
    let ps = paths(&s);
    assert_eq!(ps.len(), 1);
    let sp = &ps[0].1.subpaths[0];
    assert_eq!(sp.anchors.len(), 2, "{sp:?}");
    assert!(sp.anchors.iter().all(|a| a.h_in == a.p && a.h_out == a.p), "no handles: {sp:?}");
    let (a, b) = (sp.anchors[0].p, sp.anchors[1].p);
    assert!(a.distance(Point::new(100.0, 400.0)) < 1e-6, "{a:?}");
    let angle = (-(b.y - a.y)).atan2(b.x - a.x).to_degrees();
    assert!((angle - 75.0).abs() < 1e-6, "{angle}");
}

/// #3799: with Alt held from the press to the release, a wavy Pencil drag draws one open straight
/// segment from the press point to the release point, at the angle the pointer took.
#[test]
fn pencil_with_alt_draws_an_open_straight_line() {
    let mut s = session();
    let v = view();
    let alt = Mods { alt: true, ..Default::default() };
    s.select_tool("pencil", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 400.0).with_mods(alt), v).unwrap();
    for (x, y) in [(120.0, 380.0), (125.0, 330.0), (160.0, 320.0), (170.0, 333.0)] {
        s.pointer(&PointerEvent::new(PointerKind::Drag, x, y).with_mods(alt), v).unwrap();
    }
    s.pointer(&PointerEvent::new(PointerKind::Up, 170.0, 333.0).with_mods(alt), v).unwrap();
    let ps = paths(&s);
    assert_eq!(ps.len(), 1);
    assert!(!ps[0].1.is_closed());
    let sp = &ps[0].1.subpaths[0];
    assert_eq!(sp.anchors.len(), 2, "{sp:?}");
    assert!(sp.anchors.iter().all(|a| a.h_in == a.p && a.h_out == a.p), "no handles: {sp:?}");
    assert!(sp.anchors[0].p.distance(Point::new(100.0, 400.0)) < 1e-6, "{sp:?}");
    assert!(sp.anchors[1].p.distance(Point::new(170.0, 333.0)) < 1e-6, "{sp:?}");
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

/// Edit Selected Paths in the middle of a path (#2850): a Pencil stroke that starts and ends on a
/// selected path replaces the section between, drawn either way; the path stays one path with
/// its ends, and is one undo step.
#[test]
fn pencil_redraws_the_section_of_a_selected_path_between_its_ends() {
    let bump = [(100.0, 101.0), (125.0, 70.0), (150.0, 60.0), (175.0, 70.0), (200.0, 101.0)];
    for reversed in [false, true] {
        let mut s = session();
        let id = line(&mut s, 0.0, 100.0, 300.0, 100.0);
        let mut pts = bump.to_vec();
        if reversed {
            pts.reverse();
        }
        let undo = undo_steps(&s);
        drag(&mut s, "pencil", &pts, Mods::default());
        assert_eq!(paths(&s).len(), 1, "no new object");
        assert_eq!(undo_steps(&s), undo + 1);
        let sp = &path(&s, id).subpaths[0];
        assert!(!sp.closed);
        let ps: Vec<Point> = sp.anchors.iter().map(|a| a.p).collect();
        assert!(near(ps[0], Point::new(0.0, 100.0)) && near(*ps.last().unwrap(), Point::new(300.0, 100.0)), "{ps:?}");
        // The section starts and ends on the line, where the stroke met it, and rises between.
        let on = |x: f64| ps.iter().any(|p| (p.x - x).abs() < 1.5 && (p.y - 100.0).abs() < 1e-6);
        assert!(on(100.0) && on(200.0), "{reversed}: {ps:?}");
        let top = kurbo::Shape::bounding_box(&path(&s, id).to_bezpath()).y0;
        assert!(top < 75.0, "the bump: top {top}");
        assert!(ps.windows(2).all(|w| w[0].x <= w[1].x + 1.0), "in order along the line: {ps:?}");
    }
}

/// On a closed path the stroke replaces the shorter way between where it starts and ends, and the
/// path stays closed; a stroke that doesn't end on the path draws a new one.
#[test]
fn pencil_redraws_the_shorter_way_round_a_closed_path() {
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 200.0, 100.0);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("path.freehand", &json!({"points": [[50, 1], [80, -25], [100, -30], [120, -25], [150, 1]], "redraw": {"id": id.0, "within": 4}}))
        .unwrap();
    assert_eq!(paths(&s).len(), 1);
    let sp = &path(&s, id).subpaths[0];
    assert!(sp.closed);
    let ps: Vec<Point> = sp.anchors.iter().map(|a| a.p).collect();
    for corner in [(0.0, 0.0), (200.0, 0.0), (200.0, 100.0), (0.0, 100.0)] {
        assert!(ps.iter().any(|p| near(*p, Point::new(corner.0, corner.1))), "{corner:?} kept: {ps:?}");
    }
    let top = kurbo::Shape::bounding_box(&path(&s, id).to_bezpath()).y0;
    assert!(top < -20.0, "the dip: top {top}");
    // Not ending on the path: a new path, the rectangle as it was.
    let before = path(&s, id);
    s.execute("path.freehand", &json!({"points": [[50, 100], [80, 150], [120, 160]], "redraw": {"id": id.0, "within": 4}})).unwrap();
    assert_eq!(paths(&s).len(), 2);
    assert_eq!(path(&s, id), before);
    assert!(s.execute("path.freehand", &json!({"points": [[0, 0], [1, 1]], "redraw": {"id": id.0, "within": -1}})).is_err());
}

/// With Keep selected off, the path a stroke draws or continues is not selected afterwards, the
/// rest of the selection stays, and the stroke is one undo step.
#[test]
fn pencil_with_keep_selected_off_leaves_the_new_path_unselected() {
    let mut s = session();
    let r = rect(&mut s, 400.0, 400.0, 50.0, 50.0);
    let selected = |s: &Session| s.doc().unwrap().selection.objects.clone();
    // On (the default): the new path replaces the selection.
    let on = s.execute("path.freehand", &json!({"points": [[100, 100], [150, 120], [200, 100]]})).unwrap();
    assert_eq!(selected(&s), vec![NodeId(on["id"].as_u64().unwrap())]);
    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    let undo = undo_steps(&s);
    let off = s.execute("path.freehand", &json!({"points": [[100, 200], [150, 220], [200, 200]], "keepSelected": false})).unwrap();
    let off = NodeId(off["id"].as_u64().unwrap());
    assert!(s.doc().unwrap().doc.node(off).is_some());
    assert_eq!(selected(&s), vec![r]);
    assert_eq!(undo_steps(&s), undo + 1);
    // Continuing a selected open path deselects it and keeps the rest.
    let l = line(&mut s, 100.0, 300.0, 200.0, 300.0);
    s.execute("select.set", &json!({"ids": [r.0, l.0]})).unwrap();
    s.execute("path.freehand", &json!({"points": [[200, 300], [250, 320], [300, 300]], "extend": {"id": l.0}, "keepSelected": false})).unwrap();
    assert!(path(&s, l).anchor_count() >= 3, "continued");
    assert_eq!(selected(&s), vec![r]);
    // The tools pass the option.
    s.execute("select.none", &json!({})).unwrap();
    s.select_tool("paintbrush", view()).unwrap();
    s.set_tool_option("keepSelected", &json!(false));
    let before = paths(&s).len();
    drag(&mut s, "paintbrush", &wave(100.0, 300.0, 500.0), Mods::default());
    assert_eq!(paths(&s).len(), before + 1, "a new brush path");
    assert!(selected(&s).is_empty());
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

/// Merge only with selection: a Blob Brush stroke merges with the selected blobs it touches and
/// leaves the others. Keep selected, off by default: the blob a stroke draws or merges into is not
/// selected afterwards, and the rest of the selection stays; on, it is selected.
#[test]
fn blob_brush_merges_only_with_the_selection_and_keep_selected_off_deselects() {
    let mut s = session();
    let r = rect(&mut s, 600.0, 400.0, 50.0, 50.0);
    let selected = |s: &Session| s.doc().unwrap().selection.objects.clone();
    let blob = |s: &mut Session, p: serde_json::Value| NodeId(s.execute("path.blob", &p).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    let a = blob(&mut s, json!({"points": [[100, 60], [100, 140]], "size": 20}));
    assert_eq!(selected(&s), vec![r], "Keep selected is off by default: the new blob is not selected");
    let b = blob(&mut s, json!({"points": [[200, 60], [200, 140]], "size": 20, "keepSelected": true}));
    assert_eq!(selected(&s), vec![b], "Keep selected on: the new blob is selected");
    let n = paths(&s).len();
    let (area_a, area_b) = (area(&path(&s, a)), area(&path(&s, b)));
    // Only `b` is selected: the stroke across both merges into `b` alone.
    let undo = undo_steps(&s);
    let across = json!({"points": [[50, 100], [250, 100]], "size": 10, "mergeOnlySelection": true, "keepSelected": false});
    assert_eq!(blob(&mut s, across.clone()), b);
    assert_eq!(paths(&s).len(), n);
    assert_eq!(area(&path(&s, a)), area_a, "a is left as it was");
    assert!(area(&path(&s, b)) > area_b + 500.0, "the stroke joined b");
    assert!(selected(&s).is_empty());
    assert_eq!(undo_steps(&s), undo + 1);
    // With nothing selected, the stroke merges with nothing, and Keep selected off leaves the new
    // blob unselected.
    let new = blob(&mut s, across);
    assert!(new != a && new != b && paths(&s).len() == n + 1, "a new blob");
    assert!(selected(&s).is_empty());
    // A selection that holds no blob: a new blob, and the selection stays.
    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    let other = blob(&mut s, json!({"points": [[50, 300], [250, 300]], "size": 10, "mergeOnlySelection": true, "keepSelected": false}));
    assert_eq!(paths(&s).len(), n + 2);
    assert_eq!(selected(&s), vec![r]);
    // Without the options, a stroke merges with every touching blob, selected or not, and the
    // blob it merges into is not selected.
    let all = blob(&mut s, json!({"points": [[50, 100], [250, 100]], "size": 10}));
    assert!([a, b, new].contains(&all) && paths(&s).len() == n, "one blob of a, b and the new one");
    assert_eq!(selected(&s), vec![r]);
    // Keep selected on: the blob a stroke merges into is selected.
    assert_eq!(blob(&mut s, json!({"points": [[150, 80], [150, 120]], "size": 10, "keepSelected": true})), all);
    assert_eq!(paths(&s).len(), n);
    assert_eq!(selected(&s), vec![all]);
    // The tool passes both options; Keep selected is off by default.
    s.execute("select.set", &json!({"ids": [other.0]})).unwrap();
    s.select_tool("blobBrush", view()).unwrap();
    assert_eq!(s.tool_options()["keepSelected"], json!(false));
    s.set_tool_option("mergeOnlySelection", &json!(true));
    drag(&mut s, "blobBrush", &[(150.0, 50.0), (150.0, 320.0)], Mods::default());
    assert_eq!(paths(&s).len(), n, "merged into the selected blob only");
    assert!(selected(&s).is_empty());
    assert!(s.doc().unwrap().doc.node(all).is_some() && s.doc().unwrap().doc.node(other).is_some());
}

/// Three points already curve in the saved path: both ends carry a handle into the curve.
#[test]
fn path_curvature_with_three_points_saves_end_handles() {
    let mut s = session();
    let pts = json!([{"x": 0, "y": 0}, {"x": 100, "y": -100}, {"x": 200, "y": 0}]);
    let id = NodeId(s.execute("path.curvature", &json!({ "points": pts })).unwrap()["id"].as_u64().unwrap());
    let sp = path(&s, id).subpaths[0].clone();
    assert_eq!(sp.anchors.len(), 3);
    let (first, mid, last) = (sp.anchors[0], sp.anchors[1], sp.anchors[2]);
    assert!(first.has_out() && !first.has_in(), "{first:?}");
    assert!(last.has_in() && !last.has_out(), "{last:?}");
    assert_eq!(mid.kind, vectorcraft_geom::AnchorKind::Smooth);
    assert!(!sp.segment_is_line(0) && !sp.segment_is_line(1));
    // Two points stay a straight line.
    let two = json!([{"x": 0, "y": 0}, {"x": 100, "y": -100}]);
    let id = NodeId(s.execute("path.curvature", &json!({ "points": two })).unwrap()["id"].as_u64().unwrap());
    assert!(path(&s, id).subpaths[0].segment_is_line(0));
}

/// The rubber band while drawing, and while going on from a selected path's end, is the path the
/// click then makes.
#[test]
fn curvature_rubber_band_is_the_path_the_click_makes() {
    let band = |s: &mut Session| {
        s.overlays(view()).into_iter().find_map(|o| if let vectorcraft_tools::Overlay::Path { path, .. } = o { Some(path) } else { None }).unwrap()
    };
    let bez = |sp: &vectorcraft_geom::SubPath| {
        let mut bp = kurbo::BezPath::new();
        sp.to_bezpath_into(&mut bp);
        bp
    };
    let mut s = session();
    let v = view();
    s.select_tool("curvature", v).unwrap();
    for (x, y) in [(100.0, 300.0), (130.0, 280.0), (300.0, 360.0)] {
        s.pointer(&PointerEvent::new(PointerKind::Down, x, y), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, x, y), v).unwrap();
    }
    s.pointer(&PointerEvent::new(PointerKind::Move, 420.0, 200.0), v).unwrap();
    let preview = band(&mut s);
    s.pointer(&PointerEvent::new(PointerKind::Down, 420.0, 200.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 420.0, 200.0), v).unwrap();
    let id = paths(&s)[0].0;
    let made = path(&s, id).subpaths[0].clone();
    assert_eq!(made.anchors.len(), 4);
    assert_eq!(preview, bez(&made));
    // Going on from the end of a selected pen path: the band is its last segment and the new one.
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    let anchors = json!([{"x": 100, "y": 500}, {"x": 200, "y": 420, "in": [160, 430], "out": [240, 410]}, {"x": 320, "y": 480, "in": [290, 440]}]);
    let id = NodeId(s.execute("path.create", &json!({ "anchors": anchors })).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 2]], "mode": "set"})).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Move, 450.0, 400.0), v).unwrap();
    let preview = band(&mut s);
    s.pointer(&PointerEvent::new(PointerKind::Down, 450.0, 400.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 450.0, 400.0), v).unwrap();
    let made = path(&s, id).subpaths[0].clone();
    assert_eq!(made.anchors.len(), 4);
    let tail = vectorcraft_geom::SubPath::new(made.anchors[1..].to_vec(), false);
    assert_eq!(preview, bez(&tail));
}

/// Without a handle position, `path.convertAnchor` to smooth curves the point as the Curvature
/// tool does: along the circle through it and its neighbours.
#[test]
fn convert_anchor_to_smooth_uses_the_curvature_rule() {
    let mut s = session();
    let anchors = json!([{"x": 0, "y": 0}, {"x": 10, "y": 0}, {"x": 200, "y": 100}]);
    let id = NodeId(s.execute("path.create", &json!({ "anchors": anchors })).unwrap()["id"].as_u64().unwrap());
    s.execute("path.convertAnchor", &json!({"id": id.0, "subpath": 0, "anchor": 1, "to": "smooth"})).unwrap();
    let a = path(&s, id).subpaths[0].anchors[1];
    assert_eq!(a.kind, vectorcraft_geom::AnchorKind::Smooth);
    // Along the circle through it and its neighbours, each between a third and two thirds of
    // its own chord; the shorter side's handle is the shorter.
    let (prev, next) = (Point::new(0.0, 0.0), Point::new(200.0, 100.0));
    for (h, to) in [(a.h_in, prev), (a.h_out, next)] {
        let r = a.p.distance(h) / a.p.distance(to);
        assert!((1.0 / 3.0 - 1e-9..=2.0 / 3.0 + 1e-9).contains(&r), "{r}");
    }
    assert!(a.p.distance(a.h_in) < a.p.distance(a.h_out));
    let (u_in, u_out) = ((a.p - a.h_in).normalize(), (a.h_out - a.p).normalize());
    assert!((u_in - u_out).hypot() < 1e-9, "the handles are in line");
    // Converting back takes the handles off.
    s.execute("path.convertAnchor", &json!({"id": id.0, "subpath": 0, "anchor": 1, "to": "corner"})).unwrap();
    assert!(!path(&s, id).subpaths[0].anchors[1].has_out());
    assert!(s.execute("path.convertAnchor", &json!({"id": id.0, "subpath": 0, "anchor": 9, "to": "smooth"})).is_err());
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

/// #2267: clicking one handle end with the Anchor Point tool retracts that handle alone, as one
/// undo step.
#[test]
fn anchor_point_tool_click_on_a_handle_retracts_it() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    drag(&mut s, "anchorPoint", &[(100.0, 100.0), (130.0, 80.0)], Mods::default());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let smooth = path(&s, id).subpaths[0].anchors[0];
    let undo = undo_steps(&s);
    click(&mut s, "anchorPoint", 130.0, 80.0);
    let a = path(&s, id).subpaths[0].anchors[0];
    assert!(!a.has_out());
    assert_eq!((a.p, a.h_in, a.kind), (smooth.p, smooth.h_in, vectorcraft_geom::AnchorKind::Corner));
    assert_eq!(undo_steps(&s), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(path(&s, id).subpaths[0].anchors[0], smooth);
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

/// #1512: a short drag over a small bump smooths the stretch of path under it. The long straight
/// sides next to the bump stay straight lines from their far ends, which keep their place.
#[test]
fn a_short_smooth_drag_leaves_the_rest_of_long_segments_alone() {
    let mut s = session();
    let anchors: Vec<_> =
        [(50.0, 200.0), (190.0, 200.0), (200.0, 192.0), (210.0, 200.0), (350.0, 200.0)].map(|(x, y)| json!({"x": x, "y": y})).to_vec();
    let r = s.execute("path.create", &json!({"anchors": anchors})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    let before = path(&s, id);
    drag(&mut s, "smooth", &[(195.0, 194.0), (205.0, 194.0)], Mods::default());
    let p = path(&s, id);
    assert_ne!(p, before, "the bump is smoothed");
    let a = &p.subpaths[0].anchors;
    let (first, second, last) = (a[0], a[1], a[a.len() - 1]);
    assert!(first.p == Point::new(50.0, 200.0) && !first.has_out() && !second.has_in(), "the left side is a line still: {first:?} {second:?}");
    assert!(last.p == Point::new(350.0, 200.0) && !last.has_in() && !a[a.len() - 2].has_out(), "and the right one: {last:?}");
    // Every other anchor is within the tool's reach of the drag (12 px either way).
    for x in &a[1..a.len() - 1] {
        assert!((182.0..=218.0).contains(&x.p.x) && (190.0..=201.0).contains(&x.p.y), "{x:?}");
    }
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

/// The children of the grid a `shape.rectangularGrid` / `shape.polarGrid` call made, bottom first.
fn grid(s: &mut Session, cmd: &str, p: serde_json::Value) -> Vec<std::sync::Arc<vectorcraft_doc::Node>> {
    let id = NodeId(s.execute(cmd, &p).unwrap()["id"].as_u64().unwrap());
    s.doc().unwrap().doc.node(id).unwrap().children().unwrap().to_vec()
}

/// `base` with the keys of `extra` added.
fn with(base: &serde_json::Value, extra: serde_json::Value) -> serde_json::Value {
    let mut p = base.clone();
    p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    p
}

fn is_filled(n: &vectorcraft_doc::Node) -> bool {
    n.appearance.fill().is_some_and(|f| f.paint != vectorcraft_color::Paint::None)
}

/// #1377: the Rectangular Grid options. By default the frame is one rectangle under evenly spaced
/// dividers and nothing is filled; `frame: false` draws its four sides instead, `fill` paints the
/// current fill and the skews crowd the dividers to the top / right (positive) or bottom / left.
#[test]
fn rectangular_grid_options_frame_fill_and_skew() {
    const CMD: &str = "shape.rectangularGrid";
    let mut s = session();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let base = json!({"x": 0, "y": 0, "width": 100, "height": 100, "rows": 3, "columns": 3});
    // The y of the three horizontal dividers and the x of the three vertical ones (the last six children).
    let dividers = |g: &[std::sync::Arc<vectorcraft_doc::Node>]| {
        let starts: Vec<Point> = g[g.len() - 6..].iter().map(|n| n.path_data().unwrap().subpaths[0].anchors[0].p).collect();
        starts[..3].iter().map(|p| p.y).chain(starts[3..].iter().map(|p| p.x)).collect::<Vec<_>>()
    };
    let plain = grid(&mut s, CMD, base.clone());
    assert_eq!(plain.len(), 7);
    assert!(plain[0].path_data().unwrap().is_closed(), "the frame lies under the dividers");
    assert!(plain.iter().all(|n| !is_filled(n) && n.appearance.stroke().is_some()));
    assert_eq!(dividers(&plain), [25.0, 50.0, 75.0, 25.0, 50.0, 75.0]);
    // No frame: its four sides are lines of their own.
    let sides = grid(&mut s, CMD, with(&base, json!({"frame": false})));
    assert_eq!(sides.len(), 10);
    assert!(sides.iter().all(|n| !n.path_data().unwrap().is_closed() && n.path_data().unwrap().anchor_count() == 2));
    assert_eq!(dividers(&sides), dividers(&plain));
    let filled = grid(&mut s, CMD, with(&base, json!({"fill": true})));
    assert!(filled.iter().all(|n| is_filled(n)), "Fill Grid paints the current fill");
    assert_eq!(filled[0].appearance.fill().unwrap().paint, s.paint.fill);
    // Skew: toward the top and the right when positive, the bottom and the left when negative;
    // past ±500 % is ±500 %.
    let top_right = dividers(&grid(&mut s, CMD, with(&base, json!({"rowSkew": 100, "columnSkew": 100}))));
    for (got, want) in top_right.iter().zip([6.25, 25.0, 56.25, 43.75, 75.0, 93.75]) {
        assert!((got - want).abs() < 1e-9, "{top_right:?}");
    }
    let bottom_left = dividers(&grid(&mut s, CMD, with(&base, json!({"rowSkew": -100, "columnSkew": -100}))));
    assert!(bottom_left[1] > 50.0 && bottom_left[4] < 50.0, "{bottom_left:?}");
    let most = dividers(&grid(&mut s, CMD, with(&base, json!({"rowSkew": 500, "columnSkew": -500}))));
    let beyond = dividers(&grid(&mut s, CMD, with(&base, json!({"rowSkew": 1e300, "columnSkew": -1e300}))));
    assert_eq!(most, beyond);
    assert!(most.iter().all(|v| *v > 0.0 && *v < 100.0), "{most:?}");
}

/// #1377: the Polar Grid options. The ellipses lie under the radial dividers, largest first;
/// `compound` pairs them into compound-path rings, `fill` paints the current fill and the skews
/// crowd the ellipses outward and the radial dividers clockwise of the top one (positive).
#[test]
fn polar_grid_options_compound_fill_and_skew() {
    const CMD: &str = "shape.polarGrid";
    let mut s = session();
    s.execute("paint.setFill", &json!({"color": "#00ff00"})).unwrap();
    let base = json!({"x": 0, "y": 0, "width": 100, "height": 100, "concentric": 3, "radial": 4});
    let width = |n: &vectorcraft_doc::Node| n.geometric_bounds().unwrap().width();
    let tips =
        |g: &[std::sync::Arc<vectorcraft_doc::Node>]| g[4..].iter().map(|n| n.path_data().unwrap().subpaths[0].anchors[1].p).collect::<Vec<_>>();
    let plain = grid(&mut s, CMD, base.clone());
    assert_eq!(plain.len(), 8);
    assert_eq!(plain[..4].iter().map(|n| width(n).round()).collect::<Vec<_>>(), [100.0, 75.0, 50.0, 25.0], "largest at the bottom");
    assert!(plain[..4].iter().all(|n| n.path_data().unwrap().is_closed()) && plain[4..].iter().all(|n| !n.path_data().unwrap().is_closed()));
    assert!(plain.iter().all(|n| !is_filled(n)));
    assert!(grid(&mut s, CMD, with(&base, json!({"fill": true}))).iter().all(|n| is_filled(n)));
    // Compound: 4 ellipses make 2 rings, each a compound path of an outside and a hole running
    // against it that carries the grid's paint itself; with 5, the innermost ellipse stays a path.
    let rings = grid(&mut s, CMD, with(&base, json!({"compound": true, "fill": true})));
    assert_eq!(rings.len(), 6);
    for (ring, sizes) in rings[..2].iter().zip([[100.0, 75.0], [50.0, 25.0]]) {
        let NodeKind::Compound { children, rule } = &ring.kind else { panic!("{:?}", ring.kind) };
        assert_eq!(*rule, FillRule::NonZero);
        assert_eq!(children.iter().map(|n| width(n).round()).collect::<Vec<_>>(), sizes);
        let areas: Vec<f64> = children.iter().map(|n| n.path_data().unwrap().subpaths[0].signed_area()).collect();
        assert!(areas[0] * areas[1] < 0.0, "the hole runs against the outside: {areas:?}");
        assert!(is_filled(ring) && ring.appearance.stroke().is_some());
    }
    let five = grid(&mut s, CMD, with(&base, json!({"compound": true, "concentric": 4, "radial": 0})));
    assert_eq!(five.iter().map(|n| matches!(n.kind, NodeKind::Compound { .. })).collect::<Vec<_>>(), [true, true, false]);
    // Skew.
    let out = grid(&mut s, CMD, with(&base, json!({"concentricSkew": 100, "radialSkew": 500})));
    for (n, want) in out[..4].iter().zip([100.0, 93.75, 75.0, 43.75]) {
        assert!((width(n) - want).abs() < 1e-6, "{}", width(n));
    }
    assert!(tips(&out)[0].distance(Point::new(50.0, 0.0)) < 1e-9, "the top divider stays");
    assert!(tips(&out)[1..].iter().all(|t| t.x > 50.0), "clockwise of the top: {:?}", tips(&out));
    let inward = grid(&mut s, CMD, with(&base, json!({"concentricSkew": -100, "radialSkew": -500})));
    assert!((width(&inward[1]) - 56.25).abs() < 1e-6);
    assert!(tips(&inward)[1..].iter().all(|t| t.x < 50.0), "counterclockwise of the top: {:?}", tips(&inward));
}

/// The grid options are checked: a divider count or a skew is a number and a switch true or false,
/// or the command fails without an undo step.
#[test]
fn grid_commands_reject_options_of_the_wrong_type() {
    let mut s = session();
    let size = json!({"x": 0, "y": 0, "width": 10, "height": 10});
    for (cmd, key, value) in [
        ("shape.rectangularGrid", "rows", json!("3")),
        ("shape.rectangularGrid", "columns", json!([4])),
        ("shape.polarGrid", "concentric", json!(true)),
        ("shape.polarGrid", "radial", json!("6")),
        ("shape.rectangularGrid", "rowSkew", json!("50")),
        ("shape.rectangularGrid", "columnSkew", json!(true)),
        ("shape.rectangularGrid", "frame", json!(1)),
        ("shape.rectangularGrid", "fill", json!("yes")),
        ("shape.polarGrid", "concentricSkew", json!([1])),
        ("shape.polarGrid", "radialSkew", json!({})),
        ("shape.polarGrid", "compound", json!(0)),
        ("shape.polarGrid", "fill", json!("true")),
    ] {
        let before = s.doc().unwrap().history.undo.len();
        let p = with(&size, json!({key: value}));
        assert!(s.execute(cmd, &p).is_err(), "{cmd} {p}");
        assert_eq!(s.doc().unwrap().history.undo.len(), before, "{cmd} left an undo step");
        assert!(s.execute(cmd, &with(&size, json!({key: null}))).is_ok(), "{cmd}: null is the default");
    }
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

/// The Pencil refuses a `fidelity` or a redraw `within` given as text or as another value that is
/// not a number, and draws or redraws nothing; null is read as the key left out.
#[test]
fn pencil_refuses_a_fidelity_or_within_given_as_text() {
    let mut s = session();
    let id = line(&mut s, 100.0, 100.0, 300.0, 100.0);
    let stroke = json!({"points": [[100, 100], [150, 140], [200, 140], [250, 100]]});
    refuses_a_number_given_as_text(&mut s, "path.freehand", &stroke, "fidelity");
    for v in [json!("12"), json!([12]), json!(false)] {
        let p = with(&stroke, json!({"redraw": {"id": id.0, "within": v}}));
        refuses_with(&mut s, "path.freehand", &p, &format!("`within` must be a number, not {v}"));
    }
    let before = path(&s, id);
    s.execute("path.freehand", &with(&stroke, json!({"fidelity": null, "redraw": {"id": id.0, "within": null}}))).unwrap();
    assert!(path(&s, id) != before, "the stroke redraws the line within 12 pt");
}

/// The drawing tools' commands refuse a point whose x or y is not a number, such as "10" or a point
/// written as text, and a pen pressure given as text, instead of leaving the point out or drawing it
/// at full pressure; the document stays as it was.
#[test]
fn drawing_commands_refuse_a_point_or_pressure_given_as_text() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let good = [json!([90, 150]), json!([150, 150]), json!([210, 150])];
    let cmds = ["path.freehand", "path.knife", "path.eraseRegion", "path.eraseSegments", "path.smoothRegion", "path.joinScrub", "path.blob"];
    for cmd in cmds.into_iter().chain(["path.curvature"]) {
        for bad in [json!(["150", 150]), json!({"x": 150, "y": "150"}), json!("150, 150"), json!([150]), json!(null)] {
            let points: Vec<_> = good.iter().cloned().chain([bad]).collect();
            refuses_with(&mut s, cmd, &json!({"points": points}), "x and y must both be numbers");
        }
    }
    for cmd in cmds {
        for bad in [json!([150, 150, "0.5"]), json!({"x": 150, "y": 150, "pressure": "0.5"})] {
            let points: Vec<_> = good.iter().cloned().chain([bad]).collect();
            refuses_with(&mut s, cmd, &json!({"points": points}), "`pressure` must be a number, not \"0.5\"");
        }
    }
    let pressed = json!({"points": [[100, 400], [150, 420, null], {"x": 200, "y": 400, "pressure": 0.5}], "keepSelected": false});
    s.execute("path.freehand", &pressed).unwrap();
}

/// The Eraser, Path Eraser, Smooth, Join and Blob Brush commands refuse a size, width, radius,
/// fidelity or tolerance given as text or as another value that is not a number, and change
/// nothing; null is read as the key left out.
#[test]
fn drawing_commands_refuse_a_size_given_as_text() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let stroke = json!({"points": [[90, 150], [150, 150], [210, 150]]});
    for (cmd, key) in [
        ("path.eraseRegion", "size"),
        ("path.eraseSegments", "width"),
        ("path.smoothRegion", "radius"),
        ("path.smoothRegion", "fidelity"),
        ("path.joinScrub", "tolerance"),
        ("path.blob", "size"),
    ] {
        refuses_a_number_given_as_text(&mut s, cmd, &stroke, key);
    }
    let erased = s.execute("path.eraseRegion", &with(&stroke, json!({"size": null}))).unwrap();
    assert_eq!(erased["ids"].as_array().map(Vec::len), Some(2), "the default 10 pt Eraser cuts the square in two: {erased}");
}

/// A path of two open subpaths, each of three anchors from x 100 to x 300 with a middle anchor that
/// has two handles: subpath 0 at y 100, subpath 1 at y 300.
fn two_subpaths(s: &mut Session) -> NodeId {
    let id = curve(s);
    let sub = |y: f64| {
        let anchors = json!([
            {"x": 100, "y": y, "out": [130, y - 30.0]},
            {"x": 200, "y": y, "in": [170, y - 30.0], "out": [230, y - 30.0]},
            {"x": 300, "y": y, "in": [270, y - 30.0]},
        ]);
        json!({"anchors": anchors, "closed": false})
    };
    s.execute("path.setAnchors", &json!({"id": id.0, "subpaths": [sub(100.0), sub(300.0)]})).unwrap();
    id
}

/// Runs `cmd` with `p` and `key` set to each of 1.5, -1 and "1": each call fails with an error that
/// names `key`, and the document and the undo history stay as they were.
fn refuses_an_index_that_is_not_whole(s: &mut Session, cmd: &str, p: &serde_json::Value, key: &str) {
    for v in [json!(1.5), json!(-1), json!("1")] {
        let mut p = p.clone();
        p[key] = v;
        let before = s.doc().unwrap().doc.clone();
        let undo = undo_steps(s);
        let Err(e) = s.execute(cmd, &p) else { panic!("{cmd} {p} succeeded") };
        assert!(e.to_string().contains(&format!("`{key}`")), "{cmd} {p}: {e}");
        assert!(s.doc().unwrap().doc == before, "{cmd} {p} changed the document");
        assert_eq!(undo_steps(s), undo, "{cmd} {p} left an undo step");
    }
}

/// Runs `cmd` with `p` and `key` set to each of "10", [10] and true: each call fails with
/// "`key` must be a number, not <value>", and the document and the undo history stay as they were.
fn refuses_a_number_given_as_text(s: &mut Session, cmd: &str, p: &serde_json::Value, key: &str) {
    for v in [json!("10"), json!([10]), json!(true)] {
        let mut p = p.clone();
        p[key] = v.clone();
        let before = s.doc().unwrap().doc.clone();
        let undo = undo_steps(s);
        let r = s.execute(cmd, &p);
        let want = format!("`{key}` must be a number, not {v}");
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == want), "{cmd} {p}: {r:?}");
        assert!(s.doc().unwrap().doc == before, "{cmd} {p} changed the document");
        assert_eq!(undo_steps(s), undo, "{cmd} {p} left an undo step");
    }
}

/// Runs `cmd` with `p`: the call fails with `want`, and the document and the undo history stay as
/// they were.
fn refuses_with(s: &mut Session, cmd: &str, p: &serde_json::Value, want: &str) {
    let before = s.doc().unwrap().doc.clone();
    let undo = undo_steps(s);
    let r = s.execute(cmd, p);
    assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg == want), "{cmd} {p}: {r:?}");
    assert!(s.doc().unwrap().doc == before, "{cmd} {p} changed the document");
    assert_eq!(undo_steps(s), undo, "{cmd} {p} left an undo step");
}

/// Runs `cmd` with `p` on path `id` (made by [`two_subpaths`]) with `subpath: 1`, without `subpath`
/// and with `subpath: null`, undoing each: the first changes subpath 1 alone, and the others change
/// subpath 0 alone.
fn edits_the_subpath_given(s: &mut Session, cmd: &str, p: &serde_json::Value, id: NodeId) {
    let before = path(s, id).subpaths;
    for (si, p) in [(1, with(p, json!({"subpath": 1}))), (0, p.clone()), (0, with(p, json!({"subpath": null})))] {
        s.execute(cmd, &p).unwrap();
        let after = path(s, id).subpaths;
        assert_eq!(after.len(), 2, "{cmd} {p}");
        assert!(after[si] != before[si] && after[1 - si] == before[1 - si], "{cmd} {p}: {after:?}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(path(s, id).subpaths, before, "{cmd} {p}: undo");
    }
}

#[test]
fn curvature_edit_refuses_a_subpath_that_is_not_a_whole_number() {
    let mut s = session();
    let id = two_subpaths(&mut s);
    let p = json!({"id": id.0, "op": "move", "anchor": 1, "x": 200, "y": 150});
    refuses_an_index_that_is_not_whole(&mut s, "path.curvatureEdit", &p, "subpath");
    edits_the_subpath_given(&mut s, "path.curvatureEdit", &p, id);
}

#[test]
fn remove_anchor_refuses_a_subpath_that_is_not_a_whole_number() {
    let mut s = session();
    let id = two_subpaths(&mut s);
    let p = json!({"id": id.0, "anchor": 1});
    refuses_an_index_that_is_not_whole(&mut s, "path.removeAnchor", &p, "subpath");
    edits_the_subpath_given(&mut s, "path.removeAnchor", &p, id);
}

#[test]
fn convert_anchor_refuses_a_subpath_that_is_not_a_whole_number() {
    let mut s = session();
    let id = two_subpaths(&mut s);
    let p = json!({"id": id.0, "anchor": 1, "to": "corner"});
    refuses_an_index_that_is_not_whole(&mut s, "path.convertAnchor", &p, "subpath");
    edits_the_subpath_given(&mut s, "path.convertAnchor", &p, id);
}

#[test]
fn reshape_segment_refuses_a_subpath_that_is_not_a_whole_number() {
    let mut s = session();
    let id = two_subpaths(&mut s);
    let p = json!({"id": id.0, "segment": 0, "t": 0.5, "dx": 0, "dy": 20});
    refuses_an_index_that_is_not_whole(&mut s, "path.reshapeSegment", &p, "subpath");
    edits_the_subpath_given(&mut s, "path.reshapeSegment", &p, id);
}

#[test]
fn split_refuses_a_subpath_anchor_or_segment_that_is_not_a_whole_number() {
    // A bad `anchor` beside a good `segment` is refused, and so is a bad `segment` beside a good
    // `anchor`.
    let mut s = session();
    let id = two_subpaths(&mut s);
    let at_anchor = json!({"id": id.0, "anchor": 1});
    let at_segment = json!({"id": id.0, "segment": 0, "t": 0.5});
    refuses_an_index_that_is_not_whole(&mut s, "path.split", &at_anchor, "subpath");
    refuses_an_index_that_is_not_whole(&mut s, "path.split", &at_segment, "subpath");
    refuses_an_index_that_is_not_whole(&mut s, "path.split", &at_segment, "anchor");
    refuses_an_index_that_is_not_whole(&mut s, "path.split", &at_anchor, "segment");
    let before = path(&s, id).subpaths;
    for (si, p) in [(1, with(&at_anchor, json!({"subpath": 1}))), (0, at_anchor.clone()), (0, with(&at_anchor, json!({"subpath": null})))] {
        s.execute("path.split", &p).unwrap();
        let after = path(&s, id).subpaths;
        // The subpath split becomes two of two anchors each, in its place.
        let kept = if si == 1 { &after[0] } else { &after[2] };
        assert!(after.len() == 3 && after[si].anchors.len() == 2 && after[si + 1].anchors.len() == 2, "{p}: {after:?}");
        assert_eq!(kept, &before[1 - si], "{p}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(path(&s, id).subpaths, before, "{p}: undo");
    }
}

/// A subpath, anchor or segment of 2^32 + k fails, where k names one that exists, and the document
/// and the undo history stay as they were. A 32-bit build read the number as k.
#[test]
fn path_commands_refuse_a_subpath_anchor_or_segment_past_u32() {
    let mut s = session();
    let id = two_subpaths(&mut s);
    let at_anchor = json!({"id": id.0, "anchor": 1});
    let at_segment = json!({"id": id.0, "segment": 0, "t": 0.5});
    let curvature_move = json!({"id": id.0, "op": "move", "anchor": 1, "x": 200, "y": 150});
    let curvature_insert = json!({"id": id.0, "op": "insert", "segment": 0, "t": 0.5});
    let convert = json!({"id": id.0, "anchor": 1, "to": "corner"});
    let reshape = json!({"id": id.0, "segment": 0, "t": 0.5, "dx": 0, "dy": 20});
    let handle = json!({"id": id.0, "anchor": 0, "which": "out", "x": 140, "y": 250});
    let moved = json!({"id": id.0, "anchor": 0, "x": 140, "y": 250});
    for (cmd, p, key) in [
        ("path.curvatureEdit", &curvature_move, "subpath"),
        ("path.curvatureEdit", &curvature_move, "anchor"),
        ("path.curvatureEdit", &curvature_insert, "segment"),
        ("path.removeAnchor", &at_anchor, "subpath"),
        ("path.removeAnchor", &at_anchor, "anchor"),
        ("path.convertAnchor", &convert, "subpath"),
        ("path.convertAnchor", &convert, "anchor"),
        ("path.reshapeSegment", &reshape, "subpath"),
        ("path.reshapeSegment", &reshape, "segment"),
        ("path.split", &at_anchor, "anchor"),
        ("path.split", &at_segment, "segment"),
        ("path.setHandle", &handle, "subpath"),
        ("path.setHandle", &handle, "anchor"),
        ("path.moveAnchor", &moved, "subpath"),
        ("path.moveAnchor", &moved, "anchor"),
        ("path.insertAnchor", &at_segment, "subpath"),
        ("path.insertAnchor", &at_segment, "segment"),
    ] {
        let mut p = p.clone();
        let k = if key == "subpath" { 1 } else { p[key].as_u64().unwrap() };
        p[key] = json!((1u64 << 32) + k);
        let before = s.doc().unwrap().doc.clone();
        let undo = undo_steps(&s);
        assert!(s.execute(cmd, &p).is_err(), "{cmd} {p} succeeded");
        assert!(s.doc().unwrap().doc == before, "{cmd} {p} changed the document");
        assert_eq!(undo_steps(&s), undo, "{cmd} {p} left an undo step");
    }
}

#[test]
fn set_handle_refuses_a_subpath_that_is_not_a_whole_number() {
    let mut s = session();
    let id = two_subpaths(&mut s);
    let p = json!({"id": id.0, "anchor": 0, "which": "out", "x": 140, "y": 250});
    refuses_an_index_that_is_not_whole(&mut s, "path.setHandle", &p, "subpath");
    edits_the_subpath_given(&mut s, "path.setHandle", &p, id);
}

#[test]
fn move_anchor_takes_its_handles_along_and_refuses_a_subpath_that_is_not_a_whole_number() {
    let mut s = session();
    let id = two_subpaths(&mut s);
    let p = json!({"id": id.0, "anchor": 0, "x": 140, "y": 250});
    refuses_an_index_that_is_not_whole(&mut s, "path.moveAnchor", &p, "subpath");
    edits_the_subpath_given(&mut s, "path.moveAnchor", &p, id);
    // The anchor lands on (x, y) and its handles keep their offsets from it.
    s.execute("path.setHandle", &json!({"id": id.0, "anchor": 0, "which": "out", "x": 30, "y": 40})).unwrap();
    let a = path(&s, id).subpaths[0].anchors[0];
    s.execute("path.moveAnchor", &json!({"id": id.0, "anchor": 0, "x": a.p.x + 15.0, "y": a.p.y - 5.0})).unwrap();
    let b = path(&s, id).subpaths[0].anchors[0];
    assert_eq!(b.p, a.p + vectorcraft_geom::Vec2::new(15.0, -5.0));
    assert_eq!(b.h_out - b.p, a.h_out - a.p);
    assert_eq!(b.h_in - b.p, a.h_in - a.p);
    assert!(s.execute("path.moveAnchor", &json!({"id": id.0, "anchor": 99, "x": 0, "y": 0})).is_err());
}

#[test]
fn insert_anchor_refuses_a_subpath_that_is_not_a_whole_number() {
    let mut s = session();
    let id = two_subpaths(&mut s);
    let p = json!({"id": id.0, "segment": 0, "t": 0.5});
    refuses_an_index_that_is_not_whole(&mut s, "path.insertAnchor", &p, "subpath");
    edits_the_subpath_given(&mut s, "path.insertAnchor", &p, id);
}

/// `path.reshapeSegment` refuses a `t`, `dx` or `dy` given as text or as another value that is not
/// a number, instead of bending the segment by 0 or at its middle; `dy: 20` bends it, with a null
/// `t` read as the middle.
#[test]
fn reshape_segment_refuses_a_number_given_as_text() {
    let mut s = session();
    let id = curve(&mut s);
    let p = json!({"id": id.0, "segment": 0, "t": 0.5, "dx": 0, "dy": 20});
    for key in ["t", "dx", "dy"] {
        refuses_a_number_given_as_text(&mut s, "path.reshapeSegment", &p, key);
    }
    let before = path(&s, id);
    s.execute("path.reshapeSegment", &with(&p, json!({"t": null}))).unwrap();
    assert!(path(&s, id) != before, "the segment bends");
}

/// `path.convertAnchor` refuses an `x` or `y` given as text, and an `x` without a `y` or a `y`
/// without an `x`, instead of converting the anchor without a handle position; both as numbers
/// place the out handle there.
#[test]
fn convert_anchor_refuses_a_handle_position_given_as_text_or_alone() {
    let mut s = session();
    let id = curve(&mut s);
    let p = json!({"id": id.0, "anchor": 1, "to": "smooth", "x": 330, "y": 150});
    for key in ["x", "y"] {
        refuses_a_number_given_as_text(&mut s, "path.convertAnchor", &p, key);
    }
    for lone in [json!({"y": null}), json!({"x": null}), json!({"y": null, "to": "corner"})] {
        refuses_with(&mut s, "path.convertAnchor", &with(&p, lone), "x and y must be given together");
    }
    s.execute("path.convertAnchor", &p).unwrap();
    assert_eq!(path(&s, id).subpaths[0].anchors[1].h_out, Point::new(330.0, 150.0));
}

/// The Curvature tool's edits and Add Anchor Point refuse a `t`, `x` or `y` given as text or as
/// another value that is not a number, instead of inserting at the middle of the segment or leaving
/// the new anchor where it was inserted.
#[test]
fn curvature_edit_and_insert_anchor_refuse_a_number_given_as_text() {
    let mut s = session();
    let id = curve(&mut s);
    let insert = json!({"id": id.0, "op": "insert", "segment": 0, "t": 0.25});
    let moved = json!({"id": id.0, "op": "insert", "segment": 0, "t": 0.25, "x": 150, "y": 160});
    refuses_a_number_given_as_text(&mut s, "path.curvatureEdit", &insert, "t");
    for key in ["x", "y"] {
        refuses_a_number_given_as_text(&mut s, "path.curvatureEdit", &moved, key);
        refuses_a_number_given_as_text(&mut s, "path.curvatureEdit", &json!({"id": id.0, "op": "move", "anchor": 1, "x": 300, "y": 150}), key);
    }
    refuses_a_number_given_as_text(&mut s, "path.insertAnchor", &json!({"id": id.0, "segment": 0, "t": 0.25}), "t");
    s.execute("path.curvatureEdit", &moved).unwrap();
    assert_eq!(path(&s, id).subpaths[0].anchors[1].p, Point::new(150.0, 160.0));
    s.execute("path.insertAnchor", &json!({"id": id.0, "segment": 0, "t": null})).unwrap();
    assert_eq!(path(&s, id).subpaths[0].anchors.len(), 4, "a null t inserts at the middle");
}

/// Move Anchors, the Reshape tool's command and Close Path refuse a number or handle given as text
/// or as another value that is not a number, and change nothing; null is read as the key left out.
#[test]
fn move_anchors_reshape_and_close_refuse_a_number_given_as_text() {
    let mut s = session();
    let id = curve(&mut s);
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 1]]})).unwrap();
    let nudge = json!({"dx": 10, "dy": 0});
    for key in ["dx", "dy"] {
        refuses_a_number_given_as_text(&mut s, "path.moveAnchors", &nudge, key);
    }
    let reshape = json!({"id": id.0, "x": 200, "y": 100, "dx": 0, "dy": 30, "tol": 3, "radius": 50});
    for key in ["dx", "dy", "tol", "radius"] {
        refuses_a_number_given_as_text(&mut s, "path.reshape", &reshape, key);
    }
    for h in [json!("200, 150"), json!([200]), json!([200, "150"]), json!({"x": 200, "y": 150})] {
        refuses_with(&mut s, "path.close", &json!({"id": id.0, "in": h}), "`in` must be [x, y]");
    }
    let before = path(&s, id);
    s.execute("path.moveAnchors", &json!({"dx": 10, "dy": null})).unwrap();
    s.execute("path.reshape", &with(&reshape, json!({"radius": null}))).unwrap();
    s.execute("path.close", &json!({"id": id.0, "in": null})).unwrap();
    // The Reshape tool's grab adds an anchor between the two, so the moved anchor is the last.
    let after = path(&s, id);
    let moved = after.subpaths[0].anchors.last().map(|a| a.p.x);
    assert!(after.subpaths[0].closed && moved == Some(before.subpaths[0].anchors[1].p.x + 10.0), "{after:?}");
}

/// Close Path, Set Handle, Move Anchors and the Pencil, Curvature, Reshape Segment and Blob Brush
/// commands refuse an on/off option given as text or as another value that is not true or false,
/// naming the key, and change nothing; true, false and null still work.
#[test]
fn path_commands_refuse_a_boolean_given_as_text() {
    let mut s = session();
    let id = curve(&mut s);
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 1]]})).unwrap();
    let stroke = json!([[100, 300], [150, 340], [200, 300]]);
    let calls = [
        ("path.close", json!({"id": id.0, "independent": "true"}), "independent"),
        ("path.setHandle", json!({"id": id.0, "anchor": 0, "x": 160, "y": 90, "independent": 1}), "independent"),
        ("path.moveAnchors", json!({"dx": 10, "patterns": "true"}), "patterns"),
        ("path.freehand", json!({"points": stroke, "closed": "true"}), "closed"),
        ("path.freehand", json!({"points": stroke, "keepSelected": "false"}), "keepSelected"),
        ("path.freehand", json!({"points": stroke, "fill": [true]}), "fill"),
        ("path.curvature", json!({"points": stroke, "closed": "true"}), "closed"),
        ("path.curvature", json!({"points": [{"x": 100, "y": 300, "corner": "true"}, [150, 340], [200, 300]]}), "corner"),
        ("path.curvatureEdit", json!({"id": id.0, "op": "move", "anchor": 1, "x": 300, "y": 150, "corner": "true"}), "corner"),
        ("path.reshapeSegment", json!({"id": id.0, "segment": 0, "dy": 20, "perpendicular": "true"}), "perpendicular"),
        ("path.blob", json!({"points": stroke, "keepSelected": "true"}), "keepSelected"),
        ("path.blob", json!({"points": stroke, "mergeOnlySelection": 1}), "mergeOnlySelection"),
        ("path.blob", json!({"points": stroke, "merge": "false"}), "merge"),
    ]
    .map(|(cmd, p, key)| super::tests::not_a_switch(cmd, p, key));
    let wrong = super::tests::not_refused(&mut s, &calls);
    assert!(wrong.is_empty(), "not refused: {wrong:#?}");
    s.execute("path.setHandle", &json!({"id": id.0, "anchor": 0, "x": 160, "y": 90, "independent": true})).unwrap();
    s.execute("path.close", &json!({"id": id.0, "independent": null})).unwrap();
    s.execute("path.freehand", &json!({"points": stroke, "closed": false, "keepSelected": null, "fill": true})).unwrap();
    s.execute("path.blob", &json!({"points": stroke, "merge": false, "keepSelected": null, "mergeOnlySelection": false})).unwrap();
    assert!(path(&s, id).subpaths[0].closed);
}

#[test]
fn path_commands_refuse_a_delta_or_point_of_1e9_pt_or_more() {
    // `path.reshapeSegment` refuses a delta of 1e9 pt or longer, and `path.convertAnchor` and
    // `path.freehand` refuse a point whose x or y is 1e9 pt or more in size, also for a path that is
    // not selected: the document stays as it was, with no undo step. A delta just under that bound,
    // at t 0.05 or 0.95, leaves every handle and the path's bounds finite.
    let mut s = session();
    let id = curve(&mut s);
    s.execute("select.none", &json!({})).unwrap();
    let finite = |s: &Session| {
        paths(s).iter().all(|(_, pd)| {
            let mut points = pd.subpaths.iter().flat_map(|sp| &sp.anchors).flat_map(|a| [a.p, a.h_in, a.h_out]);
            points.all(|q| q.x.is_finite() && q.y.is_finite()) && pd.control_bounds().is_none_or(|b| b.width().is_finite() && b.height().is_finite())
        })
    };
    let h = 1e308;
    for (cmd, p) in [
        ("path.reshapeSegment", json!({"id": id.0, "segment": 0, "t": 0.95, "dx": h, "dy": 0})),
        ("path.reshapeSegment", json!({"id": id.0, "segment": 0, "t": 0.05, "dx": 0, "dy": -h})),
        ("path.reshapeSegment", json!({"id": id.0, "segment": 0, "t": 0.95, "dx": 0, "dy": h, "perpendicular": true})),
        ("path.reshapeSegment", json!({"id": id.0, "segment": 0, "t": 0.5, "dx": 1e9, "dy": 0})),
        ("path.convertAnchor", json!({"id": id.0, "anchor": 0, "to": "smooth", "x": h, "y": -h})),
        ("path.freehand", json!({"points": [[0, 0], [h, 0], [-h, h], [h, h]], "keepSelected": false})),
        ("path.freehand", json!({"points": [[0, 0], [h, 0], [-h, h]], "keepSelected": false, "extend": {"id": id.0, "end": "end"}})),
    ] {
        let before = s.doc().unwrap().doc.clone();
        let undo = s.doc().unwrap().history.undo.len();
        assert!(s.execute(cmd, &p).is_err(), "{cmd} {p}");
        assert!(s.doc().unwrap().doc == before && finite(&s), "{cmd} {p} changed the document");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo, "{cmd} left an undo step");
    }
    for (t, perpendicular) in [(0.05, false), (0.95, false), (0.05, true), (0.95, true)] {
        let p = json!({"id": id.0, "segment": 0, "t": t, "dx": 0, "dy": 9.9e8, "perpendicular": perpendicular});
        s.execute("path.reshapeSegment", &p).unwrap();
        assert!(finite(&s), "{p}: {:?}", path(&s, id));
    }
}

#[test]
fn a_selected_curve_whose_handles_are_near_1e308_is_refused() {
    // The canvas check measures a selected curve whose handles are near ±1e308 by its control
    // points, so `path.create` refuses such a curve, given as anchors or as path data: the document
    // stays as it was, with no undo step.
    let mut s = session();
    curve(&mut s);
    let h = 1e308;
    for p in [
        json!({"anchors": [{"x": 100, "y": 100, "out": [150, h]}, {"x": 300, "y": 100, "in": [250, h]}]}),
        json!({"anchors": [{"x": 100, "y": 100, "out": [150, -h]}, {"x": 300, "y": 100, "in": [250, h]}]}),
        json!({"anchors": [{"x": 100, "y": 100, "out": [1.7e308, 100]}, {"x": 300, "y": 100}]}),
        json!({"d": "M100 100 C150 1e308 250 1e308 300 100"}),
    ] {
        let before = s.doc().unwrap().doc.clone();
        let undo = s.doc().unwrap().history.undo.len();
        let e = s.execute("path.create", &p).unwrap_err().to_string();
        assert!(e.contains("exceed the canvas"), "{p}: {e}");
        assert!(s.doc().unwrap().doc == before, "{p} changed the document");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo, "{p} left an undo step");
    }
    // A curve whose handles stay on the canvas is kept.
    let p = json!({"anchors": [{"x": 100, "y": 100, "out": [150, 3e6]}, {"x": 300, "y": 100, "in": [250, 3e6]}]});
    let id = NodeId(s.execute("path.create", &p).unwrap()["id"].as_u64().unwrap());
    assert!(path(&s, id).bounds().is_some_and(|b| b.y1 > 2e6));
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

fn selected_anchors(s: &Session, id: NodeId) -> Vec<(usize, usize)> {
    s.doc().unwrap().selection.anchors.get(&id).map(|set| set.iter().copied().collect()).unwrap_or_default()
}

fn anchor_points(s: &Session, id: NodeId) -> Vec<Point> {
    path(s, id).subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p)).collect()
}

#[test]
fn remove_anchor_points_selects_the_next_anchor_so_it_can_go_on_along_the_path() {
    // #2241: after Remove Anchor Points the next anchor along is selected, so running it again
    // removes that one without picking it first. The path stays selected the whole way.
    let mut s = session();
    let anchors: Vec<_> = (0..5).map(|i| json!({"x": 100 * i, "y": 100 * (i % 2)})).collect();
    let id = NodeId(s.execute("path.create", &json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap());
    let original = anchor_points(&s, id);
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 1]]})).unwrap();
    for (remaining, next) in [(4, 2), (3, 3), (2, 4)] {
        assert_eq!(s.execute("path.removeAnchors", &json!({})).unwrap()["removedObjects"], 0);
        assert_eq!(path(&s, id).subpaths[0].anchors.len(), remaining);
        assert_eq!(selected_anchors(&s, id), [(0, 1)], "anchor {next} of the original is selected");
        assert_eq!(path(&s, id).subpaths[0].anchors[1].p, original[next]);
        assert_eq!(s.doc().unwrap().selection.objects, [id]);
    }
    // One anchor can't make a path: the last run removes it, and nothing is left selected.
    assert_eq!(s.execute("path.removeAnchors", &json!({})).unwrap()["removedObjects"], 1);
    assert!(s.doc().unwrap().doc.node(id).is_none());
    assert!(s.doc().unwrap().selection.is_empty());
}

#[test]
fn remove_anchor_points_selects_the_anchor_before_an_open_end_and_wraps_a_closed_path() {
    // An open path's last anchor has nothing after it: the one before is selected.
    let mut s = session();
    let anchors: Vec<_> = (0..4).map(|i| json!({"x": 100 * i, "y": 0})).collect();
    let open = NodeId(s.execute("path.create", &json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap());
    let original = anchor_points(&s, open);
    s.execute("select.anchors", &json!({"id": open.0, "anchors": [[0, 3]]})).unwrap();
    s.execute("path.removeAnchors", &json!({})).unwrap();
    assert_eq!(selected_anchors(&s, open), [(0, 2)]);
    assert_eq!(anchor_points(&s, open)[2], original[2]);

    // A closed path goes round: removing its last anchor selects its first.
    let r = rect(&mut s, 0.0, 200.0, 100.0, 80.0);
    let corners = anchor_points(&s, r);
    s.execute("select.anchors", &json!({"id": r.0, "anchors": [[0, 3]]})).unwrap();
    s.execute("path.removeAnchors", &json!({})).unwrap();
    assert_eq!(selected_anchors(&s, r), [(0, 0)]);
    assert_eq!(anchor_points(&s, r)[0], corners[0]);
}

#[test]
fn remove_anchor_points_selects_one_next_anchor_per_subpath_and_per_path() {
    // Several anchors of one subpath: the anchor after the last of them. Each path and each
    // subpath gets its own, at its index once a subpath left too short to keep is gone.
    let mut s = session();
    let made = s.execute("path.create", &json!({"d": "M0 0 L50 0 M0 100 L50 100 L100 100 L150 100 L200 100 L250 100"})).unwrap();
    let id = NodeId(made["id"].as_u64().unwrap());
    let original = anchor_points(&s, id);
    let anchors: Vec<_> = (0..4).map(|i| json!({"x": 100 * i, "y": 400})).collect();
    let other = NodeId(s.execute("path.create", &json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 0], [1, 1], [1, 2]]})).unwrap();
    s.execute("select.anchors", &json!({"id": other.0, "anchors": [[0, 0]], "mode": "add"})).unwrap();
    assert_eq!(s.execute("path.removeAnchors", &json!({})).unwrap()["removedObjects"], 0);
    let sp = &path(&s, id).subpaths;
    assert_eq!(sp.len(), 1, "the first subpath, down to one anchor, is gone");
    assert_eq!(selected_anchors(&s, id), [(0, 1)]);
    assert_eq!(sp[0].anchors[1].p, original[5], "the anchor after the two removed ones");
    assert_eq!(selected_anchors(&s, other), [(0, 0)]);

    // One undo step brings back the anchors and the selection they were removed from.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(anchor_points(&s, id), original);
    assert_eq!(selected_anchors(&s, id), [(0, 0), (1, 1), (1, 2)]);
    assert_eq!(selected_anchors(&s, other), [(0, 0)]);
}

#[test]
fn remove_anchor_point_tool_and_delete_still_select_the_remainder_whole() {
    // Only Remove Anchor Points moves the selection on. The Delete Anchor Point tool leaves the
    // path selected whole, and so does the Delete key, so that a second Delete removes the rest.
    for command in ["path.removeAnchor", "path.deleteAnchors"] {
        let mut s = session();
        let anchors: Vec<_> = (0..4).map(|i| json!({"x": 100 * i, "y": 0})).collect();
        let id = NodeId(s.execute("path.create", &json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap());
        s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 1]]})).unwrap();
        let params = if command == "path.removeAnchor" { json!({"id": id.0, "subpath": 0, "anchor": 1}) } else { json!({}) };
        s.execute(command, &params).unwrap();
        assert!(selected_anchors(&s, id).is_empty(), "{command}");
        assert_eq!(s.doc().unwrap().selection.objects, [id], "{command}");
    }
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
fn direct_segment_delete_keeps_the_other_rectangle_edges_and_one_undo() {
    for initially_selected in [false, true] {
        let mut s = session();
        let id = rect(&mut s, 100.0, 100.0, 200.0, 150.0);
        if !initially_selected {
            s.execute("select.none", &json!({})).unwrap();
        }
        let before = s.doc().unwrap().doc.clone();
        let original = path(&s, id).subpaths[0].clone();
        let undo = undo_steps(&s);
        drag(&mut s, "directSelection", &[(200.0, 100.0)], Mods::default());
        assert_eq!(s.doc().unwrap().doc, before, "a click never edits the document");
        assert_eq!(undo_steps(&s), undo);
        assert_eq!(s.doc().unwrap().selection.segments[&id], [(0, 0)].into());
        s.execute("edit.clear", &json!({})).unwrap();
        let after = s.doc().unwrap().doc.clone();
        let remainder = path(&s, id).subpaths[0].clone();
        assert!(!remainder.closed);
        assert_eq!(remainder.anchors.len(), 4, "the endpoints and neighboring edges survive");
        for i in 0..3 {
            assert_eq!(remainder.segment(i), original.segment(i + 1));
        }
        assert_eq!(undo_steps(&s), undo + 1);
        assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
        assert!(s.doc().unwrap().selection.anchors.is_empty() && s.doc().unwrap().selection.segments.is_empty());
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc, before);
        assert_eq!(s.doc().unwrap().selection.segments[&id], [(0, 0)].into());
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc, after);
        s.execute("edit.clear", &json!({})).unwrap();
        assert!(s.doc().unwrap().doc.node(id).is_none(), "a second Delete removes the rest");
    }
}

/// A segment pick's endpoints are hollow: Shift-clicking or Shift-marqueeing one selects that
/// anchor, rather than toggling it off and leaving the other endpoint selected.
#[test]
fn selecting_an_anchor_after_a_segment_pick_starts_from_no_anchors() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 200.0, 150.0);
    let segment = json!({"id": id.0, "segments": [[0, 0]]});
    s.execute("select.segments", &segment).unwrap();
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 0]], "mode": "toggle"})).unwrap();
    let sel = &s.doc().unwrap().selection;
    assert_eq!(sel.partial(id), Some(&[(0, 0)].into()));
    assert!(sel.segments.is_empty());
    s.execute("select.segments", &segment).unwrap();
    s.execute("select.anchorsMany", &json!({"items": [{"id": id.0, "anchors": [[0, 1]]}], "mode": "add"})).unwrap();
    let sel = &s.doc().unwrap().selection;
    assert_eq!(sel.partial(id), Some(&[(0, 1)].into()));
    assert!(sel.segments.is_empty());
}

#[test]
fn segment_delete_preserves_curves_appearance_and_unrelated_contours() {
    let data = "M100 100C110 70 150 130 170 100C180 75 230 125 250 100C270 140 280 80 310 100C320 120 140 140 100 100Z M400 100L500 100L500 200Z";
    for closed in [false, true] {
        for cuts in [vec![0], vec![1], vec![3], vec![0, 2], vec![0, 3], vec![0, 1, 2, 3]] {
            let mut s = session();
            let id = NodeId(s.execute("path.create", &json!({"d": data})).unwrap()["id"].as_u64().unwrap());
            s.edit("Fixture", |d, _| {
                if let NodeKind::Path { path, .. } = &mut d.node_mut(id).unwrap().kind {
                    path.subpaths[0].closed = closed;
                }
                Ok(())
            })
            .unwrap();
            s.execute("paint.setStroke", &json!({"color":"#123456"})).unwrap();
            let original = path(&s, id);
            let cuts: Vec<_> = cuts.into_iter().filter(|&i| i < original.subpaths[0].segment_count()).collect();
            if cuts.is_empty() {
                continue;
            }
            let refs: Vec<_> = cuts.iter().map(|i| json!([0, i])).collect();
            s.execute("select.segments", &json!({"id":id.0,"segments":refs})).unwrap();
            let before = s.doc().unwrap().doc.clone();
            s.execute("edit.clear", &json!({})).unwrap();
            let result = path(&s, id);
            assert_eq!(result.subpaths.last().unwrap(), original.subpaths.last().unwrap(), "the other contour stays exact");
            let expected: Vec<_> =
                (0..original.subpaths[0].segment_count()).filter(|i| !cuts.contains(i)).map(|i| original.subpaths[0].segment(i)).collect();
            let actual: Vec<_> =
                result.subpaths.iter().take(result.subpaths.len() - 1).flat_map(|sp| (0..sp.segment_count()).map(|i| sp.segment(i))).collect();
            assert_eq!(actual.len(), expected.len());
            assert!(expected.iter().all(|curve| actual.contains(curve)), "{closed} {cuts:?}");
            for sp in result.subpaths.iter().take(result.subpaths.len() - 1) {
                assert!(
                    !sp.anchors.first().unwrap().has_in() && !sp.anchors.last().unwrap().has_out(),
                    "deleted-edge handles do not remain at open endpoints"
                );
            }
            let mut restored = s.doc().unwrap().doc.clone();
            std::sync::Arc::make_mut(&mut restored).node_mut(id).unwrap().kind = before.node(id).unwrap().kind.clone();
            assert_eq!(restored, before, "only the edited path kind changes");
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(s.doc().unwrap().doc, before);
        }
    }
}

#[test]
fn segment_picks_and_deliberate_anchor_picks_delete_different_components() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 200.0, 150.0);
    s.execute("select.segments", &json!({"id":id.0,"segments":[[0,0]]})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let selection = s.doc().unwrap().selection.clone();
    for refs in [json!([[0, 99]]), json!([[99, 0]]), json!([[-1, 0]]), json!([[0, 0.5]])] {
        assert!(s.execute("select.segments", &json!({"id":id.0,"segments":refs})).is_err());
        assert_eq!(s.doc().unwrap().selection, selection);
        assert_eq!(s.doc().unwrap().doc, before);
    }
    s.execute("select.anchors", &json!({"id":id.0,"anchors":[[0,0],[0,1]]})).unwrap();
    assert!(s.doc().unwrap().selection.segments.is_empty());
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!(path(&s, id).anchor_count(), 2, "deliberate anchor Delete keeps its existing behavior");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.segments", &json!({"id":id.0,"segments":[[0,0]]})).unwrap();
    drag(&mut s, "directSelection", &[(100.0, 100.0)], Mods::default());
    assert!(s.doc().unwrap().selection.segments.is_empty());
    assert_eq!(s.doc().unwrap().selection.anchors[&id], [(0, 0)].into(), "clicking an endpoint picks the anchor itself");
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
    assert_eq!(s.cursor(Point::new(250.0, 300.0), none, v), vectorcraft_tools::Cursor::PenDrawing);
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
fn cmd_alt_lends_group_selection_while_direct_selection_is_active() {
    let (cmd, cmd_alt) = (Mods { cmd: true, ..Mods::default() }, Mods { cmd: true, alt: true, ..Mods::default() });
    let v = view();
    let mut s = session();
    // Cmd+Alt while the Pen borrows Direct Selection keeps Direct Selection (Alt is copy/subtract).
    s.select_tool("directSelection", v).unwrap();
    s.select_tool("pen", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 50.0).with_mods(cmd_alt), v).unwrap();
    assert_eq!(s.tool_id(), "directSelection");
    s.pointer(&PointerEvent::new(PointerKind::Up, 50.0, 50.0).with_mods(cmd_alt), v).unwrap();
    assert_eq!(s.tool_id(), "pen");
    // Cmd alone still lends Direct Selection.
    s.pointer(&PointerEvent::new(PointerKind::Down, 60.0, 60.0).with_mods(cmd), v).unwrap();
    assert_eq!(s.tool_id(), "directSelection");
    s.pointer(&PointerEvent::new(PointerKind::Up, 60.0, 60.0).with_mods(cmd), v).unwrap();
    // While Direct Selection is active, Cmd+Alt lends Group Selection for the gesture.
    s.select_tool("directSelection", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 70.0, 70.0).with_mods(cmd_alt), v).unwrap();
    assert_eq!(s.tool_id(), "groupSelection");
    s.pointer(&PointerEvent::new(PointerKind::Up, 70.0, 70.0).with_mods(cmd_alt), v).unwrap();
    assert_eq!(s.tool_id(), "directSelection");
    // Selection last: Cmd+Alt still lends Selection, not Group Selection.
    s.select_tool("selection", v).unwrap();
    s.select_tool("pen", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 80.0, 80.0).with_mods(cmd_alt), v).unwrap();
    assert_eq!(s.tool_id(), "selection");
    s.pointer(&PointerEvent::new(PointerKind::Up, 80.0, 80.0).with_mods(cmd_alt), v).unwrap();
}

#[test]
fn the_pen_drops_the_new_path_asterisk_once_a_path_is_started() {
    // The Pen's asterisk means a click starts a new path: it shows until the first anchor is
    // placed, the pen has no badge while the path is drawn, and the asterisk comes back once the
    // path is finished (Enter, or a click on the first anchor that closes it).
    use vectorcraft_tools::Cursor;
    let (none, v) = (Mods::default(), view());
    let empty = Point::new(500.0, 500.0);
    let mut s = session();
    s.select_tool("pen", v).unwrap();
    assert_eq!(s.cursor(empty, none, v), Cursor::Pen);
    gesture(&mut s, &[(100.0, 300.0)], none);
    assert_eq!(s.cursor(empty, none, v), Cursor::PenDrawing, "after the first anchor");
    gesture(&mut s, &[(200.0, 300.0)], none);
    assert_eq!(s.cursor(empty, none, v), Cursor::PenDrawing);
    s.tool_key(ToolKey::Enter, none, v).unwrap();
    assert_eq!(s.cursor(empty, none, v), Cursor::Pen, "Enter finished the path");
    gesture(&mut s, &[(100.0, 400.0)], none);
    gesture(&mut s, &[(200.0, 400.0)], none);
    gesture(&mut s, &[(150.0, 450.0)], none);
    assert_eq!(s.cursor(Point::new(100.0, 400.0), none, v), Cursor::PenClose);
    gesture(&mut s, &[(100.0, 400.0)], none);
    assert_eq!(s.cursor(empty, none, v), Cursor::Pen, "closing finished the path");
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

/// Both handles of segment `seg` stand at right angles to the line between its anchors, at one
/// length: that length, positive on the right of the line as it runs from the first anchor (y
/// down).
fn perpendicular_handles(sp: &vectorcraft_geom::SubPath, seg: usize) -> f64 {
    let n = sp.anchors.len();
    let (a, b) = (sp.anchors[seg], sp.anchors[(seg + 1) % n]);
    let chord = b.p - a.p;
    let normal = kurbo::Vec2::new(-chord.y, chord.x) / chord.hypot();
    let (ha, hb) = (a.h_out - a.p, b.h_in - b.p);
    assert!(ha.dot(chord).abs() < 1e-6 && hb.dot(chord).abs() < 1e-6, "at right angles: {a:?} {b:?}");
    assert!((ha.dot(normal) - hb.dot(normal)).abs() < 1e-6, "one length: {a:?} {b:?}");
    ha.dot(normal)
}

#[test]
fn shift_reshapes_a_segment_with_perpendicular_handles_of_one_length() {
    // `perpendicular` (Shift): both handles of the segment stand at right angles to the line
    // between its anchors at one length, on the side the point at t goes to, and that point lies
    // as far from the line as the moved point does.
    use kurbo::ParamCurve;
    let mut s = session();
    let reshape = |s: &mut Session, id: NodeId, seg: usize, t: f64, dx: f64, dy: f64| {
        s.execute("path.reshapeSegment", &json!({"id": id.0, "segment": seg, "t": t, "dx": dx, "dy": dy, "perpendicular": true})).unwrap();
    };
    // A straight segment: the point at 0.5 goes 30 pt up and does not follow dx sideways.
    let straight = json!([{"x": 100, "y": 300}, {"x": 200, "y": 300}, {"x": 300, "y": 300}]);
    let id = NodeId(s.execute("path.create", &json!({"anchors": straight})).unwrap()["id"].as_u64().unwrap());
    reshape(&mut s, id, 0, 0.5, 15.0, -30.0);
    let sp = &path(&s, id).subpaths[0];
    assert!((perpendicular_handles(sp, 0) + 40.0).abs() < 1e-6, "{sp:?}");
    assert!(near(sp.segment(0).eval(0.5), Point::new(150.0, 270.0)), "{:?}", sp.segment(0));
    assert!(!sp.anchors[1].has_out() && sp.segment_is_line(1), "the next segment stays straight");
    // A curved segment: its handles turn upright, and its point at 0.25 goes 10 pt further up.
    let curved = json!([{"x": 100, "y": 300, "out": [130, 280]}, {"x": 200, "y": 300, "in": [190, 250]}]);
    let id = NodeId(s.execute("path.create", &json!({"anchors": curved})).unwrap()["id"].as_u64().unwrap());
    let before = path(&s, id).subpaths[0].segment(0).eval(0.25);
    reshape(&mut s, id, 0, 0.25, 0.0, -10.0);
    let sp = &path(&s, id).subpaths[0];
    assert!(perpendicular_handles(sp, 0) < 0.0, "above the line: {sp:?}");
    assert!((sp.segment(0).eval(0.25).y - (before.y - 10.0)).abs() < 1e-6, "{:?}", sp.segment(0));
    // Pulled past the line, the handles go to its other side.
    reshape(&mut s, id, 0, 0.5, 0.0, 80.0);
    assert!(perpendicular_handles(&path(&s, id).subpaths[0], 0) > 0.0, "below the line");
}

#[test]
fn shift_reshapes_the_closing_segment_of_a_rotated_square_and_undoes() {
    // A square turned 45°: the Pen with Alt and Shift held drags its last segment, the one that
    // closes it, outward and to one side; one undo step puts it back.
    let mut s = session();
    let diamond = json!([{"x": 200, "y": 100}, {"x": 300, "y": 200}, {"x": 200, "y": 300}, {"x": 100, "y": 200}]);
    let id = NodeId(s.execute("path.create", &json!({"anchors": diamond, "closed": true})).unwrap()["id"].as_u64().unwrap());
    let original = path(&s, id);
    s.select_tool("pen", view()).unwrap();
    let alt_shift = Mods { alt: true, shift: true, ..Mods::default() };
    assert_eq!(gesture(&mut s, &[(150.0, 150.0), (140.0, 145.0), (130.0, 140.0)], alt_shift), 1);
    let sp = &path(&s, id).subpaths[0];
    let l = perpendicular_handles(sp, 3);
    assert!(l < 0.0 && sp.anchors[3].h_out.x < 100.0 && sp.anchors[0].h_in.y < 100.0, "outward: {sp:?}");
    assert!(!sp.anchors[3].has_in() && !sp.anchors[0].has_out(), "the corners' other handles stay put: {sp:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(path(&s, id), original);
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
    assert_eq!(s.cursor(Point::new(350.0, 200.0), none, v), Cursor::PenDrawing, "the middle of the other path");
    // #1009: Use Precise Cursors makes the join pointer a crosshair too.
    s.execute("prefs.set", &json!({"key": "usePreciseCursors", "value": true})).unwrap();
    assert_eq!(s.cursor(Point::new(300.0, 200.0), none, v), Cursor::Crosshair);
    s.execute("prefs.set", &json!({"key": "usePreciseCursors", "value": false})).unwrap();
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

/// #1533: two corners of a rectangle brought together by Average become one anchor on Join, in
/// one undo step; anchors that aren't on one another leave Join as it was.
#[test]
fn join_merges_the_selected_anchors_average_brought_together() {
    let mut s = session();
    let r = rect(&mut s, 100.0, 100.0, 100.0, 80.0);
    let pts = |s: &Session| path(s, r).subpaths[0].anchors.iter().map(|x| (x.p.x, x.p.y)).collect::<Vec<_>>();
    s.execute("select.anchors", &json!({"id": r.0, "anchors": [[0, 0], [0, 1]], "mode": "set"})).unwrap();
    assert!(s.execute("path.join", &json!({})).is_err(), "apart: nothing to merge, and the path is closed");
    s.execute("path.average", &json!({"axis": "both"})).unwrap();
    s.execute("path.join", &json!({})).unwrap();
    assert_eq!(pts(&s), [(150.0, 100.0), (200.0, 180.0), (100.0, 180.0)]);
    assert!(path(&s, r).subpaths[0].closed);
    let sel = &s.doc().unwrap().selection;
    assert!(sel.objects == [r] && sel.anchors.is_empty());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(pts(&s).len(), 4);
    // The closing pair: the first and last anchors of a closed path.
    s.execute("select.anchors", &json!({"id": r.0, "anchors": [[0, 0], [0, 3]], "mode": "set"})).unwrap();
    s.execute("path.average", &json!({"axis": "both"})).unwrap();
    let before = pts(&s);
    s.execute("path.join", &json!({})).unwrap();
    assert_eq!(pts(&s), before[..3]);
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
        json!({"to": "round"}),
    ] {
        assert!(s.execute("path.join", &bad).is_err(), "{bad}");
    }
}

/// `path.join {to}` makes the joined anchor a corner or smooth point.
#[test]
fn join_to_corner_or_smooth_sets_the_merged_anchor() {
    let mut s = session();
    let r = rect(&mut s, 100.0, 100.0, 100.0, 80.0);
    s.execute("select.anchors", &json!({"id": r.0, "anchors": [[0, 0], [0, 1]], "mode": "set"})).unwrap();
    s.execute("path.average", &json!({"axis": "both"})).unwrap();
    s.execute("path.join", &json!({"to": "corner"})).unwrap();
    let a = &path(&s, r).subpaths[0].anchors[0];
    assert_eq!(a.kind, vectorcraft_geom::AnchorKind::Corner);
    assert_eq!((a.h_in, a.h_out), (a.p, a.p));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("path.average", &json!({"axis": "both"})).unwrap();
    s.execute("path.join", &json!({"to": "smooth"})).unwrap();
    let a = &path(&s, r).subpaths[0].anchors[0];
    assert_eq!(a.kind, vectorcraft_geom::AnchorKind::Smooth);
    let a = line(&mut s, 0.0, 0.0, 100.0, 0.0);
    let b = line(&mut s, 100.0, 0.0, 200.0, 0.0);
    s.execute("path.join", &json!({"ids": [a.0, b.0], "to": "smooth"})).unwrap();
    let j = &path(&s, a).subpaths[0].anchors[1];
    assert_eq!(j.kind, vectorcraft_geom::AnchorKind::Smooth);
}

#[test]
fn pen_with_alt_converts_the_anchors_of_a_path_in_a_selected_group() {
    // #1273: a selected group (or compound path) shows its paths' anchors, so Alt with the Pen
    // converts them as it does a selected path's, rather than starting a new path there.
    let alt = Mods { alt: true, ..Mods::default() };
    let v = view();
    for wrap in ["object.group", "object.compoundPath.make"] {
        let mut s = session();
        let anchors = json!([{"x": 100, "y": 200}, {"x": 200, "y": 100, "in": [150, 100], "out": [250, 100]}, {"x": 300, "y": 200}]);
        let id = NodeId(s.execute("path.create", &json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap());
        s.execute(wrap, &json!({})).unwrap();
        assert!(!s.doc().unwrap().selection.objects.contains(&id), "{wrap}: the wrapper is selected, not the path");
        s.select_tool("pen", v).unwrap();
        // Alt-click the smooth anchor: a corner, in one step, and no new path.
        assert_eq!(gesture(&mut s, &[(200.0, 100.0)], alt), 1, "{wrap}");
        let a = path(&s, id).subpaths[0].anchors[1];
        assert!(!a.has_in() && !a.has_out(), "{wrap}: a corner: {a:?}");
        assert_eq!(paths(&s).len(), 1, "{wrap}: no new path");
        // Alt-drag it: smooth again.
        assert_eq!(gesture(&mut s, &[(200.0, 100.0), (220.0, 100.0), (240.0, 110.0)], alt), 1, "{wrap}");
        let a = path(&s, id).subpaths[0].anchors[1];
        assert_eq!((a.h_in, a.h_out), (Point::new(160.0, 90.0), Point::new(240.0, 110.0)), "{wrap}");
        assert_eq!(paths(&s).len(), 1, "{wrap}: no new path");
    }
}

/// #1679: a filled pencil path keeps a stroke of None; an unfilled one with no stroke gets black,
/// or it would show nothing.
#[test]
fn a_filled_pencil_path_keeps_no_stroke() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let pts = json!([[10, 10], [60, 20], [90, 80], [20, 90]]);
    let id = NodeId(s.execute("path.freehand", &json!({"points": pts, "fill": true})).unwrap()["id"].as_u64().unwrap());
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    assert!(n.appearance.stroke_paint().is_none(), "{:?}", n.appearance);
    assert!(!n.appearance.fill_paint().is_none());
    let id = NodeId(s.execute("path.freehand", &json!({"points": pts})).unwrap()["id"].as_u64().unwrap());
    assert!(!s.doc().unwrap().doc.node(id).unwrap().appearance.stroke_paint().is_none());
}

#[test]
fn open_anchor_delete_splits_runs_without_new_connections() {
    let cases = [
        (vec![3], vec![vec![0, 1, 2], vec![4, 5, 6]]),
        (vec![2, 4], vec![vec![0, 1], vec![5, 6]]),
        (vec![2, 3], vec![vec![0, 1], vec![4, 5, 6]]),
        (vec![0], vec![vec![1, 2, 3, 4, 5, 6]]),
        (vec![6], vec![vec![0, 1, 2, 3, 4, 5]]),
        (vec![0, 6], vec![vec![1, 2, 3, 4, 5]]),
        (vec![1, 3, 5], vec![]),
        (vec![0, 1, 2, 3, 4, 5, 6], vec![]),
    ];
    for (removed, expected) in cases {
        let mut s = session();
        let anchors: Vec<_> = (0..7).map(|i| json!({"x": 100 + 50 * i, "y": 100})).collect();
        let id = NodeId(s.execute("path.create", &json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap());
        let original = path(&s, id).subpaths[0].clone();
        let refs: Vec<_> = removed.iter().map(|i| [0, *i]).collect();
        s.execute("select.anchors", &json!({"id": id.0, "anchors": refs})).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let selection = s.doc().unwrap().selection.clone();
        let undo = undo_steps(&s);
        s.execute("edit.clear", &json!({})).unwrap();
        if expected.is_empty() {
            assert!(s.doc().unwrap().doc.node(id).is_none(), "only isolated points remain: {removed:?}");
            assert!(s.doc().unwrap().selection.objects.is_empty());
        } else {
            let result = path(&s, id);
            assert_eq!(result.subpaths.len(), expected.len(), "deleted anchors: {removed:?}");
            for (sp, indices) in result.subpaths.iter().zip(&expected) {
                assert!(!sp.closed);
                assert_eq!(sp.anchors.iter().map(|a| a.p).collect::<Vec<_>>(), indices.iter().map(|&i| original.anchors[i].p).collect::<Vec<_>>());
                for (segment, pair) in indices.windows(2).enumerate() {
                    assert_eq!(sp.segment(segment), original.segment(pair[0]));
                }
            }
            assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
        }
        assert_eq!(undo_steps(&s), undo + 1);
        assert!(s.doc().unwrap().selection.anchors.is_empty() && s.doc().unwrap().selection.segments.is_empty());
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc, before);
        assert_eq!(s.doc().unwrap().selection, selection);
    }
}

#[test]
fn open_anchor_delete_preserves_curves_appearance_and_history() {
    let mut s = session();
    rect(&mut s, 600.0, 300.0, 30.0, 40.0);
    let data = "M100 100C120 60 180 140 200 100C220 60 280 140 300 100C320 60 380 140 400 100C420 60 480 140 500 100 M100 300L200 300L200 400Z";
    let id = NodeId(s.execute("path.create", &json!({"d": data})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setStroke", &json!({"color": "#123456"})).unwrap();
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 2]]})).unwrap();
    s.doc_mut().unwrap().mark_saved();
    let before = s.doc().unwrap().doc.clone();
    let selection = s.doc().unwrap().selection.clone();
    let original = path(&s, id);
    let undo = undo_steps(&s);
    s.execute("path.deleteAnchors", &json!({})).unwrap();
    let result = path(&s, id);
    assert_eq!(result.subpaths.len(), 3, "two surviving runs plus the untouched contour");
    for (sp, segment) in result.subpaths[..2].iter().zip([0, 3]) {
        assert!(!sp.closed);
        assert_eq!(sp.segment_count(), 1);
        assert_eq!(sp.segment(0), original.subpaths[0].segment(segment));
        assert!(!sp.anchors.first().unwrap().has_in() && !sp.anchors.last().unwrap().has_out());
    }
    assert_eq!(result.subpaths[2], original.subpaths[1]);
    let after = s.doc().unwrap().doc.clone();
    let mut restored = after.clone();
    std::sync::Arc::make_mut(&mut restored).node_mut(id).unwrap().kind = before.node(id).unwrap().kind.clone();
    assert_eq!(restored, before, "only the edited path geometry changes");
    assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
    assert!(s.doc().unwrap().selection.anchors.is_empty() && s.doc().unwrap().selection.segments.is_empty());
    assert_eq!(undo_steps(&s), undo + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Clear");
    assert!(s.doc().unwrap().is_dirty());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert!(!s.doc().unwrap().is_dirty());
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, after);
    assert!(s.doc().unwrap().is_dirty());
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(id).is_none(), "the second Delete removes the remainder");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, after);
}

#[test]
fn closed_anchor_delete_then_join_closes_with_a_line() {
    let mut s = session();
    let data = "M100 100C120 60 180 60 200 100C220 140 280 140 300 100C320 60 380 60 400 100C420 140 480 140 500 100C500 200 100 200 100 100Z";
    let id = NodeId(s.execute("path.create", &json!({"d": data})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 2]]})).unwrap();
    s.execute("path.deleteAnchors", &json!({})).unwrap();
    let result = path(&s, id);
    assert_eq!(result.subpaths.len(), 1);
    let sp = &result.subpaths[0];
    assert!(!sp.closed);
    assert!(!sp.anchors.first().unwrap().has_in() && !sp.anchors.last().unwrap().has_out(), "dangling handles are cleared");
    s.execute("path.join", &json!({"ids": [id.0]})).unwrap();
    let joined = path(&s, id);
    let sp = &joined.subpaths[0];
    assert!(sp.closed);
    let closing = sp.segment(sp.segment_count() - 1);
    assert!(closing.p1 == closing.p0 && closing.p2 == closing.p3, "closing segment is a line: {closing:?}");
}

#[test]
fn open_anchor_delete_does_not_change_explicit_remove_anchors() {
    for command in ["path.removeAnchors", "path.removeAnchor"] {
        let mut s = session();
        let anchors: Vec<_> = (0..5).map(|i| json!({"x": 100 + 50 * i, "y": 100})).collect();
        let id = NodeId(s.execute("path.create", &json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap());
        s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 2]]})).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let original = path(&s, id);
        let undo = undo_steps(&s);
        let params = if command == "path.removeAnchor" { json!({"id": id.0, "subpath": 0, "anchor": 2}) } else { json!({}) };
        s.execute(command, &params).unwrap();
        let result = path(&s, id);
        assert_eq!(result.subpaths.len(), 1, "{command} keeps the path connected");
        let sp = &result.subpaths[0];
        assert!(!sp.closed);
        assert_eq!(sp.segment_count(), 3);
        assert_eq!(sp.anchors.iter().map(|a| a.p).collect::<Vec<_>>(), [0, 1, 3, 4].map(|i| original.subpaths[0].anchors[i].p));
        assert_eq!(undo_steps(&s), undo + 1);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc, before);
    }
}

#[test]
fn open_anchor_delete_after_transform_keeps_mixed_selection_atomic() {
    use vectorcraft_geom::Affine;
    for xf in [
        Affine::IDENTITY,
        Affine::translate((23.0, -17.0)),
        Affine::rotate(0.7),
        Affine::translate((30.0, 40.0)) * Affine::scale_non_uniform(-2.0, 0.5),
    ] {
        let mut s = session();
        let whole = rect(&mut s, 600.0, 100.0, 40.0, 30.0);
        let untouched = rect(&mut s, 600.0, 300.0, 40.0, 30.0);
        let id = NodeId(
            s.execute(
                "path.create",
                &json!({"d": "M100 100C120 60 180 140 200 100C220 60 280 140 300 100C320 60 380 140 400 100C420 60 480 140 500 100"}),
            )
            .unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        // Node::transform bakes coordinates. Deletion must not apply the transform a second time.
        s.edit("Fixture transform", |d, _| {
            d.node_mut(id).unwrap().transform(xf, false);
            Ok(())
        })
        .unwrap();
        let original = path(&s, id);
        s.execute("select.set", &json!({"ids": [whole.0]})).unwrap();
        s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 2]], "mode": "add"})).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let selection = s.doc().unwrap().selection.clone();
        let undo = undo_steps(&s);
        s.execute("edit.clear", &json!({})).unwrap();
        let st = s.doc().unwrap();
        assert!(st.doc.node(whole).is_none());
        assert_eq!(st.doc.node(untouched), before.node(untouched));
        let result = path(&s, id);
        assert_eq!(result.subpaths.len(), 2);
        for (sp, segment) in result.subpaths.iter().zip([0, 3]) {
            assert_eq!(sp.segment_count(), 1);
            assert_eq!(sp.segment(0), original.subpaths[0].segment(segment));
        }
        assert_eq!(st.selection.objects, vec![id]);
        assert!(st.selection.anchors.is_empty() && st.selection.segments.is_empty());
        assert_eq!(undo_steps(&s), undo + 1);
        let after = st.doc.clone();
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc, before);
        assert_eq!(s.doc().unwrap().selection, selection);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc, after);
        assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
    }
}
