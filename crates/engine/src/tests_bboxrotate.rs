//! Rotated bounding boxes (#39): a rotation keeps the objects' angle, the bounding box and its
//! handles turn with them, sizes are measured along their own axes, and Reset Bounding Box squares
//! the box to the page again.

use serde_json::{Value, json};
use vectorcraft_doc::OrientedBox;
use vectorcraft_geom::{Point, Rect};
use vectorcraft_tools::bbox::Handle;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

fn sel(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn angle(s: &Session, id: NodeId) -> f64 {
    s.doc().unwrap().doc.node(id).unwrap().bbox_angle
}

fn selection_box(s: &Session) -> OrientedBox {
    s.transform_box(&s.doc().unwrap().selection.objects).unwrap()
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// A 100 × 50 rectangle centred on (200, 200), selected and turned to 45°.
fn turned_rect(s: &mut Session) -> NodeId {
    let a = rect(s, 150.0, 175.0, 100.0, 50.0);
    sel(s, &[a]);
    s.execute("object.rotate", &json!({"angle": 45, "absolute": true})).unwrap();
    a
}

#[test]
fn rotating_to_45_keeps_45() {
    let mut s = session();
    let n = undo_len(&s);
    let a = turned_rect(&mut s);
    assert_eq!(undo_len(&s), n + 2, "the rectangle, then one step for the rotation");
    // The panel's angle and the inspector's.
    let b = selection_box(&s);
    assert!(near(b.angle, 45.0) && near(angle(&s, a), 45.0), "{b:?}");
    assert!(near(s.execute("document.inspect", &json!({})).unwrap()["selectionRotation"].as_f64().unwrap(), 45.0));
    // The box hugs the rectangle: its own sides, its own centre.
    assert!(near(b.rect.width(), 100.0) && near(b.rect.height(), 50.0), "{b:?}");
    assert!(b.center().distance(Point::new(200.0, 200.0)) < 1e-6);
    // Absolute angles: entering 45 again changes nothing, 90 turns it by 45 more.
    s.execute("object.rotate", &json!({"angle": 45, "absolute": true})).unwrap();
    assert!(near(angle(&s, a), 45.0));
    s.execute("object.rotate", &json!({"angle": 90, "absolute": true})).unwrap();
    assert!(near(angle(&s, a), 90.0));
    let b = s.doc().unwrap().doc.bounds_of(&[a], false).unwrap();
    assert!(near(b.width(), 50.0) && near(b.height(), 100.0), "{b:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(angle(&s, a), 45.0));
}

#[test]
fn handles_turn_with_the_object_and_scale_along_its_axes() {
    let mut s = session();
    let a = turned_rect(&mut s);
    let b = selection_box(&s);
    // The Right handle sits on the turned box, 50 pt from the centre along 45° (up and right).
    let right = b.to_doc() * Handle::Right.pos(b.rect);
    let d = std::f64::consts::FRAC_1_SQRT_2 * 50.0;
    assert!(right.distance(Point::new(200.0 + d, 200.0 - d)) < 1e-6, "{right:?}");
    // Drag it 50 pt further out along the object's own x axis.
    let to = Point::new(right.x + d, right.y - d);
    let n = undo_len(&s);
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    for (kind, p) in [(PointerKind::Down, right), (PointerKind::Drag, to), (PointerKind::Up, to)] {
        s.pointer(&PointerEvent::new(kind, p.x, p.y).with_mods(Mods::default()), v).unwrap();
    }
    assert_eq!(undo_len(&s), n + 1);
    // 150 × 50 along its own axes, still at exactly 45°, the left side where it was.
    assert_eq!(angle(&s, a), b.angle);
    let nb = selection_box(&s);
    assert!(near(nb.rect.width(), 150.0) && near(nb.rect.height(), 50.0), "{nb:?}");
    let left = |b: &OrientedBox| b.to_doc() * Handle::Left.pos(b.rect);
    assert!(left(&nb).distance(left(&b)) < 1e-6);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(selection_box(&s).rect.width(), 100.0));
}

#[test]
fn transform_panel_sizes_follow_the_turned_box() {
    let mut s = session();
    let a = turned_rect(&mut s);
    let n = undo_len(&s);
    s.execute("object.setBounds", &json!({"width": 200, "reference": 4})).unwrap();
    assert_eq!(undo_len(&s), n + 1);
    let b = selection_box(&s);
    assert!(near(b.rect.width(), 200.0) && near(b.rect.height(), 50.0) && near(b.angle, 45.0), "{b:?}");
    assert!(b.center().distance(Point::new(200.0, 200.0)) < 1e-6);
    // X/Y place the box's own reference point on the page.
    s.execute("object.setBounds", &json!({"x": 300, "y": 100, "reference": 0})).unwrap();
    let b = selection_box(&s);
    assert!(b.reference_point(0).distance(Point::new(300.0, 100.0)) < 1e-6, "{b:?}");
    assert!(near(angle(&s, a), 45.0));
}

#[test]
fn reset_bounding_box_squares_the_box_in_one_step() {
    let mut s = session();
    let a = turned_rect(&mut s);
    let geometry = s.doc().unwrap().doc.bounds_of(&[a], false).unwrap();
    let n = undo_len(&s);
    assert_eq!(s.execute("object.resetBoundingBox", &json!({})).unwrap()["changed"], 1);
    assert_eq!(undo_len(&s), n + 1);
    assert_eq!(angle(&s, a), 0.0);
    // The art stays; the box is square to the page around it.
    let b = selection_box(&s);
    assert_eq!(b.angle, 0.0);
    assert_eq!(b.rect, geometry);
    // Nothing left to reset: no step.
    assert_eq!(s.execute("object.resetBoundingBox", &json!({})).unwrap()["changed"], 0);
    assert_eq!(undo_len(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(angle(&s, a), 45.0));
}

#[test]
fn page_flips_keep_an_upright_box_upright() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 50.0);
    sel(&mut s, &[a]);
    s.execute("object.reflect", &json!({"axis": "vertical"})).unwrap();
    s.execute("object.reflect", &json!({"axis": "horizontal"})).unwrap();
    assert_eq!(angle(&s, a), 0.0);
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    s.execute("object.reflect", &json!({"axis": "vertical"})).unwrap();
    assert!(near(angle(&s, a), -30.0));
}

#[test]
fn groups_type_and_multiple_selections_keep_their_angle() {
    let mut s = session();
    // Objects turned together share the box.
    let a = rect(&mut s, 100.0, 100.0, 100.0, 50.0);
    let b = rect(&mut s, 300.0, 100.0, 100.0, 50.0);
    sel(&mut s, &[a, b]);
    s.execute("object.rotate", &json!({"angle": 20})).unwrap();
    assert!(near(selection_box(&s).angle, 20.0) && near(angle(&s, a), 20.0) && near(angle(&s, b), 20.0));
    // Mixed angles: the box is square to the page.
    let c = rect(&mut s, 100.0, 300.0, 50.0, 50.0);
    sel(&mut s, &[a, c]);
    assert_eq!(selection_box(&s).angle, 0.0);
    // A group turns as one; its members keep their own angles inside it.
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    assert_eq!(selection_box(&s).angle, 0.0);
    s.execute("object.rotate", &json!({"angle": 30, "absolute": true})).unwrap();
    assert!(near(angle(&s, g), 30.0) && near(angle(&s, a), 50.0) && near(angle(&s, c), 30.0));
    s.execute("object.ungroup", &json!({})).unwrap();
    sel(&mut s, &[c]);
    let cb = selection_box(&s);
    assert!(near(cb.angle, 30.0) && near(cb.rect.width(), 50.0) && near(cb.rect.height(), 50.0), "{cb:?}");
    // Type keeps its angle too.
    let t = id_of(&s.execute("text.create", &json!({"x": 50, "y": 500, "text": "Turn"})).unwrap());
    sel(&mut s, &[t]);
    let before = selection_box(&s).rect;
    s.execute("object.rotate", &json!({"angle": 60, "absolute": true})).unwrap();
    let tb = selection_box(&s);
    assert!(near(tb.angle, 60.0), "{tb:?}");
    assert!(near(tb.rect.width(), before.width()) && near(tb.rect.height(), before.height()), "{tb:?} vs {before:?}");
}

#[test]
fn native_files_keep_the_angle() {
    let mut s = session();
    let plain = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let a = turned_rect(&mut s);
    let data = s.execute("document.save", &json!({"format": "vectorcraft"})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    let text = String::from_utf8(vectorcraft_format::base64_decode(&data).unwrap()).unwrap();
    // Only turned objects write it.
    assert_eq!(text.matches("bbox_angle").count(), 1);
    s.execute("document.open", &json!({"name": "turned.vectorcraft", "dataBase64": data})).unwrap();
    assert!(near(angle(&s, a), 45.0));
    assert_eq!(angle(&s, plain), 0.0);
    sel(&mut s, &[a]);
    assert!(near(selection_box(&s).rect.width(), 100.0));
}

#[test]
fn rotated_box_rect_matches_page_bounds_when_upright() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 20.0, 30.0, 40.0);
    sel(&mut s, &[a]);
    assert_eq!(selection_box(&s), OrientedBox::aligned(Rect::new(10.0, 20.0, 40.0, 60.0)));
}
