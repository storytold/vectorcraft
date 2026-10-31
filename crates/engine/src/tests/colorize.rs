//! A fill color colorizes grayscale and 1-bit images (`paint.setFill` on the image).

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{ImageBlob, Node, NodeKind};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s.execute("swatch.new", &json!({"name": "Ink", "color": {"c": 0, "m": 100, "y": 100, "k": 0}, "spot": true})).unwrap();
    s
}

fn png(img: image::DynamicImage) -> ImageBlob {
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    ImageBlob::png(out)
}

/// A gray 2 × 1 image (black, white), with an alpha channel when `alpha`.
fn gray(alpha: bool) -> ImageBlob {
    if alpha {
        png(image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(2, 1, vec![0, 255, 255, 128]).unwrap()))
    } else {
        png(image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(2, 1, vec![0, 255]).unwrap()))
    }
}

/// A selected embedded 2 × 1 image of `blob`.
fn image(s: &mut Session, key: &str, blob: ImageBlob) -> NodeId {
    let key = key.to_string();
    s.edit("Place", |d, sel| {
        d.images.insert(key.clone(), blob);
        let id = d.alloc_id();
        let im = vectorcraft_doc::ImageObject {
            key,
            width: 2,
            height: 1,
            xf: Default::default(),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        d.insert(d.default_layer(), 0, Node::new(id, NodeKind::Image(Box::new(im))))?;
        sel.set([id]);
        Ok(id)
    })
    .unwrap()
}

fn colorize(s: &Session, id: NodeId) -> Option<Paint> {
    s.doc().unwrap().doc.node(id).unwrap().extras().colorize.as_deref().cloned()
}

fn image_key(s: &Session, id: NodeId) -> String {
    let d = &s.doc().unwrap().doc;
    let NodeKind::Image(im) = &d.node(id).unwrap().kind else { panic!("not an image") };
    im.key.clone()
}

fn colorized_image_with_adjustment(alpha: bool) -> (Session, NodeId, Paint) {
    let mut s = session();
    let id = image(&mut s, "gray", gray(alpha));
    s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
    let before = colorize(&s, id).unwrap();
    s.execute("effect.apply", &json!({"effect": "adjust.hueSaturation", "params": {"hue": 120}})).unwrap();
    (s, id, before)
}

fn assert_adjustment_expand_rejected(command: &str) {
    let (mut s, id, before) = colorized_image_with_adjustment(true);
    let doc_before = s.doc().unwrap().doc.clone();
    let undo_before = s.doc().unwrap().history.undo.len();
    let e = s.execute(command, &json!({})).unwrap_err().to_string();
    assert!(e.contains("a process color can't colorize a grayscale image with transparency"), "{command}: {e}");
    assert_eq!(colorize(&s, id), Some(before));
    assert_eq!(s.doc().unwrap().doc, doc_before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_before);
}

fn assert_adjustment_expand_succeeds(command: &str) {
    let (mut s, id, before) = colorized_image_with_adjustment(false);
    s.execute(command, &json!({})).unwrap();
    assert_ne!(colorize(&s, id), Some(before));
    assert!(s.doc().unwrap().doc.node(id).unwrap().appearance.effects.is_empty());
}

#[test]
fn expand_appearance_rejects_adjusted_colorized_gray_image_with_alpha() {
    assert_adjustment_expand_rejected("effect.expandAppearance");
}

#[test]
fn expand_appearance_adjusts_colorized_gray_image_without_alpha() {
    assert_adjustment_expand_succeeds("effect.expandAppearance");
}

#[test]
fn object_expand_rejects_adjusted_colorized_gray_image_with_alpha() {
    assert_adjustment_expand_rejected("object.expand");
}

#[test]
fn object_expand_adjusts_colorized_gray_image_without_alpha() {
    assert_adjustment_expand_succeeds("object.expand");
}

#[test]
fn object_expand_still_expands_a_gradient_beside_an_effect_that_cannot_expand() {
    let mut s = session();
    let guide = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("effect.apply", &json!({"effect": "adjust.hueSaturation", "params": {"hue": 120}})).unwrap();
    s.execute("view.guides.make", &json!({})).unwrap();
    let rect = s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
    s.execute("select.set", &json!({"ids": [guide, rect]})).unwrap();
    s.execute("object.expand", &json!({})).unwrap();
    let doc = &s.doc().unwrap().doc;
    assert!(doc.node(NodeId(rect)).is_none_or(|n| !matches!(n.appearance.fill_paint(), Paint::Gradient(_))), "the gradient expanded");
    assert_eq!(doc.node(NodeId(guide)).unwrap().appearance.effects.len(), 1, "the guide keeps its effect");
}

#[test]
fn object_expand_restores_earlier_expansions_when_adjusted_colorize_is_rejected() {
    let (mut s, id, before) = colorized_image_with_adjustment(true);
    let a = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    let b = s.execute("shape.rectangle", &json!({"x": 50, "y": 10, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    let blend = s.execute("object.blend.make", &json!({"ids": [a, b], "steps": 1})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [id.0, blend]})).unwrap();
    let doc_before = s.doc().unwrap().doc.clone();
    let undo_before = s.doc().unwrap().history.undo.len();
    let e = s.execute("object.expand", &json!({})).unwrap_err().to_string();
    assert!(e.contains("a process color can't colorize a grayscale image with transparency"), "{e}");
    assert_eq!(colorize(&s, id), Some(before));
    assert_eq!(s.doc().unwrap().doc, doc_before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_before);
}

fn blend_with_colorized_middle(alpha: bool) -> (Session, NodeId, Paint) {
    let mut s = session();
    let back = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    let back = NodeId(back["id"].as_u64().unwrap());
    let middle = image(&mut s, "gray", gray(alpha));
    s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
    s.execute("object.transform", &json!({"ids": [middle.0], "matrix": [1, 0, 0, 1, 30, 0]})).unwrap();
    let before = colorize(&s, middle).unwrap();
    let front = s.execute("shape.rectangle", &json!({"x": 40, "y": 0, "width": 20, "height": 20})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    let front = NodeId(front["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [back.0, middle.0, front.0]})).unwrap();
    (s, middle, before)
}

#[test]
fn blend_rejects_a_process_color_on_a_colorized_gray_image_with_alpha() {
    let (mut s, middle, before) = blend_with_colorized_middle(true);
    let doc_before = s.doc().unwrap().doc.clone();
    let undo_before = s.doc().unwrap().history.undo.len();
    let e = s.execute("edit.colors.blendHorizontally", &json!({})).unwrap_err().to_string();
    assert!(e.contains("a process color can't colorize a grayscale image with transparency"), "{e}");
    assert_eq!(colorize(&s, middle), Some(before));
    assert_eq!(s.doc().unwrap().doc, doc_before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_before);
}

#[test]
fn blend_recolors_a_colorized_gray_image_without_alpha() {
    let (mut s, middle, _) = blend_with_colorized_middle(false);
    assert_eq!(s.execute("edit.colors.blendHorizontally", &json!({})).unwrap(), json!({"changed": 1}));
    assert_eq!(colorize(&s, middle), Some(Paint::solid(Color::rgb(0.5, 0.5, 0.5))));
}

#[test]
fn fill_proxy_invert_recolors_a_colorized_gray_image_and_undo_restores_it() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"color": "#336699"})).unwrap();
    let before = colorize(&s, id).unwrap();
    assert_eq!(s.execute("paint.invert", &json!({})).unwrap(), json!({"changed": 1}));
    assert_eq!(colorize(&s, id).and_then(|p| p.color()).map(|c| c.to_hex()), Some("#cc9966".into()));
    assert_eq!(s.proxy_paints().0.color().map(|c| c.to_hex()), Some("#cc9966".into()));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(colorize(&s, id), Some(before));
}

#[test]
fn fill_proxy_complement_recolors_a_colorized_gray_image_and_undo_restores_it() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let before = colorize(&s, id).unwrap();
    let expected = Paint::solid(Color::rgb8(0, 255, 255));
    assert_eq!(s.execute("paint.complement", &json!({})).unwrap(), json!({"changed": 1}));
    assert_eq!(colorize(&s, id), Some(expected.clone()));
    assert_eq!(s.proxy_paints().0, expected);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(colorize(&s, id), Some(before));
}

