//! Gradients within, along or across a stroke: `paint.editGradient {strokeMode}`, the saved mode,
//! and the same picture on the canvas, after Outline Stroke and Expand, and in SVG and PDF.

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{Document, NodeKind, StrokeGradientMode};
use vectorcraft_geom::Rect;
use vectorcraft_render::{Rendered, Renderer};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn doc(s: &Session) -> &Document {
    &s.doc().unwrap().doc
}

/// A selected open path (an L: right, then down) with a 20 pt white → black stroke laid `mode`.
fn setup(mode: &str) -> (Session, NodeId) {
    let mut s = session();
    let anchors = json!([{"x": 20, "y": 40}, {"x": 160, "y": 40}, {"x": 160, "y": 180}]);
    let id = run(&mut s, "path.create", json!({"anchors": anchors}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [id]}));
    run(&mut s, "paint.setFill", json!({"none": true}));
    run(&mut s, "stroke.set", json!({"weight": 20}));
    run(&mut s, "paint.editGradient", json!({"stroke": true, "strokeMode": mode}));
    (s, NodeId(id))
}

fn mode_of(s: &Session, id: NodeId) -> StrokeGradientMode {
    doc(s).node(id).unwrap().appearance.stroke().unwrap().gradient_mode
}

fn render(d: &Document) -> Rendered {
    Renderer::new().render_region(d, Rect::new(0.0, 0.0, 200.0, 200.0), 1.0, true)
}

fn grey(img: &Rendered, x: u32, y: u32) -> f64 {
    img.pixel(x, y)[0] as f64 / 255.0
}

/// Points inside the stroke (away from its edges) where two pictures are compared.
const PROBES: [(u32, u32); 8] = [(30, 40), (60, 35), (100, 44), (150, 40), (163, 37), (160, 80), (156, 130), (164, 170)];

/// `b` paints the stroke as `a` does, within `tol` at every probe.
fn assert_same(a: &Rendered, b: &Rendered, tol: f64, what: &str) {
    for (x, y) in PROBES {
        let (ga, gb) = (grey(a, x, y), grey(b, x, y));
        assert!((ga - gb).abs() <= tol, "{what} at ({x}, {y}): {gb} vs {ga}");
    }
}

#[test]
fn stroke_mode_sets_how_the_gradient_lies_with_one_undo_step() {
    let (mut s, id) = setup("along");
    assert_eq!(mode_of(&s, id), StrokeGradientMode::Along);
    assert!(matches!(doc(&s).node(id).unwrap().appearance.stroke_paint(), Paint::Gradient(_)), "a solid stroke became a gradient");
    let o = &run(&mut s, "document.inspect", json!({}))["layers"][0]["children"][0]["strokeOptions"];
    assert_eq!(o["gradientMode"], "along");
    run(&mut s, "paint.editGradient", json!({"stroke": true, "strokeMode": "across"}));
    assert_eq!(mode_of(&s, id), StrokeGradientMode::Across);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(mode_of(&s, id), StrokeGradientMode::Along);
    // Bad values and fills are refused without touching the document.
    assert!(s.execute("paint.editGradient", &json!({"stroke": true, "strokeMode": "sideways"})).is_err());
    assert!(s.execute("paint.editGradient", &json!({"stroke": false, "strokeMode": "along"})).is_err());
    assert_eq!(mode_of(&s, id), StrokeGradientMode::Along);
    // With nothing selected it sets up the next object drawn.
    run(&mut s, "select.set", json!({"ids": []}));
    run(&mut s, "paint.editGradient", json!({"stroke": true, "strokeMode": "across"}));
    let r = run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 30}))["id"].as_u64().unwrap();
    assert_eq!(mode_of(&s, NodeId(r)), StrokeGradientMode::Across);
}

