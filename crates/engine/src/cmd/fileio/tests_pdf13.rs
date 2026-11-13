//! `document.exportPdf {compatibility: "1.3", flattenerPreset?, flattener?}`: a PDF 1.3 file has
//! no transparency (no soft masks, blend modes, constant opacity or transparency groups): it is
//! flattened with the chosen preset, and reads back looking like the canvas.

use serde_json::{Value, json};
use vectorcraft_geom::Rect;
use vectorcraft_render::{Rendered, Renderer};

use super::*;

/// A document with overlapping transparency: a half-opaque rectangle, a Multiply ellipse, a
/// gradient fading out, a drop shadow and a PNG with see-through pixels, in an isolated page.
fn session() -> Session {
    let mut s = Session::new();
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
    run(&mut s, "file.new", json!({"width": 200, "height": 160}));
    run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 90, "height": 70}));
    run(&mut s, "paint.setFill", json!({"color": "#2060e0"}));
    run(&mut s, "shape.rectangle", json!({"x": 50, "y": 40, "width": 90, "height": 70}));
    run(&mut s, "paint.setFill", json!({"color": "#f02010"}));
    run(&mut s, "transparency.set", json!({"opacity": 50}));
    run(&mut s, "shape.ellipse", json!({"x": 90, "y": 20, "width": 80, "height": 80}));
    run(&mut s, "paint.setFill", json!({"color": "#20c040"}));
    run(&mut s, "transparency.set", json!({"blendMode": "multiply"}));
    let shadowed = run(&mut s, "shape.rectangle", json!({"x": 20, "y": 100, "width": 50, "height": 40}))["id"].clone();
    run(&mut s, "paint.setFill", json!({"color": "#e0c020"}));
    run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "ids": [shadowed]}));
    let mut png = vec![];
    image::RgbaImage::from_fn(8, 8, |x, _| image::Rgba([0, 120, 255, if x < 4 { 0 } else { 255 }]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    run(&mut s, "file.place", json!({"name": "dots.png", "dataBase64": vectorcraft_format::base64_encode(&png), "at": [150, 120]}));
    s
}

fn export(s: &mut Session, p: Value) -> (Vec<u8>, Vec<String>) {
    let v = s.execute("document.exportPdf", &p).unwrap_or_else(|e| panic!("{p}: {e}"));
    let bytes = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
    (bytes, v["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect())
}

/// The transparency `pdf` has, in its dictionaries.
fn transparency(pdf: &[u8]) -> Vec<&'static str> {
    let text = String::from_utf8_lossy(pdf);
    let mut found = vec![];
    for (what, there) in [
        ("soft masks", text.contains("/SMask") && !text.contains("/SMask/None")),
        ("blend modes", text.contains("/BM") && !text.contains("/BM/Normal")),
        ("transparency groups", text.contains("/S/Transparency")),
        ("opacity", text.contains("/ca 0.") || text.contains("/CA 0.")),
    ] {
        if there {
            found.push(what);
        }
    }
    found
}

fn render(d: &vectorcraft_doc::Document) -> Rendered {
    Renderer::new().render_region(d, Rect::new(0.0, 0.0, 200.0, 160.0), 1.0, true)
}

/// Mean difference of the colour channels of two renders, as a share of full scale.
fn mean_diff(a: &Rendered, b: &Rendered) -> f64 {
    let sum: u64 = a.pixels.iter().zip(&b.pixels).map(|(x, y)| x.abs_diff(*y) as u64).sum();
    sum as f64 / a.pixels.len() as f64 / 255.0
}

#[test]
fn pdf_1_3_flattens_all_transparency_and_looks_the_same() {
    let mut s = session();
    let before = s.doc().unwrap().doc.clone();
    // The same document at PDF 1.4 has transparency.
    let (pdf, _) = export(&mut s, json!({"compatibility": "1.4", "compression": {"compressText": false}, "preserveEditing": false}));
    assert!(transparency(&pdf).len() >= 3, "{:?}", transparency(&pdf));
    let (pdf, warnings) = export(&mut s, json!({"compatibility": "1.3", "compression": {"compressText": false}, "preserveEditing": false}));
    assert!(pdf.starts_with(b"%PDF-1.3"));
    assert!(String::from_utf8_lossy(&pdf).contains("<pdf:PDFVersion>1.3<"), "the metadata says 1.3 too");
    assert_eq!(transparency(&pdf), Vec::<&str>::new());
    assert!(warnings.iter().any(|w| w.contains("flattened") && w.contains("PDF 1.3")), "{warnings:?}");
    // It reads back looking like the canvas.
    let canvas = render(&before);
    let diff = mean_diff(&canvas, &render(&vectorcraft_pdf::import(&pdf).unwrap()));
    assert!(diff < 0.02, "read back {:.2}% off", diff * 100.0);
    // The document itself is untouched.
    assert_eq!(s.doc().unwrap().doc, before);
    // The writer refuses a document that isn't flat; a flat one with isolated page blending gets
    // no page group.
    let opts = vectorcraft_pdf::PdfOptions { settings: serde_json::from_value(json!({"compatibility": "1.3"})).unwrap(), ..Default::default() };
    let e = vectorcraft_pdf::export(&before, &opts).unwrap_err();
    assert!(matches!(&e, vectorcraft_pdf::PdfError::Unsupported(m) if m.contains("PDF 1.3")), "{e}");
    let mut flat = vectorcraft_doc::Document::new(100.0, 100.0);
    flat.page_isolate = true;
    let l = flat.default_layer().unwrap();
    let id = flat.alloc_id();
    let rect = vectorcraft_geom::shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 50.0));
    let paint = vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK);
    flat.insert(Some(l), 0, vectorcraft_doc::Node::path(id, rect, vectorcraft_doc::Appearance::basic(paint, vectorcraft_color::Paint::None, 0.0)))
        .unwrap();
    assert!(transparency(&vectorcraft_pdf::export(&flat, &opts).unwrap()).is_empty());
}

