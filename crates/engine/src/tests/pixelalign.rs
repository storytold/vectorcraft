//! Transform panel › Align to Pixel Grid (`object.setPixelAligned`): a pixel-aligned path keeps its
//! horizontal and vertical segments on the pixel grid through transforms.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};

use crate::Session;

fn new_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

/// A closed path from SVG path data `d`, selected, with a stroke of `weight` points.
fn path(s: &mut Session, d: &str, weight: f64) -> u64 {
    let id = s.execute("path.create", &json!({"d": d})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("stroke.set", &json!({"weight": weight})).unwrap();
    id
}

/// The anchor points of path object `id`, every subpath's in order.
fn points(s: &Session, id: u64) -> Vec<(f64, f64)> {
    let n = s.doc().unwrap().doc.node(NodeId(id)).unwrap().clone();
    n.path_data().unwrap().anchors().map(|(_, _, a)| (a.p.x, a.p.y)).collect()
}

fn aligned(s: &Session, id: u64) -> bool {
    s.doc().unwrap().doc.node(NodeId(id)).unwrap().extras().pixel_aligned
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// Whether every coordinate is a whole number of points (within rounding error).
fn whole(points: &[(f64, f64)]) -> bool {
    points.iter().all(|(x, y)| (x - x.round()).abs() < 1e-9 && (y - y.round()).abs() < 1e-9)
}

fn close(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
}

/// Turning it on sets the property and snaps the path's horizontal and vertical edges to whole
/// points, in one undo step; turning it on again changes nothing and makes no undo step; undo
/// puts the path and the property back.
#[test]
fn turning_it_on_snaps_the_path_in_one_undo_step() {
    let mut s = new_doc();
    let id = path(&mut s, "M10.3 20.6 L40.7 20.6 L40.7 30.3 L10.3 30.3 Z", 2.0);
    let before = points(&s, id);
    let steps = undo_len(&s);
    let r = s.execute("object.setPixelAligned", &json!({"on": true})).unwrap();
    assert_eq!(r["ids"], json!([id]));
    assert!(aligned(&s, id));
    assert_eq!(points(&s, id), [(10.0, 21.0), (41.0, 21.0), (41.0, 30.0), (10.0, 30.0)]);
    assert_eq!(undo_len(&s), steps + 1);
    let r = s.execute("object.setPixelAligned", &json!({"ids": [id]})).unwrap();
    assert_eq!(r["ids"], json!([]));
    assert_eq!(undo_len(&s), steps + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!aligned(&s, id));
    assert_eq!(points(&s, id), before);
    // An odd stroke weight centered on the path puts the edges on half points, and the stroke
    // weight becomes whole.
    s.execute("stroke.set", &json!({"weight": 0.8})).unwrap();
    s.execute("object.setPixelAligned", &json!({"on": true})).unwrap();
    assert_eq!(points(&s, id), [(10.5, 20.5), (40.5, 20.5), (40.5, 30.5), (10.5, 30.5)]);
    assert_eq!(s.doc().unwrap().doc.node(NodeId(id)).unwrap().appearance.stroke_width(), 1.0);
    // Off keeps the geometry.
    s.execute("object.setPixelAligned", &json!({"on": false})).unwrap();
    assert!(!aligned(&s, id));
    assert_eq!(points(&s, id), [(10.5, 20.5), (40.5, 20.5), (40.5, 30.5), (10.5, 30.5)]);
}

/// A pixel-aligned path stays on the pixel grid after a fractional move, a scale, a rotation about
/// a fractional point, a reflection, Transform Each and a move of its group; one that is not
/// moves exactly as before.
#[test]
fn it_stays_aligned_through_transforms() {
    let mut s = new_doc();
    let id = path(&mut s, "M10 21 L41 21 L41 30 L10 30 Z", 2.0);
    let other = path(&mut s, "M10 121 L41 121 L41 130 L10 130 Z", 2.0);
    s.execute("object.setPixelAligned", &json!({"ids": [id]})).unwrap();
    s.execute("select.set", &json!({"ids": [id, other]})).unwrap();
    s.execute("object.move", &json!({"dx": 0.3, "dy": 0.45})).unwrap();
    assert_eq!(points(&s, id), [(10.0, 21.0), (41.0, 21.0), (41.0, 30.0), (10.0, 30.0)]);
    assert!(close(points(&s, other)[0], (10.3, 121.45)), "{:?}", points(&s, other));
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    for (cmd, p) in [
        ("object.move", json!({"dx": 5.7, "dy": -2.2})),
        ("object.scale", json!({"sx": 133, "sy": 71})),
        ("object.rotate", json!({"angle": 90, "origin": [20.2, 20.7]})),
        ("object.reflect", json!({"axis": "horizontal", "origin": [0.3, 0.3]})),
        ("object.transformEach", json!({"moveH": 0.4, "moveV": 0.35})),
        ("object.setBounds", json!({"width": 17.3})),
    ] {
        s.execute(cmd, &p).unwrap();
        let pts = points(&s, id);
        assert!(whole(&pts), "{cmd}: {pts:?}");
        let xs: Vec<f64> = pts.iter().map(|p| p.0).collect();
        assert!(xs.iter().any(|x| (x - xs[0]).abs() >= 1.0), "{cmd}: the path collapsed: {pts:?}");
    }
    // In a group that moves.
    s.execute("select.set", &json!({"ids": [id, other]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    let before = points(&s, other);
    s.execute("object.move", &json!({"dx": 0.25, "dy": 0.0})).unwrap();
    assert!(whole(&points(&s, id)), "{:?}", points(&s, id));
    assert!(close(points(&s, other)[0], (before[0].0 + 0.25, before[0].1)));
}

/// Art moved with its artboard stays on the pixel grid.
#[test]
fn it_stays_aligned_when_its_artboard_moves() {
    let mut s = new_doc();
    let id = path(&mut s, "M10 21 L41 21 L41 30 L10 30 Z", 2.0);
    s.execute("object.setPixelAligned", &json!({})).unwrap();
    s.execute("artboard.move", &json!({"index": 0, "dx": 0.4, "dy": 0.3, "moveArt": true})).unwrap();
    assert_eq!(points(&s, id), [(10.0, 21.0), (41.0, 21.0), (41.0, 30.0), (10.0, 30.0)]);
    s.execute("artboard.move", &json!({"index": 0, "dx": 0.6, "dy": 0.7, "moveArt": true})).unwrap();
    assert_eq!(points(&s, id), [(11.0, 22.0), (42.0, 22.0), (42.0, 31.0), (11.0, 31.0)]);
}

/// Diagonal segments and curves keep their shape: only the anchors that horizontal and vertical
/// segments share move, each with its handles.
#[test]
fn diagonal_segments_and_curves_keep_their_shape() {
    let mut s = new_doc();
    // A triangle: a horizontal base and two diagonal sides. Only the base's y snaps.
    let tri = path(&mut s, "M0.2 0.2 L20.4 0.2 L10.3 15.7 Z", 2.0);
    s.execute("object.setPixelAligned", &json!({})).unwrap();
    let pts = points(&s, tri);
    assert!(close(pts[0], (0.2, 0.0)) && close(pts[1], (20.4, 0.0)) && close(pts[2], (10.3, 15.7)), "{pts:?}");
    s.execute("object.move", &json!({"dx": 0.5, "dy": 0.3})).unwrap();
    let pts = points(&s, tri);
    assert!(close(pts[0], (0.7, 0.0)) && close(pts[1], (20.9, 0.0)) && close(pts[2], (10.8, 16.0)), "{pts:?}");
    // A curve between two horizontal segments: its ends move with them and its handles with its
    // ends.
    let curved = path(&mut s, "M100.3 100.3 L120.3 100.3 C125 105 125 115 120.3 120.3 L100.3 120.3 Z", 2.0);
    s.execute("object.setPixelAligned", &json!({})).unwrap();
    let n = s.doc().unwrap().doc.node(NodeId(curved)).unwrap().clone();
    let sp = &n.path_data().unwrap().subpaths[0];
    let a = &sp.anchors;
    assert!(close((a[0].p.x, a[0].p.y), (100.0, 100.0)) && close((a[3].p.x, a[3].p.y), (100.0, 120.0)));
    assert!(close((a[1].p.x, a[1].p.y), (120.3, 100.0)) && close((a[1].h_out.x, a[1].h_out.y), (125.0, 104.7)));
    assert!(close((a[2].p.x, a[2].p.y), (120.3, 120.0)) && close((a[2].h_in.x, a[2].h_in.y), (125.0, 114.7)));
    // A rectangle turned 45° has no horizontal or vertical segment, and stays as it is.
    let turned = path(&mut s, "M200.5 200 L210.5 210.3 L200.5 220.6 L190.5 210.3 Z", 2.0);
    let before = points(&s, turned);
    s.execute("object.setPixelAligned", &json!({})).unwrap();
    assert_eq!(points(&s, turned), before);
}

/// Live shapes stay live, and compound paths snap each of their paths.
#[test]
fn live_shapes_and_compound_paths_snap() {
    let mut s = new_doc();
    let rect = s.execute("shape.rectangle", &json!({"x": 10.3, "y": 20.6, "width": 30.4, "height": 9.7})).unwrap()["id"].as_u64().unwrap();
    s.execute("stroke.set", &json!({"weight": 2})).unwrap();
    s.execute("object.setPixelAligned", &json!({})).unwrap();
    let n = s.doc().unwrap().doc.node(NodeId(rect)).unwrap().clone();
    assert!(n.live_shape().is_some());
    let b = n.geometric_bounds().unwrap();
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (10.0, 21.0, 41.0, 30.0));
    s.execute("object.move", &json!({"dx": 0.4, "dy": 0.4})).unwrap();
    let b = s.doc().unwrap().doc.node(NodeId(rect)).unwrap().geometric_bounds().unwrap();
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (10.0, 21.0, 41.0, 30.0));
    let a = path(&mut s, "M100.2 100.2 L110.6 100.2 L110.6 110.6 L100.2 110.6 Z", 2.0);
    let b = path(&mut s, "M200.2 100.2 L210.6 100.2 L210.6 110.6 L200.2 110.6 Z", 2.0);
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("object.setPixelAligned", &json!({"ids": [c]})).unwrap();
    s.execute("object.move", &json!({"dx": 0.3, "dy": 0.0})).unwrap();
    let n = s.doc().unwrap().doc.node(NodeId(c)).unwrap().clone();
    let pts: Vec<(f64, f64)> =
        n.children().unwrap().iter().flat_map(|k| k.path_data().unwrap().anchors().map(|(_, _, a)| (a.p.x, a.p.y)).collect::<Vec<_>>()).collect();
    assert_eq!(pts.len(), 8);
    assert!(whole(&pts), "{pts:?}");
}

/// The anchor points of every path in compound path `id`.
fn compound_points(s: &Session, id: u64) -> Vec<(f64, f64)> {
    let n = s.doc().unwrap().doc.node(NodeId(id)).unwrap().clone();
    n.children().unwrap().iter().flat_map(|k| k.path_data().unwrap().anchors().map(|(_, _, a)| (a.p.x, a.p.y)).collect::<Vec<_>>()).collect()
}

/// A stroke edit on a pixel-aligned path rounds the new weight and snaps the path again: a 0.8 pt
/// weight becomes 1 pt with the edges on half points, a centered 3 pt stroke keeps the edges on
/// half points, and Stroke Weight Up keeps the weight whole; all in the stroke edit's one undo step.
#[test]
fn stroke_edits_keep_it_aligned() {
    let mut s = new_doc();
    let id = path(&mut s, "M10.3 20.6 L40.7 20.6 L40.7 30.3 L10.3 30.3 Z", 2.0);
    s.execute("object.setPixelAligned", &json!({})).unwrap();
    assert_eq!(points(&s, id), [(10.0, 21.0), (41.0, 21.0), (41.0, 30.0), (10.0, 30.0)]);
    let steps = undo_len(&s);
    s.execute("stroke.set", &json!({"weight": 0.8})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node(NodeId(id)).unwrap().appearance.stroke_width(), 1.0);
    assert_eq!(points(&s, id), [(10.5, 21.5), (41.5, 21.5), (41.5, 30.5), (10.5, 30.5)]);
    assert_eq!(undo_len(&s), steps + 1);
    s.execute("stroke.set", &json!({"weight": 2})).unwrap();
    assert!(whole(&points(&s, id)), "{:?}", points(&s, id));
    s.execute("stroke.set", &json!({"weight": 3})).unwrap();
    let pts = points(&s, id);
    assert!(pts.iter().all(|(x, y)| (x - x.floor() - 0.5).abs() < 1e-9 && (y - y.floor() - 0.5).abs() < 1e-9), "{pts:?}");
    // Inside alignment puts the stroke within the path, so the anchors go back to whole points.
    s.execute("stroke.set", &json!({"align": "inside"})).unwrap();
    assert!(whole(&points(&s, id)), "{:?}", points(&s, id));
    s.execute("stroke.set", &json!({"align": "center", "weight": 0.6})).unwrap();
    s.execute("stroke.weightUp", &json!({})).unwrap();
    let w = s.doc().unwrap().doc.node(NodeId(id)).unwrap().appearance.stroke_width();
    assert_eq!(w, w.round(), "{w}");
    // A path that is not pixel-aligned takes the weight as given.
    let plain = path(&mut s, "M110.3 20.6 L140.7 20.6 L140.7 30.3 L110.3 30.3 Z", 0.8);
    assert_eq!(s.doc().unwrap().doc.node(NodeId(plain)).unwrap().appearance.stroke_width(), 0.8);
    assert_eq!(points(&s, plain), [(110.3, 20.6), (140.7, 20.6), (140.7, 30.3), (110.3, 30.3)]);
}

/// Make Compound Path from pixel-aligned paths gives a pixel-aligned compound path whose paths do
/// not keep the property; they snap with the compound path's stroke, so a centered 1 pt stroke puts
/// them on half points after a move. Release gives each path the property back.
#[test]
fn a_compound_path_of_aligned_paths_is_aligned() {
    let mut s = new_doc();
    let a = path(&mut s, "M100.2 100.2 L110.6 100.2 L110.6 110.6 L100.2 110.6 Z", 1.0);
    let b = path(&mut s, "M200.2 100.2 L210.6 100.2 L210.6 110.6 L200.2 110.6 Z", 1.0);
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("object.setPixelAligned", &json!({})).unwrap();
    let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    assert!(aligned(&s, c));
    let n = s.doc().unwrap().doc.node(NodeId(c)).unwrap().clone();
    assert!(n.children().unwrap().iter().all(|k| !k.extras().pixel_aligned));
    let half = |pts: &[(f64, f64)]| pts.iter().all(|(x, y)| (x - x.floor() - 0.5).abs() < 1e-9 && (y - y.floor() - 0.5).abs() < 1e-9);
    assert!(half(&compound_points(&s, c)), "{:?}", compound_points(&s, c));
    s.execute("object.move", &json!({"dx": 0.3, "dy": 0.2})).unwrap();
    let pts = compound_points(&s, c);
    assert_eq!(pts.len(), 8);
    assert!(half(&pts), "{pts:?}");
    // A path inside the compound path cannot take the property alone.
    let member = n.children().unwrap()[0].id.0;
    let e = s.execute("object.setPixelAligned", &json!({"ids": [member]})).unwrap_err().to_string();
    assert!(e.contains("only paths and compound paths"), "{e}");
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    s.execute("object.compoundPath.release", &json!({})).unwrap();
    let released = s.doc().unwrap().selection.objects.clone();
    assert_eq!(released.len(), 2);
    assert!(released.iter().all(|id| aligned(&s, id.0)));
}

/// Make Compound Path from a pixel-aligned compound path and another path gives a pixel-aligned
/// compound path, which keeps snapping after a move.
#[test]
fn a_compound_path_of_an_aligned_compound_path_is_aligned() {
    let mut s = new_doc();
    let a = path(&mut s, "M100.2 100.2 L110.6 100.2 L110.6 110.6 L100.2 110.6 Z", 2.0);
    let b = path(&mut s, "M200.2 100.2 L210.6 100.2 L210.6 110.6 L200.2 110.6 Z", 2.0);
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("object.setPixelAligned", &json!({"ids": [c]})).unwrap();
    let other = path(&mut s, "M300.2 100.2 L310.6 100.2 L310.6 110.6 L300.2 110.6 Z", 2.0);
    s.execute("select.set", &json!({"ids": [c, other]})).unwrap();
    let joined = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    assert!(aligned(&s, joined));
    s.execute("object.move", &json!({"dx": 0.3, "dy": 0.4})).unwrap();
    let pts = compound_points(&s, joined);
    assert_eq!(pts.len(), 12);
    assert!(whole(&pts), "{pts:?}");
}

/// A stroke with no paint does not count: a fill-only path whose stroke item keeps an odd weight
/// snaps to whole points.
#[test]
fn a_stroke_without_paint_snaps_to_whole_points() {
    let mut s = new_doc();
    let id = path(&mut s, "M10.3 20.6 L40.7 20.6 L40.7 30.3 L10.3 30.3 Z", 1.0);
    // An imported file can hold a stroke item without paint that keeps its weight.
    let n = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(NodeId(id)).unwrap();
    let Some(vectorcraft_doc::AppearanceItem::Stroke(st)) =
        n.appearance.items.iter_mut().find(|i| matches!(i, vectorcraft_doc::AppearanceItem::Stroke(_)))
    else {
        panic!("no stroke");
    };
    st.paint = vectorcraft_color::Paint::None;
    assert_eq!(st.width, 1.0);
    s.execute("object.setPixelAligned", &json!({})).unwrap();
    assert_eq!(points(&s, id), [(10.0, 21.0), (41.0, 21.0), (41.0, 30.0), (10.0, 30.0)]);
}

/// With direct-selected anchors in the selection, a move snaps the pixel-aligned path whose anchors
/// moved and the pixel-aligned object selected whole.
#[test]
fn moving_direct_selected_anchors_keeps_it_aligned() {
    let mut s = new_doc();
    let a = path(&mut s, "M10.3 20.6 L40.7 20.6 L40.7 30.3 L10.3 30.3 Z", 2.0);
    let b = path(&mut s, "M110.3 20.6 L140.7 20.6 L140.7 30.3 L110.3 30.3 Z", 2.0);
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("object.setPixelAligned", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    s.execute("select.anchors", &json!({"id": a, "anchors": [[0, 1], [0, 2]], "mode": "add"})).unwrap();
    s.execute("path.moveAnchors", &json!({"dx": 0.4, "dy": 0.0})).unwrap();
    assert_eq!(points(&s, a), [(10.0, 21.0), (41.0, 21.0), (41.0, 30.0), (10.0, 30.0)]);
    assert_eq!(points(&s, b), [(110.0, 21.0), (141.0, 21.0), (141.0, 30.0), (110.0, 30.0)]);
    s.execute("path.moveAnchors", &json!({"dx": 0.7, "dy": 0.0})).unwrap();
    assert_eq!(points(&s, a), [(10.0, 21.0), (42.0, 21.0), (42.0, 30.0), (10.0, 30.0)]);
    assert_eq!(points(&s, b), [(111.0, 21.0), (142.0, 21.0), (142.0, 30.0), (111.0, 30.0)]);
}

/// A path with Live Corners snaps the outline its corners are cut from, inner edges included, and
/// keeps its Live Corners.
#[test]
fn a_path_with_live_corners_snaps_its_inner_edges() {
    let mut s = new_doc();
    let id = path(&mut s, "M10 10 L40 10 L40 20.3 L25 20.3 L25 40 L10 40 Z", 2.0);
    s.execute("object.setLiveShape", &json!({"id": id, "radius": 2})).unwrap();
    let base = |s: &Session| {
        let n = s.doc().unwrap().doc.node(NodeId(id)).unwrap().clone();
        let NodeKind::Path { live: Some(live), .. } = &n.kind else { panic!("no Live Corners") };
        let vectorcraft_doc::LiveShape::Path { base, .. } = &**live else { panic!("not a path with Live Corners") };
        base.anchors().map(|(_, _, a)| (a.p.x, a.p.y)).collect::<Vec<_>>()
    };
    assert!(base(&s).iter().any(|p| close(*p, (40.0, 20.3))));
    s.execute("object.setPixelAligned", &json!({"ids": [id]})).unwrap();
    assert_eq!(base(&s), [(10.0, 10.0), (40.0, 10.0), (40.0, 20.0), (25.0, 20.0), (25.0, 40.0), (10.0, 40.0)]);
    let pts = points(&s, id);
    assert!(pts.iter().all(|p| (p.1 - 20.3).abs() > 1e-6), "{pts:?}");
}

/// `path.setAnchors` and `object.setLiveShape` snap a pixel-aligned path again after they replace
/// its geometry.
#[test]
fn geometry_edits_keep_it_aligned() {
    let mut s = new_doc();
    let id = path(&mut s, "M10 21 L41 21 L41 30 L10 30 Z", 2.0);
    s.execute("object.setPixelAligned", &json!({"ids": [id]})).unwrap();
    let anchors = json!([{"x": 10.3, "y": 21.4}, {"x": 41.3, "y": 21.4}, {"x": 41.3, "y": 30.4}, {"x": 10.3, "y": 30.4}]);
    s.execute("path.setAnchors", &json!({"id": id, "subpaths": [{"anchors": anchors, "closed": true}]})).unwrap();
    assert_eq!(points(&s, id), [(10.0, 21.0), (41.0, 21.0), (41.0, 30.0), (10.0, 30.0)]);
    let poly = s.execute("shape.polygon", &json!({"cx": 100, "cy": 100, "radius": 50, "sides": 6})).unwrap()["id"].as_u64().unwrap();
    s.execute("object.setPixelAligned", &json!({"ids": [poly]})).unwrap();
    s.execute("object.setLiveShape", &json!({"id": poly, "sides": 5})).unwrap();
    let n = s.doc().unwrap().doc.node(NodeId(poly)).unwrap().clone();
    let b = n.geometric_bounds().unwrap();
    let on_grid = |v: f64| ((v - 0.5) - (v - 0.5).round()).abs() < 1e-9 || (v - v.round()).abs() < 1e-9;
    assert!(on_grid(b.y1), "{b:?}");
}

/// Any other edit of a pixel-aligned path, here Object › Path › Average, snaps it again too.
#[test]
fn other_path_edits_keep_it_aligned() {
    let mut s = new_doc();
    let a = path(&mut s, "M10 21 L41 21 L41 30 L10 30 Z", 2.0);
    let b = path(&mut s, "M60.3 50 L80 50 L80 60 L60.3 60 Z", 2.0);
    s.execute("object.setPixelAligned", &json!({"ids": [a]})).unwrap();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    s.execute("select.anchors", &json!({"id": a, "anchors": [[0, 1], [0, 2]], "mode": "add"})).unwrap();
    s.execute("select.anchors", &json!({"id": b, "anchors": [[0, 0]], "mode": "add"})).unwrap();
    s.execute("path.average", &json!({"axis": "vertical"})).unwrap();
    assert_eq!(points(&s, a), [(10.0, 21.0), (47.0, 21.0), (47.0, 30.0), (10.0, 30.0)]);
    assert!((points(&s, b)[0].0 - (41.0 + 41.0 + 60.3) / 3.0).abs() < 1e-9, "{:?}", points(&s, b));
}

/// A stroke edit that Global Edit carries to the other members snaps the pixel-aligned ones.
#[test]
fn global_edit_keeps_its_members_aligned() {
    let mut s = new_doc();
    let a = path(&mut s, "M10 21 L41 21 L41 30 L10 30 Z", 2.0);
    // The same shape at twice the size.
    let b = path(&mut s, "M110 21 L172 21 L172 39 L110 39 Z", 2.0);
    s.execute("object.setPixelAligned", &json!({"ids": [a, b]})).unwrap();
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let r = s.execute("select.globalEdit", &json!({})).unwrap();
    assert_eq!(r["active"], true);
    s.execute("stroke.set", &json!({"weight": 1})).unwrap();
    assert_eq!(strokes(&s, b)[0].1, 1.0);
    assert_eq!(points(&s, a), [(10.5, 21.5), (41.5, 21.5), (41.5, 30.5), (10.5, 30.5)]);
    assert!(halves(&points(&s, b)), "{:?}", points(&s, b));
}

/// Align with direct-selected anchors snaps the pixel-aligned path whose anchors moved.
#[test]
fn aligning_direct_selected_anchors_keeps_it_aligned() {
    let mut s = new_doc();
    let a = path(&mut s, "M10 21 L41 21 L41 30 L10 30 Z", 2.0);
    let b = path(&mut s, "M60.4 50.3 L80 50.3 L80 60 L60.4 60 Z", 2.0);
    s.execute("object.setPixelAligned", &json!({"ids": [a]})).unwrap();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    s.execute("select.anchors", &json!({"id": a, "anchors": [[0, 1], [0, 2]], "mode": "add"})).unwrap();
    s.execute("select.anchors", &json!({"id": b, "anchors": [[0, 0]], "mode": "add"})).unwrap();
    s.execute("object.align", &json!({"horizontal": "right"})).unwrap();
    assert_eq!(points(&s, a), [(10.0, 21.0), (60.0, 21.0), (60.0, 30.0), (10.0, 30.0)]);
    assert!(close(points(&s, b)[0], (60.4, 50.3)), "{:?}", points(&s, b));
}

/// The art of a linked opacity mask moves and scales with a path that snaps, and a placed gradient
/// keeps its place too.
#[test]
fn a_linked_opacity_mask_follows_a_snap() {
    let mut s = new_doc();
    let o = path(&mut s, "M10.3 20.6 L40.7 20.6 L40.7 30.3 L10.3 30.3 Z", 2.0);
    let m = path(&mut s, "M12 23 L39 23 L39 28 L12 28 Z", 2.0);
    s.execute("transparency.makeOpacityMask", &json!({"ids": [o, m]})).unwrap();
    let mask_bounds = |s: &Session| {
        let n = s.doc().unwrap().doc.node(NodeId(o)).unwrap().clone();
        n.extras().mask.clone().expect("an opacity mask").art.geometric_bounds().unwrap()
    };
    let (from, mask) = (s.doc().unwrap().doc.node(NodeId(o)).unwrap().geometric_bounds().unwrap(), mask_bounds(&s));
    s.execute("object.setPixelAligned", &json!({"ids": [o]})).unwrap();
    let to = s.doc().unwrap().doc.node(NodeId(o)).unwrap().geometric_bounds().unwrap();
    assert_eq!(points(&s, o), [(10.0, 21.0), (41.0, 21.0), (41.0, 30.0), (10.0, 30.0)]);
    let at = |r: vectorcraft_geom::Rect, x: f64, y: f64| ((x - r.x0) / r.width(), (y - r.y0) / r.height());
    let after = mask_bounds(&s);
    assert!(close(at(to, after.x0, after.y0), at(from, mask.x0, mask.y0)), "{after:?} from {mask:?}");
    assert!(close(at(to, after.x1, after.y1), at(from, mask.x1, mask.y1)), "{after:?} from {mask:?}");
}

/// `path.moveAnchor` (the Anchor Point tool, Smart Pen) snaps a pixel-aligned path again: two moves
/// that leave a horizontal segment off the grid put it back on whole points.
#[test]
fn moving_one_anchor_keeps_it_aligned() {
    let mut s = new_doc();
    let id = path(&mut s, "M10 21 L41 21 L41 30 L10 30 Z", 2.0);
    s.execute("object.setPixelAligned", &json!({})).unwrap();
    s.execute("path.moveAnchor", &json!({"id": id, "anchor": 0, "x": 10, "y": 21.3})).unwrap();
    s.execute("path.moveAnchor", &json!({"id": id, "anchor": 1, "x": 41, "y": 21.3})).unwrap();
    assert_eq!(points(&s, id), [(10.0, 21.0), (41.0, 21.0), (41.0, 30.0), (10.0, 30.0)]);
}

/// A path of a pixel-aligned compound path moved on its own (Group Selection) snaps with the
/// compound path.
#[test]
fn moving_a_path_of_an_aligned_compound_path_keeps_it_aligned() {
    let mut s = new_doc();
    let a = path(&mut s, "M100 100 L110 100 L110 110 L100 110 Z", 2.0);
    let b = path(&mut s, "M200 100 L210 100 L210 110 L200 110 Z", 2.0);
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("object.setPixelAligned", &json!({"ids": [c]})).unwrap();
    let n = s.doc().unwrap().doc.node(NodeId(c)).unwrap().clone();
    let member = n.children().unwrap()[0].id.0;
    s.execute("select.set", &json!({"ids": [member]})).unwrap();
    s.execute("object.move", &json!({"dx": 0.3, "dy": 0.4})).unwrap();
    let pts = compound_points(&s, c);
    assert_eq!(pts.len(), 8);
    assert!(whole(&pts), "{pts:?}");
}

/// A path that Paste in Front puts inside a pixel-aligned compound path, or that an object slice
/// move moves there, snaps with the compound path's stroke, and a pasted path drops its own
/// property there.
#[test]
fn paths_added_to_or_moved_in_an_aligned_compound_path_keep_it_aligned() {
    let mut s = new_doc();
    let a = path(&mut s, "M100 100 L110 100 L110 110 L100 110 Z", 1.0);
    let b = path(&mut s, "M200 100 L210 100 L210 110 L200 110 Z", 1.0);
    let loose = path(&mut s, "M300 100 L310 100 L310 110 L300 110 Z", 2.0);
    s.execute("object.setPixelAligned", &json!({"ids": [loose]})).unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("object.setPixelAligned", &json!({"ids": [c]})).unwrap();
    assert!(halves(&compound_points(&s, c)));
    let member = s.doc().unwrap().doc.node(NodeId(c)).unwrap().children().unwrap()[0].id.0;
    s.execute("select.set", &json!({"ids": [loose]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [member]})).unwrap();
    s.execute("edit.pasteInFront", &json!({})).unwrap();
    let pasted = s.doc().unwrap().selection.objects[0];
    assert_eq!(s.doc().unwrap().doc.parent_of(pasted), Some(NodeId(c)));
    assert!(!aligned(&s, pasted.0));
    let pts = compound_points(&s, c);
    assert_eq!(pts.len(), 12);
    assert!(halves(&pts), "paste: {pts:?}");
    s.execute("object.slice.make", &json!({"ids": [member]})).unwrap();
    s.execute("object.slice.move", &json!({"slices": [member], "dx": 0.3, "dy": 0.4})).unwrap();
    let pts = compound_points(&s, c);
    assert!(halves(&pts), "slice: {pts:?}");
}

/// Art moved by Artboard Options with Move Artwork with Artboard, and the art copies of an
/// artboard copied with its art, stay on the pixel grid.
#[test]
fn artboard_options_and_artboard_copies_keep_it_aligned() {
    let mut s = new_doc();
    let id = path(&mut s, "M10 21 L41 21 L41 30 L10 30 Z", 2.0);
    s.execute("object.setPixelAligned", &json!({})).unwrap();
    let r = s.doc().unwrap().doc.artboards[0].rect;
    s.execute("artboard.setProps", &json!({"index": 0, "x": r.x0 + 0.4, "y": r.y0 + 0.3, "moveArt": true})).unwrap();
    assert_eq!(points(&s, id), [(10.0, 21.0), (41.0, 21.0), (41.0, 30.0), (10.0, 30.0)]);
    let out = s.execute("artboard.move", &json!({"index": 0, "dx": 500.4, "dy": 0.3, "copy": true, "moveArt": true})).unwrap();
    let copies = out["moved"].as_array().unwrap();
    assert_eq!(copies.len(), 1);
    let copy = copies[0].as_u64().unwrap();
    assert!(aligned(&s, copy));
    assert!(whole(&points(&s, copy)), "{:?}", points(&s, copy));
    assert_eq!(points(&s, id), [(10.0, 21.0), (41.0, 21.0), (41.0, 30.0), (10.0, 30.0)]);
}

/// Type, images and groups cannot be aligned: the command is an error and nothing changes, also
/// for a path selected with one of them.
#[test]
fn type_and_images_cannot_be_aligned() {
    let mut s = new_doc();
    let p = path(&mut s, "M10.3 20.6 L40.7 20.6 L40.7 30.3 L10.3 30.3 Z", 2.0);
    let before = points(&s, p);
    let text = s.execute("text.create", &json!({"x": 10, "y": 100, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    let png = crate::tests_links::png(4, 4, [255, 0, 0]);
    s.execute("file.place", &json!({"name": "red.png", "dataBase64": vectorcraft_format::base64_encode(&png), "link": false})).unwrap();
    let image = {
        let st = s.doc().unwrap();
        let id = st.selection.objects[0];
        assert!(matches!(st.doc.node(id).unwrap().kind, NodeKind::Image(_)));
        id.0
    };
    let steps = undo_len(&s);
    for ids in [json!([text]), json!([image]), json!([p, text])] {
        let e = s.execute("object.setPixelAligned", &json!({"ids": ids})).unwrap_err().to_string();
        assert!(e.contains("only paths and compound paths"), "{ids}: {e}");
        assert_eq!(undo_len(&s), steps, "{ids}");
        assert!(!aligned(&s, p));
        assert_eq!(points(&s, p), before);
    }
    let e = s.execute("object.setPixelAligned", &json!({"ids": [p], "on": "yes"})).unwrap_err().to_string();
    assert!(e.contains("`on` must be true or false"), "{e}");
    assert_eq!(undo_len(&s), steps);
}

/// The property is saved in the native file and read back; an object without it is saved without
/// the key and reads back off.
#[test]
fn the_property_is_saved_and_read_back() {
    let mut s = new_doc();
    let a = path(&mut s, "M10.3 20.6 L40.7 20.6 L40.7 30.3 L10.3 30.3 Z", 2.0);
    let b = path(&mut s, "M110.3 20.6 L140.7 20.6 L140.7 30.3 L110.3 30.3 Z", 2.0);
    s.execute("object.setPixelAligned", &json!({"ids": [a]})).unwrap();
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(d, false)).unwrap();
    assert!(back.node(NodeId(a)).unwrap().extras().pixel_aligned);
    assert!(!back.node(NodeId(b)).unwrap().extras().pixel_aligned);
    let plain: Value = serde_json::to_value(d.node(NodeId(b)).unwrap()).unwrap();
    assert!(plain.get("pixel_aligned").is_none(), "{plain}");
    let node = s.execute("document.node", &json!({"id": a})).unwrap();
    assert_eq!(node["pixel_aligned"], true, "{node}");
}

/// A copy of a pixel-aligned path that Duplicate or Paste moves by a fractional offset is snapped
/// to the pixel grid and keeps the property, and so is a pixel-aligned object slice that moves.
#[test]
fn duplicates_and_pastes_keep_it_aligned() {
    let mut s = new_doc();
    let id = path(&mut s, "M10.3 20.6 L40.7 20.6 L40.7 30.3 L10.3 30.3 Z", 2.0);
    s.execute("object.setPixelAligned", &json!({"ids": [id]})).unwrap();
    s.execute("edit.duplicate", &json!({"dx": 100.3, "dy": 0.4})).unwrap();
    let copy = s.doc().unwrap().selection.objects[0].0;
    assert_ne!(copy, id);
    assert!(aligned(&s, copy));
    assert_eq!(points(&s, copy), [(110.0, 21.0), (141.0, 21.0), (141.0, 30.0), (110.0, 30.0)]);
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("edit.paste", &json!({"dx": 0.3, "dy": 50.4})).unwrap();
    let pasted = s.doc().unwrap().selection.objects[0].0;
    assert!(pasted != id && pasted != copy);
    assert!(aligned(&s, pasted));
    assert_eq!(points(&s, pasted), [(10.0, 71.0), (41.0, 71.0), (41.0, 80.0), (10.0, 80.0)]);
    // Moving an object slice moves its object, which snaps too.
    s.execute("object.slice.make", &json!({"ids": [pasted]})).unwrap();
    s.execute("object.slice.move", &json!({"slices": [pasted], "dx": 0.3, "dy": 0.4})).unwrap();
    assert_eq!(points(&s, pasted), [(10.0, 71.0), (41.0, 71.0), (41.0, 80.0), (10.0, 80.0)]);
}

/// Pixel-aligned art of a linked opacity mask snaps when its object moves.
#[test]
fn the_art_of_a_linked_opacity_mask_keeps_it_aligned() {
    let mut s = new_doc();
    let o = path(&mut s, "M10 21 L41 21 L41 30 L10 30 Z", 2.0);
    let m = path(&mut s, "M12 23 L39 23 L39 28 L12 28 Z", 2.0);
    s.execute("object.setPixelAligned", &json!({"ids": [m]})).unwrap();
    s.execute("transparency.makeOpacityMask", &json!({"ids": [o, m]})).unwrap();
    let mask_points = |s: &Session| {
        let n = s.doc().unwrap().doc.node(NodeId(o)).unwrap().clone();
        let mask = n.extras().mask.clone().expect("an opacity mask");
        assert!(mask.art.extras().pixel_aligned);
        mask.art.path_data().unwrap().anchors().map(|(_, _, a)| (a.p.x, a.p.y)).collect::<Vec<_>>()
    };
    s.execute("select.set", &json!({"ids": [o]})).unwrap();
    s.execute("object.move", &json!({"dx": 0.3, "dy": 0.4})).unwrap();
    assert_eq!(mask_points(&s), [(12.0, 23.0), (39.0, 23.0), (39.0, 28.0), (12.0, 28.0)]);
    // The masked path inside a pixel-aligned compound path.
    let other = path(&mut s, "M110 21 L141 21 L141 30 L110 30 Z", 2.0);
    s.execute("select.set", &json!({"ids": [o, other]})).unwrap();
    let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("object.setPixelAligned", &json!({"ids": [c]})).unwrap();
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    s.execute("object.move", &json!({"dx": 0.3, "dy": 0.4})).unwrap();
    assert!(whole(&mask_points(&s)), "{:?}", mask_points(&s));
}

/// The start point of path object `id`'s placed gradient fill.
fn gradient_start(s: &Session, id: u64) -> (f64, f64) {
    let n = s.doc().unwrap().doc.node(NodeId(id)).unwrap().clone();
    let Some(vectorcraft_color::Paint::Gradient(g)) = n.appearance.paint_at(None, true) else { panic!("no gradient fill") };
    let p = g.geom.expect("a placed gradient").start;
    (p.x, p.y)
}

/// A placed gradient moves with a pixel-aligned path when a stroke edit snaps it.
#[test]
fn placed_gradients_follow_the_snapped_path() {
    let mut s = new_doc();
    let id = path(&mut s, "M10 21 L41 21 L41 30 L10 30 Z", 2.0);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}, "ids": [id]})).unwrap();
    {
        let n = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(NodeId(id)).unwrap();
        let bounds = n.geometric_bounds().unwrap();
        let Some(vectorcraft_doc::AppearanceItem::Fill(f)) = n.appearance.items.iter_mut().find(|i| i.is_fill()) else { panic!("no fill") };
        let vectorcraft_color::Paint::Gradient(g) = &mut f.paint else { panic!("no gradient fill") };
        g.pin(bounds);
    }
    s.execute("object.setPixelAligned", &json!({"ids": [id]})).unwrap();
    let before = gradient_start(&s, id);
    let i = strokes(&s, id)[0].0;
    s.execute("appearance.setItem", &json!({"ids": [id], "index": i, "weight": 1})).unwrap();
    assert_eq!(points(&s, id), [(10.5, 21.5), (41.5, 21.5), (41.5, 30.5), (10.5, 30.5)]);
    assert!(close(gradient_start(&s, id), (before.0 + 0.5, before.1 + 0.5)), "{:?} from {before:?}", gradient_start(&s, id));
}

/// The paint-order indices of path object `id`'s strokes, and their weights.
fn strokes(s: &Session, id: u64) -> Vec<(usize, f64)> {
    let n = s.doc().unwrap().doc.node(NodeId(id)).unwrap().clone();
    n.appearance
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| match it {
            vectorcraft_doc::AppearanceItem::Stroke(st) => Some((i, st.width)),
            _ => None,
        })
        .collect()
}

/// Whether every coordinate is a whole number of points plus a half (within rounding error).
fn halves(points: &[(f64, f64)]) -> bool {
    points.iter().all(|(x, y)| ((x - 0.5) - (x - 0.5).round()).abs() < 1e-9 && ((y - 0.5) - (y - 0.5).round()).abs() < 1e-9)
}

/// Appearance edits that add, remove, hide, repaint or reweigh a stroke snap a pixel-aligned path
/// again: Appearance panel item edits, Remove Item, Add New Stroke, Clear Appearance, Reduce to
/// Basic Appearance, Show All Hidden Attributes, moving or copying an appearance,
/// `paint.setStroke`, a graphic style and the Eyedropper. The edges go to half
/// points while a visible, painted centered stroke has an odd weight, otherwise to whole points.
#[test]
fn appearance_edits_keep_it_aligned() {
    let mut s = new_doc();
    let b = path(&mut s, "M110.3 20.6 L140.7 20.6 L140.7 30.3 L110.3 30.3 Z", 2.2);
    let c = path(&mut s, "M210.3 20.6 L240.7 20.6 L240.7 30.3 L210.3 30.3 Z", 1.0);
    s.execute("graphicStyle.new", &json!({"id": c, "name": "One"})).unwrap();
    let a = path(&mut s, "M10.3 20.6 L40.7 20.6 L40.7 30.3 L10.3 30.3 Z", 2.0);
    s.execute("object.setPixelAligned", &json!({"ids": [a]})).unwrap();
    assert!(whole(&points(&s, a)));
    let top = |s: &Session| strokes(s, a).last().map(|(i, _)| *i).unwrap();
    let i = top(&s);
    s.execute("appearance.setItem", &json!({"ids": [a], "index": i, "weight": 3.3})).unwrap();
    assert_eq!(strokes(&s, a).last().unwrap().1, 3.0);
    assert!(halves(&points(&s, a)), "weight: {:?}", points(&s, a));
    // Hiding and showing the stroke again puts the path back where it was.
    let shown = points(&s, a);
    for _ in 0..2 {
        s.execute("appearance.setItem", &json!({"ids": [a], "index": i, "visible": false})).unwrap();
        assert!(whole(&points(&s, a)), "hidden: {:?}", points(&s, a));
        s.execute("appearance.setItem", &json!({"ids": [a], "index": i, "visible": true})).unwrap();
        assert_eq!(points(&s, a), shown);
    }
    s.execute("paint.setStroke", &json!({"ids": [a], "none": true})).unwrap();
    assert!(whole(&points(&s, a)), "no paint: {:?}", points(&s, a));
    s.execute("paint.setStroke", &json!({"ids": [a], "color": "#00ff00"})).unwrap();
    assert_eq!(strokes(&s, a).last().unwrap().1, 1.0);
    assert!(halves(&points(&s, a)), "painted: {:?}", points(&s, a));
    let i = top(&s);
    s.execute("appearance.removeItem", &json!({"ids": [a], "index": i})).unwrap();
    assert!(strokes(&s, a).is_empty());
    assert!(whole(&points(&s, a)), "removed: {:?}", points(&s, a));
    s.execute("appearance.addStroke", &json!({"ids": [a]})).unwrap();
    assert!(halves(&points(&s, a)), "added: {:?}", points(&s, a));
    s.execute("appearance.clear", &json!({"ids": [a]})).unwrap();
    assert!(whole(&points(&s, a)), "cleared: {:?}", points(&s, a));
    s.execute("graphicStyle.apply", &json!({"ids": [a], "name": "One"})).unwrap();
    assert!(halves(&points(&s, a)), "style: {:?}", points(&s, a));
    s.execute("appearance.copyFrom", &json!({"source": b, "ids": [a]})).unwrap();
    assert_eq!(strokes(&s, a).last().unwrap().1, 2.0);
    assert!(whole(&points(&s, a)), "eyedropper: {:?}", points(&s, a));
    // A 1 pt stroke below a 2 pt one: half points, and whole points once Reduce to Basic Appearance
    // keeps only the top stroke.
    let i = top(&s);
    s.execute("appearance.setItem", &json!({"ids": [a], "index": i, "weight": 1})).unwrap();
    s.execute("appearance.addStroke", &json!({"ids": [a]})).unwrap();
    let i = top(&s);
    s.execute("appearance.setItem", &json!({"ids": [a], "index": i, "weight": 2})).unwrap();
    assert_eq!(strokes(&s, a).iter().map(|(_, w)| *w).collect::<Vec<_>>(), [1.0, 2.0]);
    assert!(halves(&points(&s, a)), "two strokes: {:?}", points(&s, a));
    s.execute("appearance.reduceToBasic", &json!({"ids": [a]})).unwrap();
    assert_eq!(strokes(&s, a).iter().map(|(_, w)| *w).collect::<Vec<_>>(), [2.0]);
    assert!(whole(&points(&s, a)), "reduced: {:?}", points(&s, a));
    // Show All Hidden Attributes, and moving or copying an appearance onto or off the path.
    let i = top(&s);
    s.execute("appearance.setItem", &json!({"ids": [a], "index": i, "weight": 1})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [a], "index": i, "visible": false})).unwrap();
    assert!(whole(&points(&s, a)), "hidden: {:?}", points(&s, a));
    s.execute("appearance.showAllHidden", &json!({"ids": [a]})).unwrap();
    assert!(halves(&points(&s, a)), "show all: {:?}", points(&s, a));
    s.execute("appearance.transfer", &json!({"source": b, "target": a, "copy": true})).unwrap();
    assert!(whole(&points(&s, a)), "copied onto: {:?}", points(&s, a));
    let i = top(&s);
    s.execute("appearance.setItem", &json!({"ids": [a], "index": i, "weight": 1})).unwrap();
    assert!(halves(&points(&s, a)), "1 pt: {:?}", points(&s, a));
    s.execute("appearance.transfer", &json!({"source": a, "target": c})).unwrap();
    assert!(whole(&points(&s, a)), "moved off: {:?}", points(&s, a));
    // A path that is not aligned keeps its coordinates through the same edits.
    let before = points(&s, b);
    s.execute("appearance.setItem", &json!({"ids": [b], "index": strokes(&s, b)[0].0, "weight": 3.3})).unwrap();
    s.execute("appearance.addStroke", &json!({"ids": [b]})).unwrap();
    assert_eq!(points(&s, b), before);
}