#[test]
fn stroke_proxy_invert_leaves_a_colorized_image_fill_unchanged() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"color": "#336699"})).unwrap();
    let before = colorize(&s, id);
    let key = image_key(&s, id);
    s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().unwrap().paint.color(), Some(Color::rgb8(255, 0, 0)));
    assert_eq!(s.execute("paint.invert", &json!({"stroke": true})).unwrap(), json!({"changed": 1}));
    assert_eq!(s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().unwrap().paint.color(), Some(Color::rgb8(0, 255, 255)));
    assert_eq!(colorize(&s, id), before);
    assert_eq!(image_key(&s, id), key);
}

#[test]
fn edit_colors_invert_without_fill_recolors_a_colorized_images_stroke() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"color": "#336699"})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
    let before = colorize(&s, id);
    let key = image_key(&s, id);
    assert_eq!(s.execute("edit.colors.invert", &json!({"fill": false, "stroke": true})).unwrap(), json!({"changed": 1}));
    assert_eq!(s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().unwrap().paint.color(), Some(Color::rgb8(0, 255, 255)));
    assert_eq!(colorize(&s, id), before);
    assert_eq!(image_key(&s, id), key);
}

#[test]
fn colorized_alpha_image_rejects_a_recolored_spot_but_accepts_inverted_black() {
    let mut s = session();
    let id = image(&mut s, "alpha", gray(true));
    s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
    let before = colorize(&s, id);
    for cmd in ["paint.invert", "paint.complement", "edit.colors.invert"] {
        let e = s.execute(cmd, &json!({"includeImages": false})).unwrap_err().to_string();
        assert!(e.contains("a process color can't colorize a grayscale image with transparency"), "{cmd}: {e}");
        assert_eq!(colorize(&s, id), before, "{cmd}");
    }
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("paint.invert", &json!({})).unwrap();
    assert_eq!(colorize(&s, id), Some(Paint::solid(Color::rgb8(255, 255, 255))));
}

