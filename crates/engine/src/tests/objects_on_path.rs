//! Object › Objects on Path (#2545): objects attached to a path, spread along it and turned to
//! follow it, kept there while the path is edited. Driven through `Session::execute`.

use serde_json::{Value, json};
use vectorcraft_geom::{Point, Rect};

use crate::tests_clip::{id_of, select};
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

fn line(s: &mut Session, a: (f64, f64), b: (f64, f64)) -> NodeId {
    id_of(&s.execute("shape.line", &json!({"x1": a.0, "y1": a.1, "x2": b.0, "y2": b.1})).unwrap())
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

fn center(s: &Session, id: NodeId) -> Point {
    let c = bounds(s, id).center();
    Point::new((c.x * 1000.0).round() / 1000.0, (c.y * 1000.0).round() / 1000.0)
}

fn attach(s: &mut Session, path: NodeId, objects: &[NodeId], extra: Value) -> NodeId {
    select(s, objects);
    let mut p = json!({ "path": path.0 });
    if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    id_of(&s.execute("object.objectsOnPath.attach", &p).unwrap())
}

fn set_line(s: &mut Session, id: NodeId, a: (f64, f64), b: (f64, f64)) {
    let anchors = json!([{"x": a.0, "y": a.1}, {"x": b.0, "y": b.1}]);
    s.execute("path.setAnchors", &json!({"id": id.0, "subpaths": [{"anchors": anchors, "closed": false}]})).unwrap();
}

/// Three 20 pt squares, out of order on the canvas, and a 300 pt line from (50, 300) to the right.
fn squares_and_line(s: &mut Session) -> ([NodeId; 3], NodeId) {
    let right = rect(s, 250.0, 20.0, 20.0, 20.0);
    let left = rect(s, 10.0, 100.0, 20.0, 20.0);
    let middle = rect(s, 120.0, 0.0, 20.0, 20.0);
    let path = line(s, (50.0, 300.0), (350.0, 300.0));
    ([left, middle, right], path)
}

#[test]
fn attach_spreads_the_objects_evenly_along_an_open_path_from_left_to_right() {
    let mut s = session();
    let ([left, middle, right], path) = squares_and_line(&mut s);
    let g = attach(&mut s, path, &[right, left, middle], json!({}));
    let d = &s.doc().unwrap().doc;
    let n = d.node(g).unwrap();
    assert_eq!(n.kind_label(), "Objects on Path");
    assert_eq!(n.children().unwrap().first().map(|c| c.id), Some(path), "the path is at the bottom of the group");
    let rec = n.extras().objects_on_path.as_deref().unwrap();
    assert_eq!(rec.members.iter().map(|m| m.id).collect::<Vec<_>>(), [left, middle, right], "left to right on the canvas");
    assert_eq!(s.doc().unwrap().selection.objects.to_vec(), [g]);
    // First on the start, last on the end, evenly between: the centers on the line.
    assert_eq!(center(&s, left), Point::new(50.0, 300.0));
    assert_eq!(center(&s, middle), Point::new(200.0, 300.0));
    assert_eq!(center(&s, right), Point::new(350.0, 300.0));
    let b = bounds(&s, middle);
    assert!((b.width() - 20.0).abs() < 1e-9 && (b.height() - 20.0).abs() < 1e-9, "upright on a level path: {b:?}");
}

#[test]
fn objects_in_a_column_go_top_to_bottom() {
    let mut s = session();
    // Its left edge is the leftmost, but it is below the other one in their column.
    let lower = rect(&mut s, 8.0, 80.0, 20.0, 20.0);
    let upper = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    let beside = rect(&mut s, 100.0, 0.0, 20.0, 20.0);
    let path = line(&mut s, (0.0, 300.0), (300.0, 300.0));
    let g = attach(&mut s, path, &[beside, lower, upper], json!({}));
    let ids: Vec<NodeId> = s.doc().unwrap().doc.node(g).unwrap().extras().objects_on_path.as_deref().unwrap().members.iter().map(|m| m.id).collect();
    assert_eq!(ids, [upper, lower, beside]);
}

#[test]
fn the_objects_turn_with_the_path_and_rotate_tilts_them() {
    let mut s = session();
    // A 40 × 10 bar on a path that runs straight down: it turns a quarter.
    let bar = rect(&mut s, 0.0, 0.0, 40.0, 10.0);
    let other = rect(&mut s, 100.0, 0.0, 40.0, 10.0);
    let path = line(&mut s, (200.0, 50.0), (200.0, 350.0));
    let g = attach(&mut s, path, &[bar, other], json!({}));
    let b = bounds(&s, bar);
    assert!((b.width() - 10.0).abs() < 1e-6 && (b.height() - 40.0).abs() < 1e-6, "across the path: {b:?}");
    assert_eq!(center(&s, bar), Point::new(200.0, 50.0));
    // Rotate 90: turned back level, still centered on the path.
    s.execute("object.objectsOnPath.options", &json!({"rotate": 90})).unwrap();
    let b = bounds(&s, bar);
    assert!((b.width() - 40.0).abs() < 1e-6 && (b.height() - 10.0).abs() < 1e-6, "tilted by Rotate: {b:?}");
    assert_eq!(center(&s, other), Point::new(200.0, 350.0));
    let rec = s.doc().unwrap().doc.node(g).unwrap().extras().objects_on_path.as_deref().cloned().unwrap();
    assert_eq!((rec.pivot, rec.rotate), (4, 90.0));
    // The same angle given to Attach.
    let mut t = session();
    let bar = rect(&mut t, 0.0, 0.0, 40.0, 10.0);
    let path = line(&mut t, (200.0, 50.0), (200.0, 350.0));
    attach(&mut t, path, &[bar], json!({"rotate": 90}));
    let b = bounds(&t, bar);
    assert!((b.width() - 40.0).abs() < 1e-6 && (b.height() - 10.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn a_turned_object_attaches_square_to_the_path_or_across_it() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 40.0, 10.0);
    let b = rect(&mut s, 100.0, 0.0, 40.0, 10.0);
    s.execute("object.rotate", &json!({"ids": [a.0], "angle": 30})).unwrap();
    s.execute("object.rotate", &json!({"ids": [b.0], "angle": 60})).unwrap();
    let path = line(&mut s, (50.0, 300.0), (350.0, 300.0));
    attach(&mut s, path, &[a, b], json!({}));
    let (ba, bb) = (bounds(&s, a), bounds(&s, b));
    assert!((ba.width() - 40.0).abs() < 1e-6 && (ba.height() - 10.0).abs() < 1e-6, "30° rounds to level: {ba:?}");
    assert!((bb.width() - 10.0).abs() < 1e-6 && (bb.height() - 40.0).abs() < 1e-6, "60° rounds to upright: {bb:?}");
}

#[test]
fn a_reflected_group_keeps_the_mirrored_tilt() {
    let mut s = session();
    let bar = rect(&mut s, 0.0, 0.0, 40.0, 10.0);
    let other = rect(&mut s, 100.0, 0.0, 40.0, 10.0);
    let path = line(&mut s, (50.0, 300.0), (350.0, 300.0));
    let g = attach(&mut s, path, &[bar, other], json!({"rotate": 30}));
    // The x of the bar's topmost corner, from its center: tilted counter-clockwise, it is on the right.
    let top_x = |s: &Session| {
        let n = s.doc().unwrap().doc.node(bar).unwrap();
        let anchors = &n.path_data().unwrap().subpaths[0].anchors;
        let top = anchors.iter().min_by(|a, b| a.p.y.total_cmp(&b.p.y)).unwrap().p;
        top.x - bounds(s, bar).center().x
    };
    assert!(top_x(&s) > 1.0);
    select(&mut s, &[g]);
    s.execute("object.reflect", &json!({"axis": "horizontal"})).unwrap();
    assert!(top_x(&s) < -1.0, "mirrored: tilted clockwise");
    let rec = s.doc().unwrap().doc.node(g).unwrap().extras().objects_on_path.as_deref().cloned().unwrap();
    assert_eq!(rec.rotate, -30.0);
}

#[test]
fn scaling_the_group_out_of_proportion_keeps_the_scaled_tilt_and_scaling_back_restores_it() {
    let mut s = session();
    let bar = rect(&mut s, 0.0, 0.0, 40.0, 10.0);
    let other = rect(&mut s, 100.0, 0.0, 40.0, 10.0);
    let path = line(&mut s, (50.0, 300.0), (350.0, 300.0));
    let g = attach(&mut s, path, &[bar, other], json!({"rotate": 45}));
    let anchors = |s: &Session| -> Vec<(f64, f64)> {
        let n = s.doc().unwrap().doc.node(bar).unwrap().clone();
        n.path_data().unwrap().subpaths[0].anchors.iter().map(|a| (a.p.x, a.p.y)).collect()
    };
    let before = anchors(&s);
    select(&mut s, &[g]);
    s.execute("object.scale", &json!({"sx": 200, "sy": 100, "origin": [200, 300]})).unwrap();
    // The bar's long edge was at 45°: twice as wide, it slopes at atan(1/2).
    let a = anchors(&s);
    let (dx, dy) = (a[1].0 - a[0].0, a[1].1 - a[0].1);
    assert!((dy.abs().atan2(dx.abs()).to_degrees() - 0.5f64.atan().to_degrees()).abs() < 1e-6, "{a:?}");
    s.execute("object.scale", &json!({"sx": 50, "sy": 100, "origin": [200, 300]})).unwrap();
    for (p, q) in anchors(&s).iter().zip(&before) {
        assert!((p.0 - q.0).abs() < 1e-6 && (p.1 - q.1).abs() < 1e-6, "{:?} against {before:?}", anchors(&s));
    }
}

#[test]
fn pivot_picks_the_point_that_sits_on_the_path() {
    let mut s = session();
    let ([left, middle, right], path) = squares_and_line(&mut s);
    attach(&mut s, path, &[left, middle, right], json!({"pivot": 7}));
    // The middle of each square's bottom edge is on the line: the squares stand on it.
    let b = bounds(&s, middle);
    assert!((b.y1 - 300.0).abs() < 1e-6 && (b.center().x - 200.0).abs() < 1e-6, "{b:?}");
    s.execute("object.objectsOnPath.options", &json!({"pivot": 1})).unwrap();
    let b = bounds(&s, middle);
    assert!((b.y0 - 300.0).abs() < 1e-6, "hanging from the line by their top edge: {b:?}");
}

#[test]
fn a_closed_path_spreads_the_objects_all_the_way_round_from_its_first_anchor() {
    let mut s = session();
    let ids: Vec<NodeId> = (0..4).map(|i| rect(&mut s, 10.0 + 40.0 * f64::from(i), 0.0, 10.0, 10.0)).collect();
    let ring = rect(&mut s, 100.0, 100.0, 200.0, 200.0);
    let first = s.doc().unwrap().doc.node(ring).unwrap().path_data().unwrap().subpaths[0].anchors[0].p;
    attach(&mut s, ring, &ids, json!({}));
    assert_eq!(center(&s, ids[0]), first, "the first object on the first anchor");
    let mut at: Vec<(i64, i64)> = ids.iter().map(|id| center(&s, *id)).map(|p| (p.x.round() as i64, p.y.round() as i64)).collect();
    at.sort();
    assert_eq!(at, [(100, 100), (100, 300), (300, 100), (300, 300)], "a quarter of the way round apart");
}

#[test]
fn editing_the_path_moves_the_objects_and_undo_takes_it_back() {
    let mut s = session();
    let ([left, middle, right], path) = squares_and_line(&mut s);
    attach(&mut s, path, &[left, middle, right], json!({}));
    set_line(&mut s, path, (50.0, 250.0), (250.0, 250.0));
    assert_eq!(center(&s, left), Point::new(50.0, 250.0));
    assert_eq!(center(&s, middle), Point::new(150.0, 250.0));
    assert_eq!(center(&s, right), Point::new(250.0, 250.0));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(center(&s, middle), Point::new(200.0, 300.0), "one undo step puts the path and the objects back");
    // Attach itself is one step.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(center(&s, middle), Point::new(130.0, 10.0));
    let d = &s.doc().unwrap().doc;
    assert!(d.layers.iter().all(|l| l.children().unwrap().iter().all(|c| !vectorcraft_doc::objects_on_path::is_objects_on_path(c))));
}

#[test]
fn an_object_moved_on_its_own_goes_back_on_the_path() {
    let mut s = session();
    let ([left, middle, right], path) = squares_and_line(&mut s);
    let g = attach(&mut s, path, &[left, middle, right], json!({}));
    s.execute("object.move", &json!({"ids": [middle.0], "dx": 30, "dy": -40})).unwrap();
    assert_eq!(center(&s, middle), Point::new(200.0, 300.0));
    // The whole group moves as one.
    select(&mut s, &[g]);
    s.execute("object.move", &json!({"dx": 10, "dy": 20})).unwrap();
    assert_eq!(center(&s, middle), Point::new(210.0, 320.0));
    assert_eq!(center(&s, right), Point::new(360.0, 320.0));
}

#[test]
fn objects_moved_into_the_group_join_the_end_and_ones_moved_out_leave() {
    let mut s = session();
    let ([left, middle, right], path) = squares_and_line(&mut s);
    let g = attach(&mut s, path, &[left, middle], json!({}));
    s.execute("layer.move", &json!({"ids": [right.0], "target": g.0, "place": "inside"})).unwrap();
    assert_eq!(center(&s, left), Point::new(50.0, 300.0));
    assert_eq!(center(&s, middle), Point::new(200.0, 300.0));
    assert_eq!(center(&s, right), Point::new(350.0, 300.0), "the new object goes at the end");
    s.execute("layer.move", &json!({"ids": [middle.0], "target": g.0, "place": "above"})).unwrap();
    let rec = s.doc().unwrap().doc.node(g).unwrap().extras().objects_on_path.as_deref().cloned().unwrap();
    assert_eq!(rec.members.iter().map(|m| m.id).collect::<Vec<_>>(), [left, right]);
    assert_eq!(center(&s, right), Point::new(350.0, 300.0), "two objects stay on the ends");
}

#[test]
fn art_that_replaces_an_object_keeps_its_place_on_the_path() {
    let mut s = session();
    let mark = rect(&mut s, 0.0, 0.0, 20.0, 20.0);
    let first = id_of(&s.execute("symbol.new", &json!({"name": "Mark", "ids": [mark.0]})).unwrap());
    let second = id_of(&s.execute("symbol.place", &json!({"name": "Mark", "x": 100, "y": 10})).unwrap());
    let third = id_of(&s.execute("symbol.place", &json!({"name": "Mark", "x": 200, "y": 10})).unwrap());
    let path = line(&mut s, (50.0, 300.0), (350.0, 300.0));
    let g = attach(&mut s, path, &[first, second, third], json!({}));
    let v = s.execute("symbol.breakLink", &json!({"ids": [first.0]})).unwrap();
    let art = v["ids"].as_array().and_then(|a| a.first()).and_then(Value::as_u64).map(NodeId).unwrap();
    let rec = s.doc().unwrap().doc.node(g).unwrap().extras().objects_on_path.as_deref().cloned().unwrap();
    assert_eq!(rec.members.iter().map(|m| m.id).collect::<Vec<_>>(), [art, second, third], "the art takes the instance's place");
    assert_eq!(center(&s, art), Point::new(50.0, 300.0));
    assert_eq!(center(&s, third), Point::new(350.0, 300.0));
}

#[test]
fn a_shaper_group_stays_on_the_path_when_a_source_changes() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    rect(&mut s, 50.0, 0.0, 100.0, 100.0);
    let pts = [[25.0, 20.0], [75.0, 35.0], [30.0, 50.0], [125.0, 65.0]];
    let shaper = NodeId(s.execute("shaper.scribble", &json!({"points": pts})).unwrap()["id"].as_u64().unwrap());
    let other = rect(&mut s, 300.0, 0.0, 20.0, 20.0);
    let path = line(&mut s, (50.0, 300.0), (350.0, 300.0));
    attach(&mut s, path, &[shaper, other], json!({}));
    assert_eq!(center(&s, shaper), Point::new(50.0, 300.0));
    select(&mut s, &[a]);
    s.execute("object.setBounds", &json!({"width": 200.0, "height": 100.0, "reference": 0})).unwrap();
    // The rebuilt art is wider, and its center is still on the start of the path.
    assert!(bounds(&s, shaper).width() > 180.0, "{:?}", bounds(&s, shaper));
    assert_eq!(center(&s, shaper), Point::new(50.0, 300.0));
}

#[test]
fn the_group_saves_and_reopens_live_and_exports_as_plain_art() {
    let mut s = session();
    let ([left, middle, right], path) = squares_and_line(&mut s);
    let g = attach(&mut s, path, &[left, middle, right], json!({"rotate": 30, "pivot": 7}));
    let data = s.execute("document.export", &json!({"format": "vectorcraft"})).unwrap()["dataBase64"].clone();
    let mut o = Session::new();
    o.execute("document.open", &json!({"name": "objects.vectorcraft", "dataBase64": data})).unwrap();
    let before = s.doc().unwrap().doc.node(g).unwrap().extras().objects_on_path.clone();
    assert_eq!(o.doc().unwrap().doc.node(g).unwrap().extras().objects_on_path, before);
    set_line(&mut o, path, (50.0, 200.0), (350.0, 200.0));
    assert!((bounds(&o, middle).center().x - bounds(&s, middle).center().x).abs() < 1e-6);
    assert!(bounds(&o, middle).center().y < 210.0, "editing the reopened path moves the objects");
    for format in ["svg", "pdf"] {
        let data = s.execute("document.export", &json!({"format": format})).unwrap()["dataBase64"].clone();
        let mut o = Session::new();
        o.execute("document.open", &json!({"name": format!("objects.{format}"), "dataBase64": data})).unwrap();
        let r = vectorcraft_render::Renderer::new().render(&o.doc().unwrap().doc, 400, 400, vectorcraft_geom::Affine::IDENTITY, &Default::default());
        let c = bounds(&s, middle).center();
        assert!(r.pixel(c.x as u32, c.y as u32 - 4)[3] > 128, "{format}: the square is drawn where it is attached");
        assert!(r.pixel(130, 10)[3] < 10, "{format}: not where it was drawn");
    }
}

#[test]
fn a_pasted_copy_stays_live_on_its_own_path() {
    let mut s = session();
    let ([left, middle, right], path) = squares_and_line(&mut s);
    let g = attach(&mut s, path, &[left, middle, right], json!({}));
    select(&mut s, &[g]);
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    let copy = s.doc().unwrap().selection.objects.to_vec();
    let [copy] = copy[..] else { panic!("{copy:?}") };
    assert_ne!(copy, g);
    let n = s.doc().unwrap().doc.node(copy).unwrap().clone();
    let rec = n.extras().objects_on_path.as_deref().cloned().unwrap();
    let kids: Vec<NodeId> = n.children().unwrap().iter().map(|c| c.id).collect();
    assert_eq!(kids.first(), Some(&rec.path), "the copy names its own path");
    assert!(rec.members.iter().all(|m| kids.contains(&m.id) && ![left, middle, right].contains(&m.id)));
    let (a, b) = (s.doc().unwrap().doc.node(rec.path).unwrap().geometric_bounds().unwrap(), bounds(&s, path));
    set_line(&mut s, rec.path, (a.x0, a.y0 - 50.0), (a.x1, a.y0 - 50.0));
    assert!((bounds(&s, rec.members[1].id).center().y - (a.y0 - 50.0)).abs() < 1e-6, "the copy's objects follow the copy's path");
    assert_eq!(bounds(&s, path), b);
    assert_eq!(center(&s, middle), Point::new(200.0, 300.0), "the original stays");
}

#[test]
fn expand_leaves_a_plain_group() {
    let mut s = session();
    let ([left, middle, right], path) = squares_and_line(&mut s);
    let g = attach(&mut s, path, &[left, middle, right], json!({}));
    let v = s.execute("object.objectsOnPath.expand", &json!({})).unwrap();
    assert_eq!(v["ids"], json!([g.0]));
    let n = s.doc().unwrap().doc.node(g).unwrap().clone();
    assert_eq!(n.kind_label(), "Group");
    assert_eq!(n.children().unwrap().len(), 4);
    set_line(&mut s, path, (50.0, 250.0), (250.0, 250.0));
    assert_eq!(center(&s, middle), Point::new(200.0, 300.0), "the objects stay where they were");
    assert!(s.execute("object.objectsOnPath.expand", &json!({})).is_err(), "nothing left to expand");
}

#[test]
fn attach_without_a_path_chooses_the_tool() {
    let mut s = session();
    let ([left, ..], _) = squares_and_line(&mut s);
    select(&mut s, &[left]);
    assert_eq!(s.execute("object.objectsOnPath.attach", &json!({})).unwrap()["tool"], "objectsOnPath");
    assert_eq!(s.tool_id(), "objectsOnPath");
}

#[test]
fn what_cannot_be_attached_is_refused_with_a_reason() {
    let mut s = session();
    let ([left, _, right], path) = squares_and_line(&mut s);
    let err = |s: &mut Session, p: Value| s.execute("object.objectsOnPath.attach", &p).unwrap_err().to_string();
    select(&mut s, &[path]);
    assert!(err(&mut s, json!({"path": path.0})).contains("select the objects"), "the path alone");
    let dot = line(&mut s, (10.0, 10.0), (10.0, 10.0));
    assert!(err(&mut s, json!({"path": dot.0, "ids": [left.0]})).contains("pick a path"), "a path without length");
    let guide = line(&mut s, (0.0, 380.0), (300.0, 380.0));
    select(&mut s, &[guide]);
    s.execute("view.guides.make", &json!({})).unwrap();
    assert!(err(&mut s, json!({"path": guide.0, "ids": [right.0]})).contains("pick a path"), "a guide");
    let g = attach(&mut s, path, &[right], json!({}));
    let other = line(&mut s, (0.0, 350.0), (300.0, 350.0));
    assert!(err(&mut s, json!({"path": other.0, "ids": [right.0]})).contains("already on a path"));
    assert!(err(&mut s, json!({"path": path.0, "ids": [left.0]})).contains("already in an Objects on Path group"));
    assert!(err(&mut s, json!({"path": other.0, "ids": [g.0], "pivot": 9})).contains("pivot"));
    assert!(s.execute("object.objectsOnPath.options", &json!({"ids": [other.0]})).is_err());
}

#[test]
fn paths_and_objects_that_are_part_of_live_art_or_a_clipping_path_stay_where_they_are() {
    let mut s = session();
    let ([left, middle, _], path) = squares_and_line(&mut s);
    let err = |s: &mut Session, p: Value| s.execute("object.objectsOnPath.attach", &p).unwrap_err().to_string();
    let parent = |s: &Session, id: NodeId| s.doc().unwrap().doc.parent_of(id);
    // A compound path's member as the path; as an object, the whole compound attaches.
    let member = rect(&mut s, 20.0, 150.0, 40.0, 40.0);
    let hole = rect(&mut s, 30.0, 160.0, 10.0, 10.0);
    select(&mut s, &[member, hole]);
    let compound = id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap());
    let why = err(&mut s, json!({"path": member.0, "ids": [left.0]}));
    assert!(why.contains("a path inside a compound path, blend, envelope, repeat or Intertwine group can't take objects"), "{why}");
    assert_eq!(parent(&s, member), Some(compound));
    // A blend's key path, as the path and as an object.
    let key = line(&mut s, (50.0, 200.0), (350.0, 200.0));
    let other_key = line(&mut s, (50.0, 250.0), (350.0, 250.0));
    select(&mut s, &[key, other_key]);
    let blend = id_of(&s.execute("object.blend.make", &json!({"steps": 2})).unwrap());
    assert!(err(&mut s, json!({"path": key.0, "ids": [left.0]})).contains("can't take objects"));
    let why = err(&mut s, json!({"path": path.0, "ids": [key.0]}));
    assert!(why.contains("objects inside a blend, envelope, repeat or Intertwine group can't be attached to a path"), "{why}");
    assert_eq!((parent(&s, key), parent(&s, other_key)), (Some(blend), Some(blend)));
    // A repeat's source art as an object.
    let source = rect(&mut s, 300.0, 340.0, 10.0, 10.0);
    select(&mut s, &[source]);
    let repeat = id_of(&s.execute("object.repeat.grid", &json!({})).unwrap());
    assert!(err(&mut s, json!({"path": path.0, "ids": [source.0]})).contains("can't be attached to a path"));
    assert_eq!(parent(&s, source), Some(repeat));
    // A clipping path, as the path and as an object.
    let art = rect(&mut s, 200.0, 100.0, 60.0, 60.0);
    let mask = rect(&mut s, 210.0, 110.0, 40.0, 40.0);
    select(&mut s, &[art, mask]);
    let clip = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    assert!(err(&mut s, json!({"path": mask.0, "ids": [left.0]})).contains("a clipping path can't take objects"));
    assert!(err(&mut s, json!({"path": path.0, "ids": [mask.0]})).contains("a clipping path can't be attached to a path"));
    assert_eq!(s.doc().unwrap().doc.node(clip).unwrap().children().unwrap().first().map(|c| c.id), Some(mask));
    // The art the clipping path clips, and a compound path's member as an object, still attach.
    let g = attach(&mut s, path, &[left, middle, member, art], json!({}));
    let rec = s.doc().unwrap().doc.node(g).unwrap().extras().objects_on_path.as_deref().cloned().unwrap();
    assert!(rec.members.iter().any(|m| m.id == compound), "the compound attaches whole");
    assert_eq!(parent(&s, member), Some(compound));
}

#[test]
fn options_and_expand_take_ids_without_a_selection() {
    let mut s = session();
    let ([left, middle, right], path) = squares_and_line(&mut s);
    let g = attach(&mut s, path, &[left, middle, right], json!({}));
    s.execute("select.set", &json!({"ids": []})).unwrap();
    // An object in the group names the group.
    let v = s.execute("object.objectsOnPath.options", &json!({"ids": [middle.0], "pivot": 7})).unwrap();
    assert_eq!(v["ids"], json!([g.0]));
    assert!((bounds(&s, middle).y1 - 300.0).abs() < 1e-6, "standing on the line");
    let v = s.execute("object.objectsOnPath.expand", &json!({"ids": [g.0]})).unwrap();
    assert_eq!(v["ids"], json!([g.0]));
    assert_eq!(s.doc().unwrap().doc.node(g).unwrap().kind_label(), "Group");
    assert!(s.execute("object.objectsOnPath.expand", &json!({"ids": [g.0]})).is_err(), "no longer an Objects on Path group");
}