#[test]
fn the_mode_round_trips_through_the_native_format_and_old_files_paint_within() {
    let (s, id) = setup("across");
    let back = vectorcraft_format::load(&vectorcraft_format::save_file(doc(&s))).unwrap();
    assert_eq!(back.node(id).unwrap().appearance.stroke().unwrap().gradient_mode, StrokeGradientMode::Across);
    let (s, id) = setup("within");
    let json = serde_json::to_string(doc(&s).node(id).unwrap().appearance.stroke().unwrap()).unwrap();
    assert!(!json.contains("gradient_mode"), "the default is not written: {json}");
    let old: vectorcraft_doc::StrokeLayer = serde_json::from_str(r#"{"paint":{"type":"none"},"width":2.0}"#).unwrap();
    assert_eq!(old.gradient_mode, StrokeGradientMode::Within);
}

#[test]
fn along_and_across_paint_the_canvas_by_position_on_the_path() {
    // The path is 280 long: (90, 40) is a quarter of the way, (160, 110) three quarters.
    let (s, _) = setup("along");
    let img = render(doc(&s));
    for (x, y, t) in [(90, 40, 0.25), (90, 47, 0.25), (160, 110, 0.75), (154, 110, 0.75), (166, 34, 0.5)] {
        assert!((grey(&img, x, y) - (1.0 - t)).abs() < 0.03, "along ({x}, {y}): {}", grey(&img, x, y));
    }
    // Across: travelling right the left edge is the top one; going down, the right-hand one.
    let (s, _) = setup("across");
    let img = render(doc(&s));
    assert!(grey(&img, 90, 32) > 0.8 && grey(&img, 90, 48) < 0.2);
    assert!(grey(&img, 168, 120) > 0.8 && grey(&img, 152, 120) < 0.2);
}

#[test]
fn outline_stroke_and_expand_keep_the_picture() {
    for mode in ["along", "across"] {
        let (mut s, _) = setup(mode);
        let before = render(doc(&s));
        let ids = run(&mut s, "object.path.outlineStroke", json!({}))["ids"].clone();
        let n = doc(&s).node(NodeId(ids[0].as_u64().unwrap())).unwrap().clone();
        let NodeKind::Group { children, clip: true } = &n.kind else { panic!("a clip group: {:?}", n.kind_label()) };
        assert!(children[1..].iter().all(|c| matches!(c.kind, NodeKind::Mesh(_))), "gradient meshes");
        assert_same(&before, &render(doc(&s)), 0.06, &format!("{mode} outlined"));
        run(&mut s, "edit.undo", json!({}));
        run(&mut s, "object.expand", json!({}));
        assert_same(&before, &render(doc(&s)), 0.06, &format!("{mode} expanded"));
    }
}

#[test]
fn outline_stroke_keeps_the_effect_of_a_stroke_with_a_gradient_along_it() {
    let (mut s, _) = setup("along");
    let stroke = doc(&s).node(s.doc().unwrap().selection.objects[0]).unwrap().appearance.items.len() - 1;
    run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "item": stroke}));
    let ids = run(&mut s, "object.path.outlineStroke", json!({}))["ids"].clone();
    let n = doc(&s).node(NodeId(ids[0].as_u64().unwrap())).unwrap().clone();
    let mut shadowed = false;
    n.walk(&mut |c| shadowed |= matches!(c.kind, NodeKind::Group { clip: true, .. }) && !c.appearance.effects.is_empty());
    assert!(shadowed, "the stroke's drop shadow is on its outline's clip group");
}

#[test]
fn svg_and_pdf_write_the_same_gradient_as_slices_with_a_warning() {
    for mode in ["along", "across"] {
        let (s, _) = setup(mode);
        let before = render(doc(&s));
        let (svg, warnings) = vectorcraft_svg::export_with_report(doc(&s), &Default::default());
        assert!(warnings.iter().any(|w| w.contains("along or across strokes")), "{warnings:?}");
        assert!(svg.contains("<clipPath") && svg.matches("<linearGradient").count() > 3, "{mode}: sliced");
        let back = vectorcraft_svg::import(&svg).unwrap();
        assert_same(&before, &render(&back), 0.04, &format!("{mode} SVG"));
        let opts = vectorcraft_pdf::PdfOptions::uncompressed();
        let r = vectorcraft_pdf::export_with_report(doc(&s), &opts).unwrap();
        assert!(r.warnings.iter().any(|w| w.contains("along or across strokes")), "{:?}", r.warnings);
        assert!(String::from_utf8_lossy(&r.bytes).matches("/ShadingType 2").count() > 3, "{mode}: axial shadings");
        let back = vectorcraft_pdf::import(&r.bytes).unwrap();
        assert_same(&before, &render(&back), 0.04, &format!("{mode} PDF"));
    }
    // Within, a stroke is still written as one plain stroke.
    let (s, _) = setup("within");
    let (svg, warnings) = vectorcraft_svg::export_with_report(doc(&s), &Default::default());
    assert!(warnings.is_empty() && !svg.contains("<clipPath"));
}