#[test]
fn edit_colors_recolors_an_image_colorize_with_pixels_excluded() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let before = colorize(&s, id);
    assert_eq!(s.execute("edit.colors.invert", &json!({"includeImages": false})).unwrap(), json!({"changed": 1}));
    assert_eq!(colorize(&s, id), Some(Paint::solid(Color::rgb8(0, 255, 255))));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(colorize(&s, id), before);
}

#[test]
fn edit_colors_invert_recolors_colorize_without_changing_pixels_and_undo_restores() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"color": "#336699"})).unwrap();
    let before = colorize(&s, id);
    let key = image_key(&s, id);
    assert_eq!(s.execute("edit.colors.invert", &json!({})).unwrap(), json!({"changed": 1}));
    assert_eq!(colorize(&s, id).and_then(|p| p.color()).map(|c| c.to_hex()), Some("#cc9966".into()));
    assert_eq!(image_key(&s, id), key);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(colorize(&s, id), before);
    assert_eq!(image_key(&s, id), key);
}

#[test]
fn edit_colors_invert_recolors_colorized_image_inside_group() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"color": "#336699"})).unwrap();
    let shape = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let shape = NodeId(shape["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0, shape.0]})).unwrap();
    let group = s.execute("object.group", &json!({})).unwrap();
    assert!(s.execute("edit.colors.invert", &json!({"ids": [group["id"]]})).unwrap()["changed"].as_u64().unwrap() >= 1);
    assert_eq!(colorize(&s, id).and_then(|p| p.color()).map(|c| c.to_hex()), Some("#cc9966".into()));
}

#[test]
fn edit_colors_invert_without_fill_leaves_colorized_image_unchanged() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"color": "#336699"})).unwrap();
    let before = colorize(&s, id);
    let key = image_key(&s, id);
    assert_eq!(s.execute("edit.colors.invert", &json!({"fill": false})).unwrap(), json!({"changed": 0}));
    assert_eq!(colorize(&s, id), before);
    assert_eq!(image_key(&s, id), key);
}

