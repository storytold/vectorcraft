//! Gradients on type: a vector across the text, and run strokes fitted like object strokes.

use serde_json::json;
use vectorcraft_color::Paint;
use vectorcraft_doc::{Document, NodeId, NodeKind, TextObject, appearance::stroke_paint_bounds};
use vectorcraft_geom::Rect;
use vectorcraft_testkit::fixtures::{exec, session_with};
use vectorcraft_testkit::raster::{Image, render_view};

/// Mean red and blue of the solid ink (no green: the ramp has none, the paper and anti-aliased
/// edges do) in columns `x0..x1`.
fn ink_tint(img: &Image, x0: u32, x1: u32) -> (f64, f64) {
    let px: Vec<[u8; 4]> =
        (x0..x1).flat_map(|x| (0..img.height).map(move |y| (x, y))).map(|(x, y)| img.pixel(x, y)).filter(|p| p[3] == 255 && p[1] < 40).collect();
    assert!(px.len() > 20, "ink in {x0}..{x1}: {}", px.len());
    let mean = |c: usize| px.iter().map(|p| p[c] as f64).sum::<f64>() / px.len() as f64;
    (mean(0), mean(2))
}

#[test]
fn gradient_vector_runs_across_the_text() {
    let mut s = session_with(300.0, 100.0);
    exec(&mut s, "text.create", json!({"x": 10, "y": 80, "text": "HHHHHH", "size": 60}));
    exec(&mut s, "paint.setFill", json!({"gradient": {"stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]}}));
    exec(&mut s, "paint.setGradientGeom", json!({"start": [10, 50], "end": [200, 50]}));
    let img = render_view(&s.doc().unwrap().doc, 300, 100);
    let (r0, b0) = ink_tint(&img, 10, 50);
    let (r1, b1) = ink_tint(&img, 150, 200);
    assert!(r0 > 180.0 && b0 < 75.0, "red where the vector starts: {r0} {b0}");
    assert!(b1 > 160.0 && r1 < 95.0, "blue where it ends: {r1} {b1}");
}

fn text_mut(doc: &mut Document, id: NodeId) -> &mut TextObject {
    match &mut doc.node_mut(id).unwrap().kind {
        NodeKind::Text(t) => t,
        k => panic!("not text: {k:?}"),
    }
}

/// `doc` with every run's stroke gradient pinned to its fit on `b`.
fn pinned(doc: &Document, id: NodeId, b: Rect) -> Document {
    let mut d = doc.clone();
    for r in &mut text_mut(&mut d, id).runs {
        if let Paint::Gradient(g) = &mut r.style.stroke {
            g.pin(b);
        }
    }
    d
}

#[test]
fn unplaced_run_stroke_gradients_fit_the_stroke_inflated_box() {
    let mut s = session_with(300.0, 100.0);
    exec(&mut s, "text.create", json!({"x": 10, "y": 80, "text": "HHHH", "size": 60}));
    exec(&mut s, "paint.setFill", json!({"none": true}));
    exec(&mut s, "paint.setStroke", json!({"gradient": {"stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]}}));
    let id = s.doc().unwrap().selection.objects[0];
    let mut doc: Document = (*s.doc().unwrap().doc).clone();
    let t = text_mut(&mut doc, id);
    t.runs.iter_mut().for_each(|r| r.style.stroke_width = 8.0);
    let lb = t.local_bounds();
    let unplaced = render_view(&doc, 300, 100);
    // Pinned to the fit on the layout box grown by half the 8 pt weight, it draws the same pixels;
    // the bare layout box would not.
    assert_eq!(render_view(&pinned(&doc, id, stroke_paint_bounds(lb, 8.0)), 300, 100), unplaced);
    assert_ne!(render_view(&pinned(&doc, id, lb), 300, 100), unplaced);
}