#[test]
fn the_flattener_preset_and_options_apply() {
    let mut s = session();
    let images = |pdf: &[u8]| String::from_utf8_lossy(pdf).matches("/Subtype/Image").count();
    let p = |extra: Value| {
        let mut p = json!({"compatibility": "1.3", "preserveEditing": false});
        super::pdf::merge(&mut p, &extra);
        p
    };
    let (high, _) = export(&mut s, p(json!({})));
    let (default, _) = export(&mut s, p(json!({"flattenerPreset": "High Resolution"})));
    assert_eq!(high.len(), default.len(), "the default is High Resolution");
    let (low, _) = export(&mut s, p(json!({"flattenerPreset": "low"})));
    assert!(low.len() < high.len(), "lower resolution rasters");
    // All raster: one image a flattened group at most, and still flat.
    let (raster, _) = export(&mut s, p(json!({"flattener": {"balance": 0}})));
    assert!(images(&raster) >= 1 && transparency(&raster).is_empty());
    // A saved preset by name, through every PDF path; an unknown one is refused.
    s.execute("flattener.presets.save", &json!({"name": "All Raster", "preset": "high", "balance": 0})).unwrap();
    let (saved, _) = export(&mut s, p(json!({"flattenerPreset": "All Raster"})));
    assert_eq!(saved.len(), raster.len());
    let v = s
        .execute("document.export", &json!({"format": "pdf", "compatibility": "1.3", "flattenerPreset": "All Raster", "preserveEditing": false}))
        .unwrap();
    assert_eq!(vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap().len(), raster.len());
    s.execute("pdf.preset.save", &json!({"name": "Old Press", "compatibility": "1.3", "flattenerPreset": "All Raster", "preserveEditing": false}))
        .unwrap();
    assert_eq!(export(&mut s, json!({"preset": "Old Press"})).0.len(), raster.len(), "a PDF preset keeps its flattener preset");
    assert!(s.execute("document.exportPdf", &p(json!({"flattenerPreset": "Nope"}))).is_err());
    // Other versions keep their transparency, whatever the preset.
    let (kept, warnings) = export(&mut s, json!({"flattenerPreset": "low", "compression": {"compressText": false}}));
    assert!(!transparency(&kept).is_empty() && !warnings.iter().any(|w| w.contains("flattened")));
}

#[test]
fn pdf_1_3_files_are_encrypted_with_40_bit_rc4_and_can_be_linearised() {
    let mut s = session();
    let (pdf, warnings) = export(
        &mut s,
        json!({"compatibility": "1.3", "fastWebView": true, "thumbnails": true, "security": {"openPassword": "open", "permissionsPassword": "owner", "plaintextMetadata": false}}),
    );
    assert!(pdf.starts_with(b"%PDF-1.3") && warnings.iter().all(|w| !w.contains("metadata")), "{warnings:?}");
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/V 1/R 2/Length 40") && text.contains("/Linearized 1") && text.contains("/Thumb "));
    for password in ["open", "owner"] {
        s.execute("document.close", &json!({"force": true})).ok();
        s.execute("document.open", &json!({"dataBase64": vectorcraft_format::base64_encode(&pdf), "name": "old.pdf", "password": password})).unwrap();
        assert_eq!(s.doc().unwrap().doc.artboards.len(), 1, "{password}");
    }
    assert!(
        s.execute("document.open", &json!({"dataBase64": vectorcraft_format::base64_encode(&pdf), "name": "old.pdf", "password": "nope"})).is_err()
    );
}

/// Pixel width of the widest image in the PDF (export options `p`) of a rectangle with a drop shadow, in a
/// document at `ppi` Raster Effects resolution.
fn widest_image_at(ppi: u32, p: Value) -> u32 {
    let mut s = Session::new();
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
    run(&mut s, "file.new", json!({"width": 200, "height": 200, "rasterEffectsPpi": ppi}));
    let id = run(&mut s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100}))["id"].clone();
    run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "ids": [id]}));
    let (bytes, _) = export(&mut s, p);
    let mut widest = 0;
    vectorcraft_pdf::import(&bytes).unwrap().walk(|n| {
        if let vectorcraft_doc::NodeKind::Image(im) = &n.kind {
            widest = widest.max(im.width);
        }
    });
    widest
}

/// A plain PDF takes its effect images at the document's Raster Effects resolution. PDF 1.3 and
/// PDF/X-1a files are flattened first, and the flattener preset (High Resolution by default) sets
/// the resolution of what it rasterizes, so the document's value does not change those images.
#[test]
fn pdf_1_3_and_pdfx_effect_images_come_from_the_flattener_preset_not_the_documents_resolution() {
    let _g = crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let plain: Vec<u32> = [72, 150, 300].map(|ppi| widest_image_at(ppi, json!({}))).into();
    assert!(plain[0] < plain[1] && plain[1] < plain[2], "plain PDF follows the document: {plain:?}");
    for p in [json!({"compatibility": "1.3"}), json!({"standard": "pdfX1a"})] {
        let flat: Vec<u32> = [72, 150, 300].map(|ppi| widest_image_at(ppi, p.clone())).into();
        assert!(flat[0] == flat[1] && flat[1] == flat[2], "{p}: {flat:?}");
        // The default High Resolution preset is finer than any of the three presets.
        assert!(flat[0] > plain[2], "{p}: {flat:?} against {plain:?}");
    }
}
