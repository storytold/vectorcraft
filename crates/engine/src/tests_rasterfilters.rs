//! The Photoshop-style raster effects (Effect › Blur › Radial Blur and Smart Blur, Brush Strokes,
//! Distort › Diffuse Glow, Glass and Ocean Ripple, Pixelate › Color Halftone, Crystallize, Mezzotint and Pointillize, Sharpen › Unsharp Mask, Texture ›
//! Craquelure, Grain, Mosaic Tiles, Patchwork, Stained Glass and Texturizer, Video): applied and
//! edited as commands, listed in the catalogue, drawn on the canvas, and written to PDF and SVG
//! as images of the effected object.

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_geom::Rect;
use vectorcraft_render::{Rendered, Renderer};

use super::*;

/// The new effects with parameters that change a striped square visibly.
const EFFECTS: [(&str, &str); 3] = [
    ("blur.radial", r#"{"amount": 30, "quality": "draft"}"#),
    ("blur.smart", r#"{"radius": 8, "threshold": 100}"#),
    ("sharpen.unsharpMask", r#"{"amount": 300, "radius": 3}"#),
];

/// Effect › Brush Strokes, at their defaults.
const BRUSH_STROKES: [(&str, &str); 8] = [
    ("brushStrokes.accentedEdges", "{}"),
    ("brushStrokes.angledStrokes", "{}"),
    ("brushStrokes.crosshatch", "{}"),
    ("brushStrokes.darkStrokes", "{}"),
    ("brushStrokes.inkOutlines", "{}"),
    ("brushStrokes.spatter", "{}"),
    ("brushStrokes.sprayedStrokes", "{}"),
    ("brushStrokes.sumiE", "{}"),
];

/// Effect › Distort (Diffuse Glow with its highlights glowing from a low brightness up, as the
/// striped art's reds are dark).
const DISTORT: [(&str, &str); 3] =
    [("distort.diffuseGlow", r#"{"clearAmount": 0, "glowAmount": 20}"#), ("distort.glass", "{}"), ("distort.oceanRipple", "{}")];

/// Effect › Pixelate, at their defaults.
const PIXELATE: [(&str, &str); 4] =
    [("pixelate.colorHalftone", "{}"), ("pixelate.crystallize", "{}"), ("pixelate.mezzotint", "{}"), ("pixelate.pointillize", "{}")];

/// Effect › Texture, at their defaults.
const TEXTURE: [(&str, &str); 6] = [
    ("texture.craquelure", "{}"),
    ("texture.grain", "{}"),
    ("texture.mosaicTiles", "{}"),
    ("texture.patchwork", "{}"),
    ("texture.stainedGlass", "{}"),
    ("texture.texturizer", "{}"),
];

/// Effect › Video (NTSC Colors leaves the striped art's legal colours as they are: see
/// `ntsc_colors_draws_saturated_yellow_safer`).
const VIDEO: [(&str, &str); 1] = [("video.deinterlace", r#"{"create": "interpolation"}"#)];

/// A group of a red square and a darker red stripe across it; returns the group's id.
fn striped() -> (Session, u64) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setFill", &json!({"color": "#e03020", "ids": [id]})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true, "ids": [id]})).unwrap();
    let stripe = s.execute("shape.rectangle", &json!({"x": 90, "y": 50, "width": 20, "height": 100})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setFill", &json!({"color": "#b04848", "ids": [stripe]})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true, "ids": [stripe]})).unwrap();
    s.execute("select.set", &json!({"ids": [id, stripe]})).unwrap();
    let g = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    (s, g)
}

fn render(s: &Session) -> Rendered {
    Renderer::new().render_region(&s.doc().unwrap().doc, Rect::new(0.0, 0.0, 200.0, 200.0), 1.0, true)
}

fn mean_diff(a: &Rendered, b: &Rendered) -> f64 {
    let sum: u64 = a.pixels.iter().zip(&b.pixels).map(|(x, y)| x.abs_diff(*y) as u64).sum();
    sum as f64 / a.pixels.len() as f64 / 255.0
}

#[test]
fn listed_applied_edited_and_undone() {
    let (mut s, g) = striped();
    let list = s.execute("effect.list", &json!({})).unwrap();
    for (id, _) in EFFECTS {
        let e = list["catalog"].as_array().unwrap().iter().find(|e| e["id"] == id).unwrap_or_else(|| panic!("{id} listed"));
        assert_eq!(e["raster"], json!(true), "{id}");
        assert!(e["params"].as_str().unwrap().contains("radius") || id == "blur.radial", "{id}");
    }
    let menus: Vec<Value> =
        EFFECTS.iter().map(|(id, _)| list["catalog"].as_array().unwrap().iter().find(|e| e["id"] == *id).unwrap()["menu"].clone()).collect();
    assert_eq!(menus, [json!(["Effect", "Blur"]), json!(["Effect", "Blur"]), json!(["Effect", "Sharpen"])]);
    for (group, menu) in [
        (BRUSH_STROKES.as_slice(), "Brush Strokes"),
        (DISTORT.as_slice(), "Distort"),
        (PIXELATE.as_slice(), "Pixelate"),
        (TEXTURE.as_slice(), "Texture"),
        (VIDEO.as_slice(), "Video"),
        ([("video.ntscColors", "{}")].as_slice(), "Video"),
    ] {
        for (id, _) in group {
            let e = list["catalog"].as_array().unwrap().iter().find(|e| e["id"] == *id).unwrap_or_else(|| panic!("{id} listed"));
            assert_eq!((&e["raster"], &e["menu"]), (&json!(true), &json!(["Effect", menu])), "{id}");
        }
    }
    // Junk parameters are stored as given and read clamped; the canvas still draws.
    for (id, _) in EFFECTS {
        s.execute("effect.apply", &json!({"effect": id, "ids": [g], "params": {"amount": 1e308, "radius": -1e308, "threshold": "x", "quality": 7}}))
            .unwrap();
    }
    let _ = render(&s);
    let n = s.doc().unwrap().doc.node(NodeId(g)).unwrap().clone();
    assert_eq!(n.appearance.effects.len(), 3);
    assert_eq!(n.appearance.effects[0].params["method"], json!("spin"), "defaults fill the rest");
    s.execute("effect.setParams", &json!({"index": 0, "ids": [g], "params": {"method": "zoom"}})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node(NodeId(g)).unwrap().appearance.effects[0].params["method"], json!("zoom"));
    for _ in 0..4 {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert!(s.doc().unwrap().doc.node(NodeId(g)).unwrap().appearance.effects.is_empty());
}

#[test]
fn each_effect_changes_the_canvas_and_draws_the_same_twice() {
    for (id, params) in EFFECTS.into_iter().chain(BRUSH_STROKES).chain(DISTORT).chain(PIXELATE).chain(TEXTURE).chain(VIDEO) {
        let (mut s, g) = striped();
        let plain = render(&s);
        let params: Value = serde_json::from_str(params).unwrap();
        s.execute("effect.apply", &json!({"effect": id, "ids": [g], "params": params})).unwrap();
        let a = render(&s);
        assert!(mean_diff(&plain, &a) > 0.0005, "{id} shows: {}", mean_diff(&plain, &a));
        assert_eq!(a.pixels, render(&s).pixels, "{id} is deterministic");
        // The multithreaded pipeline draws the same.
        let mut mt = Renderer::new();
        mt.threads = 4;
        let b = mt.render_region(&s.doc().unwrap().doc, Rect::new(0.0, 0.0, 200.0, 200.0), 1.0, true);
        assert!(mean_diff(&a, &b) < 0.002, "{id}: multithreaded differs by {}", mean_diff(&a, &b));
    }
}

#[test]
fn spin_blur_reaches_past_the_corners() {
    let (mut s, g) = striped();
    s.execute("effect.apply", &json!({"effect": "blur.radial", "ids": [g], "params": {"amount": 60, "quality": "good"}})).unwrap();
    let img = render(&s);
    // Turned corners sweep beyond the square's sides (centre 100, 100; corner 70.7 pt out).
    let p = img.pixel(100, 46);
    assert!(p[1] < 250, "the arc of the corners reaches above the square: {p:?}");
    assert!(img.pixel(100, 20)[1] > 250, "but not beyond the corners' circle");
}

#[test]
fn pdf_and_svg_write_the_effected_object_as_an_image() {
    for (id, params) in EFFECTS.into_iter().chain(BRUSH_STROKES).chain(DISTORT).chain(PIXELATE).chain(TEXTURE).chain(VIDEO) {
        let (mut s, g) = striped();
        let params: Value = serde_json::from_str(params).unwrap();
        s.execute("effect.apply", &json!({"effect": id, "ids": [g], "params": params})).unwrap();
        let doc = s.doc().unwrap().doc.clone();
        // PDF: an image that reads back as the canvas shows it.
        let r = crate::cmd::rasterfx::export_pdf_with_report(&doc, &Default::default()).unwrap();
        assert!(r.warnings.is_empty(), "{id}: {:?}", r.warnings);
        let back = vectorcraft_pdf::import(&r.bytes).unwrap();
        let diff = mean_diff(&render(&s), &Renderer::new().render_region(&back, Rect::new(0.0, 0.0, 200.0, 200.0), 1.0, true));
        assert!(diff < 0.03, "{id}: PDF reads back {:.2}% off", diff * 100.0);
        // SVG: the object becomes an image; a Gaussian Blur alone stays a live filter.
        let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap()["text"].as_str().unwrap().to_string();
        assert!(svg.contains("<image") && !svg.contains("<filter"), "{id}: {svg}");
        let flat = crate::cmd::rasterfx::flatten_pixel_effects(&doc).unwrap();
        assert!(matches!(flat.layers[0].children().unwrap()[0].kind, NodeKind::Image(_)), "{id}");
    }
    let (mut s, g) = striped();
    s.execute("effect.apply", &json!({"effect": "blur.gaussian", "ids": [g]})).unwrap();
    assert!(crate::cmd::rasterfx::flatten_pixel_effects(&s.doc().unwrap().doc).is_none());
    let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap()["text"].as_str().unwrap().to_string();
    assert!(svg.contains("feGaussianBlur") && !svg.contains("<image"));
}

#[test]
fn expand_appearance_makes_an_image() {
    let (mut s, g) = striped();
    s.execute("effect.apply", &json!({"effect": "sharpen.unsharpMask", "ids": [g]})).unwrap();
    s.execute("effect.expandAppearance", &json!({"ids": [g]})).unwrap();
    assert!(matches!(s.doc().unwrap().doc.node(NodeId(g)).unwrap().kind, NodeKind::Image(_)));
}

/// Video › NTSC Colors on the canvas: a saturated yellow, too strong for a television signal, draws
/// less saturated; the effect is listed and applies without options.
#[test]
fn ntsc_colors_draws_saturated_yellow_safer() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setFill", &json!({"color": "#ffff00", "ids": [id]})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true, "ids": [id]})).unwrap();
    let before = render(&s);
    s.execute("effect.apply", &json!({"effect": "video.ntscColors", "ids": [id]})).unwrap();
    let after = render(&s);
    let px = |r: &Rendered| r.pixel(100, 100);
    let (a, b) = (px(&before), px(&after));
    assert_eq!(&a[..3], &[255, 255, 0]);
    assert!(b[2] > 20 && b[0] < 255 && b[3] == 255, "less saturated: {b:?}");
    assert_eq!(after.pixels, render(&s).pixels, "deterministic");
}
