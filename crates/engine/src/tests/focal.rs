//! Radial gradients' extent and focal point (M3.98): `paint.setGradientGeom {aspect, focal}`,
//! focal points through transforms, params and the native format, and the radial annotator.

use serde_json::json;
use vectorcraft_color::{GradientGeom, Paint};
use vectorcraft_geom::Point;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use crate::*;

/// A 200 pt square at (100, 100) with a radial fill centred on it, radius 100.
fn radial() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"gradient": {"kind": "radial", "start": [200, 200], "end": [300, 200]}})).unwrap();
    (s, id)
}

fn geom(s: &Session, id: NodeId) -> GradientGeom {
    match s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint() {
        Paint::Gradient(g) => g.geom.expect("placed"),
        p => panic!("not a gradient: {p:?}"),
    }
}

fn close(a: Point, b: Point) -> bool {
    a.distance(b) < 1e-6
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn set_gradient_geom_sets_aspect_and_focal_in_one_step() {
    let (mut s, id) = radial();
    let n = undo_len(&s);
    s.execute("paint.setGradientGeom", &json!({"focal": [160, 210], "aspect": 50})).unwrap();
    let g = geom(&s, id);
    assert_eq!((g.start, g.end, g.aspect, g.focal), (Point::new(200.0, 200.0), Point::new(300.0, 200.0), 0.5, Some(Point::new(160.0, 210.0))));
    assert_eq!(undo_len(&s), n + 1);
    // Out of the extent ellipse it is pulled in; null centres it.
    s.execute("paint.setGradientGeom", &json!({"focal": [200, 400]})).unwrap();
    assert!(close(geom(&s, id).focal.unwrap(), Point::new(200.0, 249.5)), "{:?}", geom(&s, id));
    s.execute("paint.setGradientGeom", &json!({"focal": null})).unwrap();
    assert_eq!(geom(&s, id).focal, None);
    // One end without the other is an error; linear gradients have no focal point.
    assert!(s.execute("paint.setGradientGeom", &json!({"end": [0, 0]})).is_err());
    s.execute("paint.editGradient", &json!({"kind": "linear"})).unwrap();
    s.execute("paint.setGradientGeom", &json!({"start": [100, 200], "end": [300, 200], "focal": [150, 200]})).unwrap();
    assert_eq!(geom(&s, id).focal, None);
}

#[test]
fn the_focal_point_keeps_its_place_as_the_gradient_moves_and_reshapes() {
    let (mut s, id) = radial();
    s.execute("paint.setGradientGeom", &json!({"focal": [160, 210]})).unwrap();
    // A new vector: moved 10 right and turned to point down.
    s.execute("paint.setGradientGeom", &json!({"start": [210, 200], "end": [210, 300]})).unwrap();
    assert!(close(geom(&s, id).focal.unwrap(), Point::new(200.0, 160.0)), "{:?}", geom(&s, id));
    // The aspect ratio (Gradient panel) squashes it with the ellipse.
    s.execute("paint.editGradient", &json!({"aspect": 50})).unwrap();
    assert!(close(geom(&s, id).focal.unwrap(), Point::new(205.0, 160.0)), "{:?}", geom(&s, id));
    // Transforming the object maps it.
    s.execute("object.move", &json!({"dx": 5, "dy": 7})).unwrap();
    assert!(close(geom(&s, id).focal.unwrap(), Point::new(210.0, 167.0)), "{:?}", geom(&s, id));
    s.execute("object.transform", &json!({"matrix": [0, 1, -1, 0, 0, 0]})).unwrap();
    assert!(close(geom(&s, id).focal.unwrap(), Point::new(-167.0, 210.0)), "{:?}", geom(&s, id));
}

#[test]
fn the_focal_point_round_trips_through_params_and_the_native_format() {
    let (mut s, id) = radial();
    s.execute("paint.setGradientGeom", &json!({"focal": [170, 190], "aspect": 80})).unwrap();
    let want = geom(&s, id);
    // Gradient params (what swatches and agents read) carry it, and give it back.
    let Paint::Gradient(g) = s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint() else { panic!("a gradient fill") };
    let params = vectorcraft_tools::params::gradient_params(&g);
    assert_eq!(params["focal"], json!([170.0, 190.0]));
    s.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
    s.execute("paint.setFill", &json!({ "gradient": params })).unwrap();
    assert_eq!(geom(&s, id), want);
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let Paint::Gradient(g) = back.node(id).unwrap().appearance.fill_paint() else { panic!("gradient lost") };
    assert_eq!(g.geom, Some(want));
}

#[test]
fn the_radial_annotator_drags_the_focal_dot_the_aspect_handle_and_the_ring() {
    let (mut s, id) = radial();
    let v = ViewInfo::default();
    s.select_tool("gradient", v).unwrap();
    let drag = |s: &mut Session, pts: &[(f64, f64)]| {
        let n = undo_len(s);
        for (i, (x, y)) in pts.iter().enumerate() {
            let kind = [PointerKind::Down, PointerKind::Drag][i.min(1)];
            s.pointer(&PointerEvent::new(kind, *x, *y).with_mods(Mods::default()), v).unwrap();
        }
        let (x, y) = pts[pts.len() - 1];
        s.pointer(&PointerEvent::new(PointerKind::Up, x, y), v).unwrap();
        assert_eq!(undo_len(s), n + 1, "one undo step");
    };
    // The focal dot sits on the centre: dragging it moves only the focal point.
    drag(&mut s, &[(200.0, 200.0), (180.0, 195.0), (170.0, 190.0)]);
    let g = geom(&s, id);
    assert_eq!((g.start, g.focal), (Point::new(200.0, 200.0), Some(Point::new(170.0, 190.0))));
    // Back onto the centre, it centres.
    drag(&mut s, &[(170.0, 190.0), (190.0, 195.0), (200.5, 200.5)]);
    assert_eq!(geom(&s, id).focal, None);
    // The handle across the bar (100 pt above the centre) sets the aspect ratio.
    drag(&mut s, &[(200.0, 100.0), (200.0, 120.0), (200.0, 150.0)]);
    assert!((geom(&s, id).aspect - 0.5).abs() < 1e-9, "{:?}", geom(&s, id));
    // The ellipse elsewhere rotates it, keeping its radius: from its bottom round to its left
    // turns the vector a quarter clockwise, to point down.
    drag(&mut s, &[(200.0, 250.0), (170.0, 240.0), (150.0, 200.0)]);
    let g = geom(&s, id);
    assert!(close(g.end, Point::new(200.0, 300.0)) && (g.aspect - 0.5).abs() < 1e-9, "{g:?}");
}