#[test]
fn recolor_apply_recolors_a_colorized_image_and_checks_alpha() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    assert_eq!(s.execute("recolor.apply", &json!({"map": {"#ff0000": "#00ff00"}, "includeImages": false})).unwrap()["changed"], 1);
    assert_eq!(colorize(&s, id), Some(Paint::solid(Color::rgb8(0, 255, 0))));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(colorize(&s, id), Some(Paint::solid(Color::rgb8(255, 0, 0))));

    let mut s = session();
    let id = image(&mut s, "alpha", gray(true));
    s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
    let before = colorize(&s, id);
    let e = s.execute("recolor.apply", &json!({"map": {"cmyk 0 100 100 0": "#00ff00"}, "includeImages": false})).unwrap_err().to_string();
    assert!(e.contains("a process color can't colorize a grayscale image with transparency"), "{e}");
    assert_eq!(colorize(&s, id), before);
}

#[test]
fn proxy_invert_recolors_a_colorized_image_and_another_fill_in_one_undo_step() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let shape = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let shape = NodeId(shape["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0, shape.0]})).unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    assert_eq!(s.execute("paint.invert", &json!({})).unwrap(), json!({"changed": 2}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(colorize(&s, id), Some(Paint::solid(Color::rgb8(0, 255, 255))));
    assert_eq!(s.doc().unwrap().doc.node(shape).unwrap().appearance.fill().unwrap().paint.color(), Some(Color::rgb8(255, 255, 0)));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(colorize(&s, id), Some(Paint::solid(Color::rgb8(255, 0, 0))));
    assert_eq!(s.doc().unwrap().doc.node(shape).unwrap().appearance.fill().unwrap().paint.color(), Some(Color::rgb8(0, 0, 255)));
}

#[test]
fn colorized_gray_image_is_flattened_for_pdf_13() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    assert!(!s.doc().unwrap().doc.layers[0].shows_transparency());
    let plain = s.execute("document.exportPdf", &json!({"compatibility": "1.3"})).unwrap();
    assert!(
        plain["warnings"].as_array().unwrap().iter().all(|w| !w.as_str().unwrap().contains("transparency is flattened")),
        "{}",
        plain["warnings"]
    );

    s.execute("paint.setFill", &json!({"color": "#ff0000", "ids": [id.0]})).unwrap();
    assert!(s.doc().unwrap().doc.layers[0].shows_transparency());
    let tinted = s.execute("document.exportPdf", &json!({"compatibility": "1.3"})).unwrap();
    assert!(
        tinted["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("transparency is flattened")),
        "{}",
        tinted["warnings"]
    );
    let bytes = vectorcraft_format::base64_decode(tinted["dataBase64"].as_str().unwrap()).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("/SMask"), "PDF 1.3 has no soft mask");
}

#[test]
fn colorized_gray_image_is_flattened_for_pdf_x_1a() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"color": "#ff0000", "ids": [id.0]})).unwrap();
    let out = s.execute("document.exportPdf", &json!({"standard": "pdfX1a"})).unwrap();
    assert!(out["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("transparency is flattened")), "{}", out["warnings"]);
    let bytes = vectorcraft_format::base64_decode(out["dataBase64"].as_str().unwrap()).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("/SMask"), "PDF/X-1a has no soft mask");
}

