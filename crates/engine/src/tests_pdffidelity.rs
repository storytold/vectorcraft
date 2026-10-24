//! PDF vector fidelity: pattern fills and strokes are written as tiling patterns (strokes clipped
//! to the stroke's outline), freeform gradients as images of their colour field clipped to what they paint, and
//! both read back looking like the canvas (within 2% at 150 ppi), with nothing to report.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, Document, Node};
use vectorcraft_geom::Rect;
use vectorcraft_render::{Rendered, Renderer};

use super::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn node(s: &Session, id: &Value) -> Node {
    s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().clone()
}

/// `s`'s document with object `id`'s fills and strokes repainted.
fn repainted(s: &Session, id: &Value, fill: Option<Paint>, stroke: Option<Paint>) -> Document {
    let mut d = (*s.doc().unwrap().doc).clone();
    for item in &mut d.node_mut(NodeId(id.as_u64().unwrap())).unwrap().appearance.items {
        match (item, &fill, &stroke) {
            (AppearanceItem::Fill(f), Some(p), _) => f.paint = p.clone(),
            (AppearanceItem::Stroke(st), _, Some(p)) => st.paint = p.clone(),
            _ => {}
        }
    }
    d
}

fn render(d: &Document) -> Rendered {
    Renderer::new().render_region(d, Rect::new(0.0, 0.0, 200.0, 200.0), 150.0 / 72.0, true)
}

/// Mean difference of the colour channels of two renders, as a share of full scale.
fn mean_diff(a: &Rendered, b: &Rendered) -> f64 {
    let sum: u64 = a.pixels.iter().zip(&b.pixels).map(|(x, y)| x.abs_diff(*y) as u64).sum();
    sum as f64 / a.pixels.len() as f64 / 255.0
}

/// The PDF of `s`'s document: no warnings, an embedded image when `image`, and read back within 2%
/// of the canvas and much closer than `rough` (the art painted the old way).
fn assert_faithful(what: &str, s: &Session, image: bool, rough: &Document) {
    let d = &s.doc().unwrap().doc;
    let r = crate::cmd::rasterfx::export_pdf_with_report(d, &Default::default()).unwrap();
    assert!(r.warnings.is_empty(), "{what}: {:?}", r.warnings);
    assert_eq!(String::from_utf8_lossy(&r.bytes).contains("/Subtype/Image"), image, "{what}: image XObject");
    let canvas = render(d);
    let diff = mean_diff(&canvas, &render(&vectorcraft_pdf::import(&r.bytes).unwrap()));
    let before = mean_diff(&canvas, &render(rough));
    assert!(diff < 0.02 && diff < before / 4.0, "{what}: read back {:.2}% off (the old way {:.2}%)", diff * 100.0, before * 100.0);
}

#[test]
fn pattern_strokes_are_tiling_patterns_clipped_to_the_stroke() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let dot = run(&mut s, "shape.ellipse", json!({"x": 0, "y": 0, "width": 6, "height": 6}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [dot]}));
    run(&mut s, "paint.setFill", json!({"color": "#d02020"}));
    run(&mut s, "object.pattern.make", json!({"name": "Dots", "width": 8, "height": 8}));
    run(&mut s, "object.pattern.done", json!({}));
    let frame = run(&mut s, "shape.rectangle", json!({"x": 30, "y": 30, "width": 140, "height": 120}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [frame]}));
    run(&mut s, "paint.setFill", json!({"color": "#2040c0"}));
    run(&mut s, "stroke.set", json!({"weight": 16, "align": "outside", "dash": [30, 10]}));
    run(&mut s, "paint.setStroke", json!({"swatch": "Dots"}));
    // The old way: a mid-grey stroke.
    let grey = Paint::solid(Color::rgb(0.5, 0.5, 0.5));
    assert_faithful("pattern stroke", &s, false, &repainted(&s, &frame, None, Some(grey)));
}

/// A brick tiling whose art reaches over the next tiles repeats as one period of the tiling (#3849).
#[test]
fn brick_tilings_with_overlapping_tiles_are_tiling_patterns() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let dot = run(&mut s, "shape.ellipse", json!({"x": 0, "y": 0, "width": 12, "height": 12}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [dot]}));
    run(&mut s, "paint.setFill", json!({"color": "#d02020"}));
    run(&mut s, "object.pattern.make", json!({"name": "Bricks", "tileType": "brickByRow", "width": 10, "height": 10}));
    run(&mut s, "object.pattern.done", json!({}));
    let frame = run(&mut s, "shape.rectangle", json!({"x": 30, "y": 30, "width": 140, "height": 120}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [frame]}));
    run(&mut s, "paint.setFill", json!({"swatch": "Bricks"}));
    let grey = Paint::solid(Color::rgb(0.5, 0.5, 0.5));
    assert_faithful("brick fill", &s, false, &repainted(&s, &frame, Some(grey), None));
}

#[test]
fn freeform_gradients_are_images_of_their_colour_field() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let id = run(&mut s, "shape.ellipse", json!({"x": 20, "y": 30, "width": 160, "height": 130}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [id]}));
    run(&mut s, "stroke.set", json!({"weight": 14}));
    run(&mut s, "paint.editGradient", json!({"kind": "freeform"}));
    run(&mut s, "paint.editGradient", json!({"kind": "freeform", "stroke": true}));
    // The old way: the average colour.
    let Paint::Gradient(g) = node(&s, &id).appearance.fill_paint() else { panic!("a freeform fill") };
    let n = g.gradient.stops.len() as f32;
    let c = g.gradient.stops.iter().fold([0.0f32; 3], |acc, st| {
        let c = st.color.to_rgb();
        [acc[0] + c[0] / n, acc[1] + c[1] / n, acc[2] + c[2] / n]
    });
    let avg = Paint::solid(Color::rgb(c[0], c[1], c[2]));
    let rough = repainted(&s, &id, Some(avg.clone()), Some(avg));
    assert_faithful("freeform", &s, true, &rough);
}
