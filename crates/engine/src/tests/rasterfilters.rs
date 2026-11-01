//! The Photoshop-style raster effects (Effect › Artistic, Blur › Radial Blur and Smart Blur, Brush Strokes,
//! Distort › Diffuse Glow, Glass and Ocean Ripple, Pixelate › Color Halftone, Crystallize, Mezzotint and Pointillize, Sharpen › Unsharp Mask, Sketch › Chalk & Charcoal, Chrome, Graphic Pen, Halftone Pattern and Reticulation, Texture ›
//! Craquelure, Grain, Mosaic Tiles, Patchwork, Stained Glass and Texturizer, Video): applied and
//! edited as commands, listed in the catalogue, drawn on the canvas, and written to PDF and SVG
//! as images of the effected object.

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_geom::Rect;
use vectorcraft_render::{Rendered, Renderer};

use crate::*;

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

/// Effect › Artistic, at their defaults.
const ARTISTIC: [(&str, &str); 15] = [
    ("artistic.coloredPencil", "{}"),
    ("artistic.cutout", "{}"),
    ("artistic.dryBrush", "{}"),
    ("artistic.filmGrain", "{}"),
    ("artistic.fresco", "{}"),
    ("artistic.neonGlow", "{}"),
    // Wide Blurry softens dabs; Simple leaves uniform fills unchanged on the striped art.
    ("artistic.paintDaubs", r#"{"brushType": "wideBlurry"}"#),
    ("artistic.paletteKnife", "{}"),
    ("artistic.plasticWrap", "{}"),
    ("artistic.posterEdges", "{}"),
    ("artistic.underpainting", "{}"),
    ("artistic.roughPastels", "{}"),
    ("artistic.smudgeStick", "{}"),
    ("artistic.sponge", "{}"),
    ("artistic.watercolor", "{}"),
];

/// Effect › Distort (Diffuse Glow with its highlights glowing from a low brightness up, as the
/// striped art's reds are dark).
const DISTORT: [(&str, &str); 3] =
    [("distort.diffuseGlow", r#"{"clearAmount": 0, "glowAmount": 20}"#), ("distort.glass", "{}"), ("distort.oceanRipple", "{}")];

/// Effect › Pixelate, at their defaults.
const PIXELATE: [(&str, &str); 4] =
    [("pixelate.colorHalftone", "{}"), ("pixelate.crystallize", "{}"), ("pixelate.mezzotint", "{}"), ("pixelate.pointillize", "{}")];

/// Effect › Sketch, at their defaults.
const SKETCH: [(&str, &str); 13] = [
    ("sketch.basRelief", "{}"),
    ("sketch.chalkCharcoal", "{}"),
    ("sketch.charcoal", "{}"),
    ("sketch.chrome", "{}"),
    ("sketch.conteCrayon", "{}"),
    ("sketch.graphicPen", "{}"),
    ("sketch.halftonePattern", "{}"),
    ("sketch.notePaper", "{}"),
    ("sketch.photocopy", "{}"),
    ("sketch.plaster", "{}"),
    ("sketch.reticulation", "{}"),
    ("sketch.stamp", "{}"),
    ("sketch.waterPaper", "{}"),
];

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
        (ARTISTIC.as_slice(), "Artistic"),
        (BRUSH_STROKES.as_slice(), "Brush Strokes"),
        (DISTORT.as_slice(), "Distort"),
        (PIXELATE.as_slice(), "Pixelate"),
        (SKETCH.as_slice(), "Sketch"),
        (TEXTURE.as_slice(), "Texture"),
        (VIDEO.as_slice(), "Video"),
        ([("video.ntscColors", "{}")].as_slice(), "Video"),
    ] {
        for (id, _) in group {
            let e = list["catalog"].as_array().unwrap().iter().find(|e| e["id"] == *id).unwrap_or_else(|| panic!("{id} listed"));
            assert_eq!((&e["raster"], &e["menu"]), (&json!(true), &json!(["Effect", menu])), "{id}");
        }
    }
    // Numbers far out of range are stored as given and read clamped; the canvas still draws.
    for (id, _) in EFFECTS {
        let defaults = vectorcraft_render::effects::default_params(id).unwrap();
        let junk: serde_json::Map<String, Value> = defaults
            .as_object()
            .unwrap()
            .iter()
            .filter(|(_, v)| v.is_number())
            .enumerate()
            .map(|(i, (k, _))| (k.clone(), json!(if i % 2 == 0 { 1e308 } else { -1e308 })))
            .collect();
        assert!(!junk.is_empty(), "{id}");
        s.execute("effect.apply", &json!({"effect": id, "ids": [g], "params": junk})).unwrap();
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
    for (id, params) in
        EFFECTS.into_iter().chain(ARTISTIC).chain(BRUSH_STROKES).chain(DISTORT).chain(PIXELATE).chain(SKETCH).chain(TEXTURE).chain(VIDEO)
    {
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

/// Halftone Pattern's Size, Contrast and Pattern Type are saved with the document and draw the same
/// when it opens again.
#[test]
fn halftone_pattern_settings_survive_a_save_and_open() {
    let (mut s, g) = striped();
    let set = json!({"size": 7.0, "contrast": 20.0, "patternType": "line"});
    s.execute("effect.apply", &json!({"effect": "sketch.halftonePattern", "ids": [g], "params": set})).unwrap();
    let drawn = render(&s);
    let doc = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(doc.node(NodeId(g)).unwrap().appearance.effects[0].params, set);
    assert_eq!(Renderer::new().render_region(&doc, Rect::new(0.0, 0.0, 200.0, 200.0), 1.0, true).pixels, drawn.pixels);
    // Each setting shows: changing any one draws differently.
    for change in [json!({"size": 3.0}), json!({"contrast": 50.0}), json!({"patternType": "dot"})] {
        s.execute("effect.setParams", &json!({"index": 0, "ids": [g], "params": change})).unwrap();
        assert!(mean_diff(&drawn, &render(&s)) > 0.001, "{change}");
        s.execute("effect.setParams", &json!({"index": 0, "ids": [g], "params": set})).unwrap();
    }
}

/// The Artistic filters' settings are saved with the document and draw the same when it opens
/// again; changing any one of them draws differently.
#[test]
fn artistic_settings_survive_a_save_and_open() {
    for (id, set, changes) in [
        (
            "artistic.cutout",
            json!({"levels": 3.0, "edgeSimplicity": 2.0, "edgeFidelity": 3.0}),
            [json!({"levels": 2.0}), json!({"edgeSimplicity": 10.0})],
        ),
        ("artistic.fresco", json!({"brushSize": 4.0, "brushDetail": 6.0, "texture": 2.0}), [json!({"brushSize": 8.0}), json!({"texture": 3.0})]),
        (
            "artistic.neonGlow",
            json!({"glowSize": -6.0, "glowBrightness": 30.0, "color": "#ff3300"}),
            [json!({"glowSize": 6.0}), json!({"color": "#00ff00"})],
        ),
        (
            "artistic.posterEdges",
            json!({"edgeThickness": 4.0, "edgeIntensity": 5.0, "posterization": 1.0}),
            [json!({"edgeIntensity": 0.0}), json!({"posterization": 6.0})],
        ),
        (
            "artistic.plasticWrap",
            json!({"highlightStrength": 18.0, "detail": 10.0, "smoothness": 8.0}),
            [json!({"highlightStrength": 5.0}), json!({"highlightStrength": 0.0})],
        ),
        (
            "artistic.roughPastels",
            json!({
                "strokeLength": 8.0,
                "strokeDetail": 12.0,
                "texture": "sandstone",
                "scaling": 100.0,
                "relief": 8.0,
                "lightDirection": "topLeft",
                "invert": false
            }),
            [json!({"strokeLength": 20.0}), json!({"relief": 0.0})],
        ),
    ] {
        let (mut s, g) = striped();
        // Black and a grey beside the reds: Cutout puts the grey with black at 2 levels and apart
        // from it at 3.
        let mut ids = vec![g];
        for (x, color) in [(55, "#000000"), (115, "#666666")] {
            let id = s.execute("shape.ellipse", &json!({"x": x, "y": 60, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap();
            s.execute("paint.setFill", &json!({"color": color, "ids": [id]})).unwrap();
            ids.push(id);
        }
        s.execute("select.set", &json!({"ids": ids})).unwrap();
        let g = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
        s.execute("effect.apply", &json!({"effect": id, "ids": [g], "params": set})).unwrap();
        let drawn = render(&s);
        let doc = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
        assert_eq!(doc.node(NodeId(g)).unwrap().appearance.effects[0].params, set, "{id}");
        assert_eq!(Renderer::new().render_region(&doc, Rect::new(0.0, 0.0, 200.0, 200.0), 1.0, true).pixels, drawn.pixels, "{id}");
        for change in changes {
            s.execute("effect.setParams", &json!({"index": 0, "ids": [g], "params": change})).unwrap();
            assert!(mean_diff(&drawn, &render(&s)) > 0.001, "{id}: {change}");
            s.execute("effect.setParams", &json!({"index": 0, "ids": [g], "params": set})).unwrap();
        }
    }
}

/// A PNG of rings around its center, 12 × 10 pixels.
fn rings_png() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(12, 10, |x, y| {
        let v = ((f64::from(x) - 5.5).hypot(f64::from(y) - 4.5) * 50.0) as u32 % 256;
        image::Rgba([v as u8, v as u8, 255 - v as u8, 255])
    });
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    png
}

/// Glass's parameters on the striped group.
fn glass_params(s: &Session, g: u64) -> Value {
    s.doc().unwrap().doc.node(NodeId(g)).unwrap().appearance.effects[0].params.clone()
}

/// Effect › Distort › Glass › Load Texture: an image file, given by path or as bytes, is stored
/// with the effect as its gray levels and picked; the canvas draws it, and a save and open keeps it.
#[test]
fn glass_loads_a_texture_saves_it_and_draws_it() {
    let png = rings_png();
    let data = vectorcraft_format::base64_encode(&png);
    let (mut s, g) = striped();
    let file = json!({"name": "rings.png", "dataBase64": data});
    s.execute("effect.apply", &json!({"effect": "distort.glass", "ids": [g], "params": {"distortion": 12, "textureImage": file}})).unwrap();
    let p = glass_params(&s, g);
    assert_eq!(p["texture"], json!("loaded"), "loading a texture picks it");
    let stored = p["textureImage"].as_str().unwrap().to_string();
    let texture = vectorcraft_render::effects::pixel::texture(&stored).unwrap();
    assert_eq!(texture.size(), (12, 10));
    let loaded = render(&s);
    // The same Glass with each built-in texture draws differently.
    for (_, builtin) in vectorcraft_render::effects::pixel::GLASS_TEXTURES {
        s.execute("effect.setParams", &json!({"index": 0, "ids": [g], "params": {"texture": builtin}})).unwrap();
        assert!(mean_diff(&loaded, &render(&s)) > 0.001, "{builtin}");
    }
    s.execute("effect.setParams", &json!({"index": 0, "ids": [g], "params": {"texture": "loaded"}})).unwrap();
    assert_eq!(render(&s).pixels, loaded.pixels, "the texture stayed with the effect");
    // Saved and opened again: the same texture, drawn the same.
    let doc = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(doc.node(NodeId(g)).unwrap().appearance.effects[0].params, glass_params(&s, g));
    assert_eq!(Renderer::new().render_region(&doc, Rect::new(0.0, 0.0, 200.0, 200.0), 1.0, true).pixels, loaded.pixels);
    // The same file by path, through Effect Options, stores the same texture.
    let dir = std::env::temp_dir().join(format!("vc-glass-texture-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("rings.png");
    std::fs::write(&path, &png).unwrap();
    let (mut t, h) = striped();
    t.execute("effect.apply", &json!({"effect": "distort.glass", "ids": [h], "params": {"distortion": 12}})).unwrap();
    assert!(glass_params(&t, h).get("textureImage").is_none(), "a Glass without a loaded texture has no textureImage");
    t.execute("effect.setParams", &json!({"index": 0, "ids": [h], "params": {"textureImage": {"path": path.to_string_lossy()}}})).unwrap();
    assert_eq!(glass_params(&t, h)["textureImage"], json!(stored));
    assert_eq!(glass_params(&t, h)["texture"], json!("loaded"));
    assert_eq!(render(&t).pixels, loaded.pixels);
    let _ = std::fs::remove_dir_all(&dir);
    // The stored string is taken as it is (as effect.list gives it, and a graphic style copies it).
    t.execute("effect.apply", &json!({"effect": "distort.glass", "ids": [h], "params": {"textureImage": stored, "texture": "frosted"}})).unwrap();
    let second = t.doc().unwrap().doc.node(NodeId(h)).unwrap().appearance.effects[1].params.clone();
    assert_eq!((&second["textureImage"], &second["texture"]), (&json!(stored), &json!("frosted")), "kept, not picked: texture was given");
    t.execute("edit.undo", &json!({})).unwrap();
    t.execute("edit.undo", &json!({})).unwrap();
    assert!(glass_params(&t, h).get("textureImage").is_none(), "undone");
}

/// A texture that isn't an image file, or isn't one Glass stored, is refused and changes nothing.
#[test]
fn glass_refuses_a_texture_that_is_not_an_image() {
    let (mut s, g) = striped();
    let junk = vectorcraft_format::base64_encode(b"not an image");
    let not_stored = "textureImage: give {path} or {name, dataBase64}";
    let refused = [
        (json!({"name": "a.png", "dataBase64": junk}), "not an image VectorCraft reads"),
        (json!("bm90IGEgdGV4dHVyZQ=="), not_stored),
        (json!(5), not_stored),
        (json!({"path": "C:/no/such/texture.png"}), "texture.png"),
    ];
    for (texture, says) in refused {
        let e = s.execute("effect.apply", &json!({"effect": "distort.glass", "ids": [g], "params": {"textureImage": texture}})).unwrap_err();
        assert!(e.to_string().contains(says), "{texture}: {e}");
        assert!(s.doc().unwrap().doc.node(NodeId(g)).unwrap().appearance.effects.is_empty(), "{texture}");
    }
    s.execute("effect.apply", &json!({"effect": "distort.glass", "ids": [g]})).unwrap();
    let before = glass_params(&s, g);
    let e = s.execute("effect.setParams", &json!({"index": 0, "ids": [g], "params": {"textureImage": {"dataBase64": junk}}})).unwrap_err();
    assert!(e.to_string().contains("textureImage"), "{e}");
    assert_eq!(glass_params(&s, g), before);
}

/// Effect › Texture › Texturizer › Load Texture: an image is stored with the effect and drawn in relief.
#[test]
fn texturizer_loads_a_texture_and_draws_it() {
    let png = rings_png();
    let data = vectorcraft_format::base64_encode(&png);
    let (mut s, g) = striped();
    let file = json!({"name": "rings.png", "dataBase64": data});
    s.execute("effect.apply", &json!({"effect": "texture.texturizer", "ids": [g], "params": {"relief": 20, "textureImage": file}})).unwrap();
    let p = glass_params(&s, g);
    assert_eq!(p["texture"], json!("loaded"));
    let loaded = render(&s);
    assert!(glass_params(&s, g)["textureImage"].as_str().is_some());
    for (_, builtin) in vectorcraft_render::effects::pixel::TEXTURES {
        s.execute("effect.setParams", &json!({"index": 0, "ids": [g], "params": {"texture": builtin}})).unwrap();
        assert!(mean_diff(&loaded, &render(&s)) > 0.001, "{builtin}");
    }
    s.execute("effect.setParams", &json!({"index": 0, "ids": [g], "params": {"texture": "loaded"}})).unwrap();
    assert_eq!(render(&s).pixels, loaded.pixels);
    let doc = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(doc.node(NodeId(g)).unwrap().appearance.effects[0].params, glass_params(&s, g));
    assert_eq!(Renderer::new().render_region(&doc, Rect::new(0.0, 0.0, 200.0, 200.0), 1.0, true).pixels, loaded.pixels);
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
    for (id, params) in
        EFFECTS.into_iter().chain(ARTISTIC).chain(BRUSH_STROKES).chain(DISTORT).chain(PIXELATE).chain(SKETCH).chain(TEXTURE).chain(VIDEO)
    {
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
