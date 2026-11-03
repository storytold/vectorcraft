//! Fill and stroke on a clipping path (M3.89): Make Clipping Mask leaves the clipping path
//! unpainted, paint given to it later shows (its fill behind the clipped art, its stroke over it
//! and unclipped) on the canvas, in SVG and in PDF, and Release and Ungroup keep it.

use serde_json::json;
use vectorcraft_geom::Affine;
use vectorcraft_render::{RenderOptions, Renderer};

use crate::*;

fn rect(s: &mut Session, x: f64, w: f64, fill: &str) -> NodeId {
    let id = NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 50, "width": w, "height": 100})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"color": fill, "ids": [id.0]})).unwrap();
    id
}

/// Red art (40..100) clipped by a rectangle (50..150) painted blue with a 10 pt green stroke.
fn painted_clip() -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    rect(&mut s, 40.0, 60.0, "#ff0000");
    let clip = rect(&mut s, 50.0, 100.0, "#ffffff");
    s.execute("select.all", &json!({})).unwrap();
    let group = NodeId(s.execute("object.clippingMask.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    let n = s.doc().unwrap().doc.node(clip).unwrap().clone();
    assert!(n.clip_paint().fill.is_none() && n.clip_paint().stroke.is_none(), "Make leaves the clipping path unpainted");
    s.execute("paint.setFill", &json!({"color": "#0000ff", "ids": [clip.0]})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#00ff00", "ids": [clip.0]})).unwrap();
    s.execute("stroke.set", &json!({"weight": 10, "ids": [clip.0]})).unwrap();
    (s, group, clip)
}

/// RGB at (x, 100) of `doc` rendered on white.
fn rgb(doc: &Document, x: u32) -> [u8; 3] {
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let p = Renderer::new().render(doc, 200, 200, Affine::IDENTITY, &opts).pixel(x, 100);
    [p[0], p[1], p[2]]
}

/// Behind the clipped art, its fill; over it (also outside the clip), its stroke.
fn assert_painted(doc: &Document, what: &str) {
    assert_eq!(rgb(doc, 70), [255, 0, 0], "{what}: the clipped art over the fill");
    assert_eq!(rgb(doc, 120), [0, 0, 255], "{what}: the fill inside the clip");
    assert_eq!(rgb(doc, 45), [0, 255, 0], "{what}: the stroke's outer half, unclipped (the art below it is clipped away)");
    assert_eq!(rgb(doc, 52), [0, 255, 0], "{what}: the stroke over the clipped art");
    assert_eq!(rgb(doc, 147), [0, 255, 0], "{what}: the stroke over the fill");
    assert_eq!(rgb(doc, 30), [255, 255, 255], "{what}: outside the clip and its stroke");
}

#[test]
fn a_clipping_paths_fill_and_stroke_show_in_render_svg_and_pdf() {
    let (s, group, clip) = painted_clip();
    let doc = s.doc().unwrap().doc.clone();
    assert_painted(&doc, "canvas");
    // The clip group's bounds take in the stroke.
    let b = doc.node(group).unwrap().visual_bounds().unwrap();
    assert!((b.x0 - 45.0).abs() < 1e-6 && (b.x1 - 155.0).abs() < 1e-6, "{b:?}");
    // SVG: the fill inside the clip, the stroke after it, one element keeping the clipping path's id.
    let svg = vectorcraft_svg::export(&doc, &Default::default());
    assert_painted(&vectorcraft_svg::import(&svg).unwrap(), "SVG");
    // PDF.
    let pdf = vectorcraft_pdf::export(&doc, &Default::default()).unwrap();
    assert_painted(&vectorcraft_pdf::import(&pdf).unwrap(), "PDF");
    // Unpainted clipping paths write nothing extra.
    let mut bare = s;
    bare.execute("paint.setFill", &json!({"none": true, "ids": [clip.0]})).unwrap();
    bare.execute("paint.setStroke", &json!({"none": true, "ids": [clip.0]})).unwrap();
    let doc = bare.doc().unwrap().doc.clone();
    assert_eq!(rgb(&doc, 120), [255, 255, 255]);
    assert_eq!(rgb(&doc, 45), [255, 255, 255]);
    let svg = vectorcraft_svg::export(&doc, &Default::default());
    assert_eq!(vectorcraft_testkit::svg_shapes(&svg), 2, "the clip region and the red art: {svg}");
}

#[test]
fn release_and_ungroup_keep_the_clipping_paths_paint() {
    for cmd in ["object.clippingMask.release", "object.ungroup"] {
        let (mut s, group, clip) = painted_clip();
        s.execute("select.set", &json!({"ids": [group.0]})).unwrap();
        s.execute(cmd, &json!({})).unwrap();
        let n = s.doc().unwrap().doc.node(clip).unwrap().clone();
        assert_eq!(n.appearance.fill_paint(), vectorcraft_color::Paint::solid(vectorcraft_color::Color::from_hex("#0000ff").unwrap()), "{cmd}");
        assert_eq!(n.appearance.stroke().map(|st| st.width), Some(10.0), "{cmd}");
    }
}

#[test]
fn type_as_a_clipping_path_paints_its_characters() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    rect(&mut s, 0.0, 200.0, "#ff0000");
    let t = NodeId(s.execute("text.create", &json!({"x": 10, "y": 120, "text": "Clip", "size": 72})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.all", &json!({})).unwrap();
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    let paint = |s: &Session| s.doc().unwrap().doc.node(t).unwrap().clip_paint();
    assert!(paint(&s).fill.is_none() && paint(&s).stroke.is_none());
    s.execute("paint.setFill", &json!({"color": "#0000ff", "ids": [t.0]})).unwrap();
    let p = paint(&s);
    assert!(p.stroke.is_none());
    let NodeKind::Text(fill) = &p.fill.expect("the characters' fill paints").kind else { panic!("type") };
    assert!(fill.runs.iter().all(|r| r.style.fill.color().is_some() && r.style.stroke.is_none()));
}
