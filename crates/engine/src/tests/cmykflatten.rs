//! Flatten Transparency in CMYK documents composites inks, as the canvas shows them (M3.90).

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Color};
use vectorcraft_doc::{AppearanceItem, Node, NodeKind};
use vectorcraft_geom::{Point, Rect, Shape};
use vectorcraft_render::Renderer;

use crate::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

/// A CMYK document with an unstroked rectangle of inks `cmyk` (percentages) at (10, 10) and one
/// at (60, 60), 100 pt square.
fn session(below: [f64; 4], above: [f64; 4]) -> (Session, u64, u64) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 200, "height": 200, "colorMode": "cmyk"}));
    let mut rect = |x: f64, [c, m, y, k]: [f64; 4]| {
        let id = run(&mut s, "shape.rectangle", json!({"x": x, "y": x, "width": 100, "height": 100}))["id"].as_u64().unwrap();
        run(&mut s, "paint.setFill", json!({"color": {"c": c, "m": m, "y": y, "k": k}, "ids": [id]}));
        run(&mut s, "paint.setStroke", json!({"none": true, "ids": [id]}));
        id
    };
    let (a, b) = (rect(10.0, below), rect(60.0, above));
    (s, a, b)
}

/// The fill colour of the flattened region at `p` in group `group`.
fn region_color(s: &Session, group: u64, p: Point) -> Color {
    let doc = &s.doc().unwrap().doc;
    let g = doc.node(NodeId(group)).unwrap();
    let inside = |n: &Node| matches!(&n.kind, NodeKind::Path { path, .. } if path.to_bezpath().winding(p) != 0);
    let r = g.children().unwrap().iter().find(|c| inside(c)).expect("a region there");
    let AppearanceItem::Fill(f) = &r.appearance.items[0] else { panic!("a region has one fill") };
    f.paint.color().unwrap()
}

fn picture(s: &Session) -> vectorcraft_render::Rendered {
    Renderer::new().render_region(&s.doc().unwrap().doc, Rect::new(0.0, 0.0, 200.0, 200.0), 1.0, true)
}

#[test]
fn cmyk_documents_flatten_to_ink_colours() {
    let _settings = crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut s, a, b) = session([0.0, 100.0, 0.0, 0.0], [100.0, 0.0, 0.0, 0.0]);
    run(&mut s, "transparency.set", json!({"ids": [b], "blend": "Multiply"}));
    let ids = run(&mut s, "object.flattenTransparency", json!({"ids": [a, b]}))["ids"].as_array().unwrap()[0].as_u64().unwrap();
    // Multiply prints both inks where they overlap.
    let near = |c: Color, want: [f32; 4]| matches!(c, Color::Cmyk { c, m, y, k } if [c, m, y, k].iter().zip(want).all(|(x, w)| (x - w).abs() < 1e-3));
    let overlap = region_color(&s, ids, Point::new(85.0, 85.0));
    assert!(near(overlap, [1.0, 1.0, 0.0, 0.0]), "{overlap:?}");
    assert!(near(region_color(&s, ids, Point::new(130.0, 130.0)), [1.0, 0.0, 0.0, 0.0]));
}

#[test]
fn every_blend_mode_flattens_to_the_canvas_pixels_in_cmyk() {
    let _settings = crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    for mode in BlendMode::ALL {
        let (mut s, a, b) = session([20.0, 60.0, 10.0, 30.0], [70.0, 10.0, 50.0, 10.0]);
        run(&mut s, "transparency.set", json!({"ids": [b], "blend": mode.label(), "opacity": 70}));
        let before = picture(&s);
        let ids = run(&mut s, "object.flattenTransparency", json!({"ids": [a, b]}))["ids"].as_array().unwrap()[0].as_u64().unwrap();
        for p in [Point::new(30.5, 30.5), Point::new(85.5, 85.5), Point::new(140.5, 140.5)] {
            let c = region_color(&s, ids, p);
            assert!(matches!(c, Color::Cmyk { .. }), "{mode:?}: {c:?}");
            let got = c.to_rgba8(1.0);
            let want = before.pixel(p.x as u32, p.y as u32);
            // The canvas rounds each ink plane to 8 bits and interpolates the profile.
            assert!((0..3).all(|i| got[i].abs_diff(want[i]) <= 4), "{mode:?} at {p:?}: region {got:?}, canvas {want:?}");
        }
    }
}