#[test]
fn patterns_follow_the_snap_with_transform_patterns() {
    let mut s = new_doc();
    let tile = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setFill", &json!({"ids": [tile], "color": "#ff0000"})).unwrap();
    s.execute("select.set", &json!({"ids": [tile]})).unwrap();
    s.execute("object.pattern.make", &json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    let a = path(&mut s, "M 100 100 L 150 100 L 150 150 L 100 150 Z", 2.0);
    s.execute("paint.setFill", &json!({"ids": [a], "swatch": "Dots"})).unwrap();
    s.execute("object.setPixelAligned", &json!({"ids": [a]})).unwrap();
    let tiles = |s: &Session| {
        let n = s.doc().unwrap().doc.node(NodeId(a)).unwrap().clone();
        let vectorcraft_color::Paint::Pattern { xf, .. } = n.appearance.fill_paint() else { panic!("no pattern fill") };
        xf.as_coeffs()
    };
    let (before, xf) = (points(&s, a), tiles(&s));
    // The move snaps the path back to where it was, and its tiles go back with it.
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    s.execute("object.move", &json!({"dx": 0.3, "dy": 0.0, "patterns": true})).unwrap();
    assert_eq!(points(&s, a), before);
    assert!(tiles(&s).iter().zip(xf).all(|(t, x)| (t - x).abs() < 1e-9), "tiles: {:?}, before {xf:?}", tiles(&s));
    // A move that the snap keeps moves the tiles as far as the path.
    s.execute("object.move", &json!({"dx": 0.7, "dy": 0.0, "patterns": true})).unwrap();
    assert_eq!(points(&s, a)[0], (before[0].0 + 1.0, before[0].1));
    assert!((tiles(&s)[4] - (xf[4] + 1.0)).abs() < 1e-9, "tiles: {:?}, before {xf:?}", tiles(&s));
}

#[test]
fn objects_on_path_members_stay_on_the_grid() {
    let mut s = new_doc();
    let spine = s.execute("path.create", &json!({"d": "M 0.3 100.4 L 300.3 100.4"})).unwrap()["id"].as_u64().unwrap();
    let a = path(&mut s, "M 10 10 L 30 10 L 30 30 L 10 30 Z", 2.0);
    s.execute("object.objectsOnPath.attach", &json!({"path": spine, "ids": [a]})).unwrap();
    assert!(!whole(&points(&s, a)), "placed: {:?}", points(&s, a));
    // Placing the member on the path after the edit must not move it off the grid again.
    s.execute("object.setPixelAligned", &json!({"ids": [a]})).unwrap();
    assert!(whole(&points(&s, a)), "aligned: {:?}", points(&s, a));
    // Editing the path places the member again, and it goes back on the grid.
    let anchors = json!([{"x": 0.6, "y": 120.7}, {"x": 300.6, "y": 120.7}]);
    s.execute("path.setAnchors", &json!({"id": spine, "subpaths": [{"anchors": anchors, "closed": false}]})).unwrap();
    assert!(whole(&points(&s, a)), "path edited: {:?}", points(&s, a));
    assert!(points(&s, a).iter().all(|(_, y)| *y > 105.0), "followed the path: {:?}", points(&s, a));
}

fn align_new(s: &Session) -> bool {
    s.doc().unwrap().doc.setup.align_new_objects_to_pixel_grid
}

/// Align New Objects to Pixel Grid (`document.setup`): while it is on, a rectangle and an open path
/// drawn in the document are pixel-aligned and snapped in the step that makes them, and type is
/// not; art already in the document, duplicates and pastes keep their own setting; turning it off
/// leaves the next drawing as it is drawn, and the object's own setting can be turned off while
/// the document's stays on.
#[test]
fn align_new_objects_makes_new_paths_pixel_aligned() {
    let mut s = new_doc();
    assert!(!align_new(&s));
    assert_eq!(s.execute("document.setup", &json!({})).unwrap()["alignNewObjectsToPixelGrid"], false);
    let old = path(&mut s, "M10.3 120.6 L40.7 120.6 L40.7 130.3 L10.3 130.3 Z", 2.0);
    let old_points = points(&s, old);
    let steps = undo_len(&s);
    let r = s.execute("document.setup", &json!({"alignNewObjectsToPixelGrid": true})).unwrap();
    assert_eq!(r["alignNewObjectsToPixelGrid"], true);
    assert!(align_new(&s));
    assert_eq!(undo_len(&s), steps + 1, "one undo step");
    // Art already in the document stays as it was.
    assert!(!aligned(&s, old));
    assert_eq!(points(&s, old), old_points);
    // A rectangle is drawn pixel-aligned: with the 2 pt stroke `path` left as the current one, its
    // edges go to whole points.
    let steps = undo_len(&s);
    let rect = s.execute("shape.rectangle", &json!({"x": 10.3, "y": 20.6, "width": 30.4, "height": 9.7})).unwrap()["id"].as_u64().unwrap();
    assert!(aligned(&s, rect));
    assert_eq!(points(&s, rect), [(10.0, 21.0), (41.0, 21.0), (41.0, 30.0), (10.0, 30.0)]);
    assert_eq!(undo_len(&s), steps + 1, "made and snapped in one undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(NodeId(rect)).is_none());
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(aligned(&s, rect));
    // An open path from the Pen.
    let line = s.execute("path.create", &json!({"anchors": [{"x": 50.3, "y": 60.2}, {"x": 90.6, "y": 60.2}]})).unwrap()["id"].as_u64().unwrap();
    assert!(aligned(&s, line));
    // Its horizontal segment goes to a whole point.
    assert!(points(&s, line).iter().all(|(_, y)| *y == 60.0), "{:?}", points(&s, line));
    // Type cannot be pixel-aligned.
    let text = s.execute("text.create", &json!({"x": 10.3, "y": 200.6, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    assert!(!aligned(&s, text));
    // A duplicate and a paste of art that is not pixel-aligned keep it so.
    s.execute("select.set", &json!({"ids": [old]})).unwrap();
    s.execute("edit.duplicate", &json!({"dx": 100.3, "dy": 0.4})).unwrap();
    let copy = s.doc().unwrap().selection.objects[0].0;
    assert!(copy != old && !aligned(&s, copy));
    s.execute("select.set", &json!({"ids": [old]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    let pasted = s.doc().unwrap().selection.objects[0].0;
    assert!(pasted != old && pasted != copy && !aligned(&s, pasted));
    // The object's own setting turns off while the document's stays on.
    s.execute("object.setPixelAligned", &json!({"ids": [rect], "on": false})).unwrap();
    assert!(!aligned(&s, rect) && align_new(&s));
    // Off: the next rectangle is drawn as it is given, and the first one stays pixel-aligned.
    s.execute("document.setup", &json!({"alignNewObjectsToPixelGrid": false})).unwrap();
    let plain = s.execute("shape.rectangle", &json!({"x": 10.3, "y": 20.6, "width": 30.4, "height": 9.7})).unwrap()["id"].as_u64().unwrap();
    assert!(!aligned(&s, plain));
    let drawn = [(10.3, 20.6), (40.7, 20.6), (40.7, 30.3), (10.3, 30.3)];
    assert!(points(&s, plain).iter().zip(drawn).all(|(a, b)| close(*a, b)), "{:?}", points(&s, plain));
    assert!(aligned(&s, line));
    // A value that is not true or false changes nothing.
    let steps = undo_len(&s);
    assert!(s.execute("document.setup", &json!({"alignNewObjectsToPixelGrid": "yes"})).is_err());
    assert!(!align_new(&s));
    assert_eq!(undo_len(&s), steps);
}

/// While Align New Objects to Pixel Grid is on, the paths of a new rectangular grid, polar grid
/// and flare are pixel-aligned (the group is not), and a new 3D Box add-on shape is snapped as
/// Align to Pixel Grid snaps the same box drawn without it.
#[test]
fn align_new_objects_reaches_grids_flares_and_add_on_shapes() {
    let mut s = new_doc();
    s.execute("document.setup", &json!({"alignNewObjectsToPixelGrid": true})).unwrap();
    let doc = |s: &Session| s.doc().unwrap().doc.clone();
    let children = |s: &Session, id: u64| -> Vec<u64> { doc(s).node(NodeId(id)).unwrap().children().unwrap().iter().map(|c| c.id.0).collect() };
    let grid = s.execute("shape.rectangularGrid", &json!({"x": 10.3, "y": 20.6, "width": 100.4, "height": 50.7})).unwrap()["id"].as_u64().unwrap();
    assert!(!aligned(&s, grid));
    let lines = children(&s, grid);
    assert!(!lines.is_empty() && lines.iter().all(|id| aligned(&s, *id)));
    // Under the 1 pt stroke, a divider sits on a half point across its length, and the frame
    // has every corner on half points.
    let half = |v: f64| ((v - 0.5) - (v - 0.5).round()).abs() < 1e-9;
    let snapped =
        |p: &[(f64, f64)]| halves(p) || (p.iter().all(|q| q.1 == p[0].1) && half(p[0].1)) || (p.iter().all(|q| q.0 == p[0].0) && half(p[0].0));
    assert!(lines.iter().all(|id| snapped(&points(&s, *id))), "{:?}", lines.iter().map(|id| points(&s, *id)).collect::<Vec<_>>());
    let polar = s.execute("shape.polarGrid", &json!({"x": 200.3, "y": 20.6, "width": 80.4, "height": 80.7})).unwrap()["id"].as_u64().unwrap();
    assert!(!aligned(&s, polar) && children(&s, polar).iter().all(|id| aligned(&s, *id)));
    let flare = s.execute("shape.flare", &json!({"cx": 100.3, "cy": 200.6})).unwrap()["id"].as_u64().unwrap();
    assert!(!aligned(&s, flare) && children(&s, flare).iter().all(|id| aligned(&s, *id)));
    // Seen straight on, the box's front face has horizontal and vertical edges to snap.
    let params = json!({"x": 20.3, "y": 300.6, "width": 60.4, "height": 40.7, "angleX": 0, "angleY": 0, "angleZ": 0});
    let drawn = s.execute("addon.shapes.box3d", &params).unwrap()["id"].as_u64().unwrap();
    assert!(aligned(&s, drawn));
    s.execute("document.setup", &json!({"alignNewObjectsToPixelGrid": false})).unwrap();
    let later = s.execute("addon.shapes.box3d", &params).unwrap()["id"].as_u64().unwrap();
    assert!(!aligned(&s, later));
    let unaligned = points(&s, later);
    s.execute("object.setPixelAligned", &json!({"ids": [later]})).unwrap();
    assert_eq!(points(&s, drawn), points(&s, later));
    assert_ne!(points(&s, drawn), unaligned, "the box has segments to snap");
}

/// The document's Align New Objects to Pixel Grid is saved in the native file only when on and
/// read back; a file from before it opens with it off.
#[test]
fn align_new_objects_is_saved_and_read_back() {
    let mut s = new_doc();
    let off = serde_json::to_value(&s.doc().unwrap().doc.setup).unwrap();
    assert!(off.get("alignNewObjectsToPixelGrid").is_none(), "{off}");
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert!(!back.setup.align_new_objects_to_pixel_grid);
    s.execute("document.setup", &json!({"alignNewObjectsToPixelGrid": true})).unwrap();
    let on = serde_json::to_value(&s.doc().unwrap().doc.setup).unwrap();
    assert_eq!(on["alignNewObjectsToPixelGrid"], true);
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert!(back.setup.align_new_objects_to_pixel_grid);
    let mut older = on;
    older.as_object_mut().unwrap().remove("alignNewObjectsToPixelGrid");
    assert!(!serde_json::from_value::<vectorcraft_doc::DocSetup>(older).unwrap().align_new_objects_to_pixel_grid);
}

fn selected_ids(s: &Session) -> Vec<u64> {
    let mut ids: Vec<u64> = s.doc().unwrap().selection.objects.iter().map(|i| i.0).collect();
    ids.sort_unstable();
    ids
}

/// Select › Object › Not Aligned to Pixel Grid selects the paths and compound paths whose
/// property is off, whole coordinates or not, in groups too, and replaces the selection; a compound
/// path counts as one object; type, guides and aligned paths stay out; the art, its property and
/// the undo history stay as they are.
#[test]
fn not_aligned_to_pixel_grid_selects_paths_without_the_property() {
    let mut s = new_doc();
    let fractional = path(&mut s, "M10.3 20.6 L40.7 20.6 L40.7 30.3 L10.3 30.3 Z", 1.0);
    let whole_but_off = path(&mut s, "M50 50 L60 50 L60 60 L50 60 Z", 1.0);
    let on = path(&mut s, "M70.4 70 L80 70 L80 80 L70.4 80 Z", 1.0);
    s.execute("object.setPixelAligned", &json!({"on": true})).unwrap();
    let guide = path(&mut s, "M0 200 L400 200", 1.0);
    s.execute("view.guides.make", &json!({})).unwrap();
    s.execute("text.create", &json!({"x": 100, "y": 100, "text": "type"})).unwrap();
    let m1 = path(&mut s, "M100 100 L120 100 L120 120 L100 120 Z", 1.0);
    let m2 = path(&mut s, "M130 100 L150 100 L150 120 L130 120 Z", 1.0);
    s.execute("select.set", &json!({"ids": [m1, m2]})).unwrap();
    let compound = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    let grouped = path(&mut s, "M200 200 L210 200 L210 210 L200 210 Z", 1.0);
    s.execute("object.group", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [on]})).unwrap();
    let before = points(&s, fractional);
    let steps = undo_len(&s);
    let r = s.execute("select.object.notAlignedToPixelGrid", &json!({})).unwrap();
    let mut want = vec![fractional, whole_but_off, compound, grouped];
    want.sort_unstable();
    assert_eq!(r["count"], 4);
    assert_eq!(selected_ids(&s), want);
    assert!(!selected_ids(&s).contains(&guide));
    assert_eq!(points(&s, fractional), before);
    assert!(!aligned(&s, fractional) && aligned(&s, on));
    assert_eq!(undo_len(&s), steps);
    // With every such object aligned, nothing is selected.
    s.execute("object.setPixelAligned", &json!({"on": true})).unwrap();
    let r = s.execute("select.object.notAlignedToPixelGrid", &json!({})).unwrap();
    assert_eq!(r["count"], 0);
    assert!(selected_ids(&s).is_empty());
}

/// Not Aligned to Pixel Grid leaves out hidden and locked paths and, in isolation mode, the art
/// outside the isolated group.
#[test]
fn not_aligned_to_pixel_grid_leaves_out_what_cant_be_selected() {
    let mut s = new_doc();
    let hidden = path(&mut s, "M10 10 L20 10 L20 20 L10 20 Z", 1.0);
    s.execute("object.setProps", &json!({"ids": [hidden], "visible": false})).unwrap();
    let locked = path(&mut s, "M30 10 L40 10 L40 20 L30 20 Z", 1.0);
    s.execute("object.setProps", &json!({"ids": [locked], "locked": true})).unwrap();
    let outside = path(&mut s, "M50 10 L60 10 L60 20 L50 20 Z", 1.0);
    let inside = path(&mut s, "M70 10 L80 10 L80 20 L70 20 Z", 1.0);
    let g = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(s.execute("select.object.notAlignedToPixelGrid", &json!({})).unwrap()["count"], 2);
    assert_eq!(selected_ids(&s), {
        let mut v = vec![outside, inside];
        v.sort_unstable();
        v
    });
    s.execute("object.isolate", &json!({"id": g})).unwrap();
    assert_eq!(s.execute("select.object.notAlignedToPixelGrid", &json!({})).unwrap()["count"], 1);
    assert_eq!(selected_ids(&s), vec![inside]);
}