#[test]
fn colorized_gray_image_is_omitted_from_legacy_ai_when_requested() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"color": "#ff0000", "ids": [id.0]})).unwrap();
    let out = s.execute("file.saveCopy", &json!({"format": "ai", "aiVersion": "7", "transparentImages": "omit"})).unwrap();
    assert!(out["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("are left out")), "{}", out["warnings"]);
}

#[test]
fn flattened_colorized_image_shows_the_art_under_white_pixels() {
    let mut s = session();
    let background = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff", "ids": [background]})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true, "ids": [background]})).unwrap();
    let id = image(&mut s, "gray", gray(false));
    s.edit("Scale image", |d, _| {
        if let NodeKind::Image(im) = &mut d.node_mut(id).unwrap().kind {
            im.xf = vectorcraft_geom::Affine::scale(20.0);
        }
        Ok(())
    })
    .unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000", "ids": [id.0]})).unwrap();
    s.execute("object.arrange.bringToFront", &json!({"ids": [id.0]})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let mut options = crate::cmd::flatten::FlattenOptions::preset("low").unwrap();
    options.no_transparency = Some(Default::default());
    options.clip_complex_regions = true;
    let flat = crate::cmd::flatten::flatten_document(doc, &options).unwrap().unwrap();
    let region = vectorcraft_geom::Rect::new(0.0, 0.0, 40.0, 20.0);
    let before = vectorcraft_render::Renderer::new().render_region(doc, region, 1.0, true);
    let after = vectorcraft_render::Renderer::new().render_region(&flat, region, 1.0, true);
    for (x, expected) in [(10, [255, 0, 0]), (30, [0, 0, 255])] {
        let a = before.pixel(x, 10);
        let b = after.pixel(x, 10);
        for channel in 0..3 {
            assert!((i16::from(a[channel]) - i16::from(b[channel])).abs() <= 10, "x={x}: {a:?} became {b:?}");
            assert!((i16::from(b[channel]) - expected[channel]).abs() <= 10, "x={x}: {b:?}");
        }
    }
}

#[test]
fn a_fill_color_colorizes_a_gray_image_and_none_takes_it_off() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"swatch": "Ink", "tint": 40})).unwrap();
    let Some(Paint::Solid { swatch, tint, .. }) = colorize(&s, id) else { panic!("not colorized: {:?}", colorize(&s, id)) };
    assert_eq!((swatch.as_deref(), tint), (Some("Ink"), 0.4));
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    assert!(n.appearance.items.is_empty(), "no fill item over the image");
    assert_eq!(s.proxy_paints().0, colorize(&s, id).unwrap(), "the Fill proxy shows the color");
    // A process color too.
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    assert!(matches!(colorize(&s, id), Some(Paint::Solid { swatch: None, .. })));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(matches!(colorize(&s, id), Some(Paint::Solid { swatch: Some(_), .. })), "undo restores the spot color");
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    assert_eq!(colorize(&s, id), None);
    // `none` takes the color off after an edit that leaves the image pixels gray.
    s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
    let key = image_key(&s, id);
    s.execute("edit.colors.invert", &json!({"includeImages": true})).unwrap();
    let d = &s.doc().unwrap().doc;
    let NodeKind::Image(im) = &d.node(id).unwrap().kind else { panic!() };
    assert_eq!(im.key, key);
    assert!(d.image_blob(im).and_then(|b| b.gray_source()).is_some(), "the pixels remain gray");
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    assert_eq!(colorize(&s, id), None);

    // `none` also takes the color off when a colorized image has color pixels.
    s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
    s.edit("Replace Image", |d, _| {
        d.images.insert("rgb".into(), png(image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(2, 1, vec![255, 0, 0, 0, 0, 255]).unwrap())));
        if let Some(node) = d.node_mut(id)
            && let NodeKind::Image(im) = &mut node.kind
        {
            im.key = "rgb".into();
        }
        Ok(())
    })
    .unwrap();
    let d = &s.doc().unwrap().doc;
    let NodeKind::Image(im) = &d.node(id).unwrap().kind else { panic!() };
    assert_eq!(d.image_blob(im).and_then(|b| b.gray_source()), None, "the pixels are no longer gray");
    assert!(colorize(&s, id).is_some());
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    assert_eq!(colorize(&s, id), None);
}

#[test]
fn an_image_with_alpha_takes_a_spot_color_black_or_white_only() {
    let mut s = session();
    let id = image(&mut s, "alpha", gray(true));
    let e = s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap_err().to_string();
    assert!(e.contains("a process color can't colorize a grayscale image with transparency"), "{e}");
    assert_eq!(colorize(&s, id), None);
    s.execute("paint.setFill", &json!({"color": {"gray": 100}})).unwrap();
    assert!(colorize(&s, id).is_some(), "black");
    s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
    assert!(matches!(colorize(&s, id), Some(Paint::Solid { swatch: Some(_), .. })));
}

