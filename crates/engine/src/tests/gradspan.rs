//! `paint.setFill` / `paint.setStroke` `span: "selection"`: one gradient placed on the union of
//! the targets' bounds, so the ramp runs across them (#2127).

use serde_json::{Value, json};
use vectorcraft_color::{GradientPaint, Paint};
use vectorcraft_geom::{Affine, Point};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn fill_gradient(s: &Session, id: NodeId) -> GradientPaint {
    match s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint() {
        Paint::Gradient(g) => *g,
        p => panic!("expected a gradient fill, got {p:?}"),
    }
}

fn stroke_gradient(s: &Session, id: NodeId) -> GradientPaint {
    match &s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().unwrap().paint {
        Paint::Gradient(g) => (**g).clone(),
        p => panic!("expected a gradient stroke, got {p:?}"),
    }
}

fn near(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-6
}

fn black_to_white() -> Value {
    json!({"kind": "linear", "stops": [{"offset": 0, "color": "#000000"}, {"offset": 1, "color": "#ffffff"}]})
}

/// The red channel of the rendered document at (x, y).
fn red_at(s: &Session, x: u32, y: u32) -> u8 {
    let doc = s.doc().unwrap().doc.clone();
    let img = vectorcraft_render::Renderer::new().render(&doc, 400, 200, Affine::IDENTITY, &Default::default());
    img.pixel(x, y)[0]
}

#[test]
fn a_linear_gradient_spans_the_union_of_the_targets() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 50.0, 100.0, 100.0);
    let b = rect(&mut s, 100.0, 0.0, 100.0, 200.0);
    let c = rect(&mut s, 300.0, 50.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"ids": [a.0, b.0, c.0], "gradient": black_to_white(), "span": "selection"})).unwrap();
    // One vector for all three, across the union 0..400 × 0..200 at angle 0.
    for id in [a, b, c] {
        let g = fill_gradient(&s, id);
        let geom = g.geom.expect("placed in document coordinates");
        assert!(near(geom.start, Point::new(0.0, 100.0)) && near(geom.end, Point::new(400.0, 100.0)), "{id:?}: {geom:?}");
    }
    // The ramp runs on across the objects: dark on the left one, light on the right one, and the
    // edge the first two share is the same colour on both sides.
    let (left, right) = (red_at(&s, 5, 100), red_at(&s, 395, 100));
    assert!(left < 20 && right > 235, "{left} {right}");
    let (edge_a, edge_b) = (i32::from(red_at(&s, 98, 100)), i32::from(red_at(&s, 102, 100)));
    assert!((edge_a - edge_b).abs() <= 4 && (50..80).contains(&edge_a), "{edge_a} {edge_b}");
}

#[test]
fn without_span_each_object_fits_its_own_box() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 50.0, 100.0, 100.0);
    let b = rect(&mut s, 100.0, 50.0, 100.0, 100.0);
    for span in [json!(null), json!("object")] {
        let mut p = json!({"ids": [a.0, b.0], "gradient": black_to_white()});
        if !span.is_null() {
            p["span"] = span;
        }
        s.execute("paint.setFill", &p).unwrap();
        assert_eq!(fill_gradient(&s, a).geom, None);
        // Each ramp starts dark again on its own object.
        assert!(red_at(&s, 102, 100) < 20, "{p}");
    }
}

#[test]
fn the_selection_is_the_default_target_and_angle_and_radial_are_kept() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 200.0, 100.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let mut g = black_to_white();
    g["angle"] = json!(-90);
    s.execute("paint.setFill", &json!({"gradient": g, "span": "selection"})).unwrap();
    // Vertical, top to bottom, over the union 0..300 × 0..200.
    let geom = fill_gradient(&s, b).geom.unwrap();
    assert!(near(geom.start, Point::new(150.0, 0.0)) && near(geom.end, Point::new(150.0, 200.0)), "{geom:?}");
    assert_eq!(fill_gradient(&s, a).geom, Some(geom));
    // Radial: centred on the union, radius half its larger side, aspect kept.
    s.execute("paint.setFill", &json!({"gradient": {"kind": "radial", "aspect": 50, "stops": black_to_white()["stops"]}, "span": "selection"}))
        .unwrap();
    let geom = fill_gradient(&s, a).geom.unwrap();
    assert!(near(geom.start, Point::new(150.0, 100.0)), "{geom:?}");
    assert!(((geom.end - geom.start).hypot() - 150.0).abs() < 1e-6, "{geom:?}");
    assert!((geom.aspect - 0.5).abs() < 1e-9, "{geom:?}");
    assert_eq!(fill_gradient(&s, b).geom, Some(geom));
}

#[test]
fn strokes_span_the_targets_too() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 50.0, 100.0, 100.0);
    let b = rect(&mut s, 300.0, 50.0, 100.0, 100.0);
    s.execute("paint.setStroke", &json!({"ids": [a.0, b.0], "gradient": black_to_white(), "span": "selection"})).unwrap();
    let (ga, gb) = (stroke_gradient(&s, a).geom.unwrap(), stroke_gradient(&s, b).geom.unwrap());
    assert_eq!(ga, gb);
    assert!(ga.start.x < 0.0 && ga.end.x > 400.0, "the stroke's box, half its weight out: {ga:?}");
}

#[test]
fn the_new_art_default_stays_unplaced() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"ids": [a.0], "gradient": black_to_white(), "span": "selection"})).unwrap();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    let n = rect(&mut s, 200.0, 0.0, 50.0, 50.0);
    assert_eq!(fill_gradient(&s, n).geom, None, "the next object fits the gradient to itself");
}

#[test]
fn bad_spans_are_refused_and_change_nothing() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 200.0, 0.0, 100.0, 100.0);
    let before = s.doc().unwrap().doc.clone();
    let mut with_vector = black_to_white();
    with_vector["start"] = json!([0, 0]);
    with_vector["end"] = json!([10, 0]);
    for bad in [
        json!({"ids": [a.0, b.0], "gradient": black_to_white(), "span": "page"}),
        json!({"ids": [a.0, b.0], "gradient": black_to_white(), "span": true}),
        json!({"ids": [a.0, b.0], "color": "#ff0000", "span": "selection"}),
        json!({"ids": [a.0, b.0], "gradient": {"kind": "freeform"}, "span": "selection"}),
        json!({"ids": [a.0, b.0], "gradient": with_vector, "span": "selection"}),
    ] {
        let e = s.execute("paint.setFill", &bad).unwrap_err().to_string();
        assert!(e.contains("span"), "{bad}: {e}");
        assert_eq!(s.doc().unwrap().doc, before, "{bad}");
    }
}