/// A swatch edit that would leave an image with alpha in a process color is refused, as
/// `paint.setFill` refuses one.
#[test]
fn swatch_edits_keep_an_image_with_alpha_out_of_process_colors() {
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Other", "color": {"c": 100, "m": 0, "y": 0, "k": 0}, "global": true})).unwrap();
    let id = image(&mut s, "alpha", gray(true));
    s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
    let before = colorize(&s, id);
    let message = "a process color can't colorize a grayscale image with transparency";
    for (cmd, p) in [
        ("swatch.edit", json!({"name": "Ink", "spot": false})),
        ("swatch.edit", json!({"name": "Ink", "spot": false, "global": false})),
        ("swatch.setSpot", json!({"name": "Ink", "spot": false})),
        ("swatch.delete", json!({"names": ["Ink"]})),
        ("swatch.merge", json!({"names": ["Other", "Ink"]})),
        ("swatch.resetDefaults", json!({"replace": true})),
    ] {
        let e = s.execute(cmd, &p).unwrap_err().to_string();
        assert!(e.contains(message), "{cmd} {p}: {e}");
        assert_eq!(colorize(&s, id), before, "{cmd} {p} left the image as it was");
    }
    // A new color for the spot swatch keeps it a spot color.
    s.execute("swatch.edit", &json!({"name": "Ink", "color": {"c": 100, "m": 0, "y": 0, "k": 0}})).unwrap();
    // An image without alpha takes a process color.
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
    s.execute("swatch.edit", &json!({"name": "Ink", "spot": false})).unwrap();
    assert!(colorize(&s, id).is_some());
}

/// With the Appearance panel's active item on a fill of the image, the paint goes to that fill.
#[test]
fn the_active_fill_item_takes_the_paint_instead() {
    let mut s = session();
    let id = image(&mut s, "alpha", gray(true));
    s.execute("appearance.addFill", &json!({})).unwrap();
    let index = s.doc().unwrap().doc.node(id).unwrap().appearance.items.iter().position(|it| it.is_fill()).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": index})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    assert_eq!(colorize(&s, id), None);
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let blue = vectorcraft_color::Color::rgb(0.0, 0.0, 1.0);
    assert_eq!(n.appearance.fill().and_then(|f| f.paint.color()), Some(blue), "the fill item took the color");
}

/// `none` on a gray image without a color takes the paint off its fill item.
#[test]
fn none_on_a_gray_image_without_a_color_clears_its_fill_item() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("appearance.addFill", &json!({})).unwrap();
    let index = s.doc().unwrap().doc.node(id).unwrap().appearance.items.iter().position(|it| it.is_fill()).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": index})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": null})).unwrap();
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    assert_eq!(colorize(&s, id), None);
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    assert_eq!(n.appearance.fill().map(|f| &f.paint), Some(&Paint::None), "the fill item's paint is none");
}

#[test]
fn a_color_image_takes_a_fill_item_as_before() {
    let mut s = session();
    let rgb = png(image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(2, 1, vec![255, 0, 0, 0, 0, 255]).unwrap()));
    let id = image(&mut s, "rgb", rgb);
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    assert_eq!(colorize(&s, id), None);
    assert!(s.doc().unwrap().doc.node(id).unwrap().appearance.fill().is_some(), "a fill item, as before");
}

#[test]
fn colorized_images_save_and_open_with_their_color() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(d, false)).unwrap();
    assert_eq!(&back.layers, &d.layers);
    assert!(back.node(id).unwrap().extras().colorize.is_some());
    // Swatch edits reach it as they reach fills.
    s.execute("swatch.edit", &json!({"name": "Ink", "color": {"c": 100, "m": 0, "y": 0, "k": 0}})).unwrap();
    let Some(Paint::Solid { color, .. }) = colorize(&s, id) else { panic!() };
    assert_eq!(color, vectorcraft_color::Color::Cmyk { c: 1.0, m: 0.0, y: 0.0, k: 0.0 });
}
