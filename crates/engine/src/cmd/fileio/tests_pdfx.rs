//! `document.exportPdf {standard}` with the PDF/X standards: choosing one sets its version and
//! turns off what it forbids, PDF/X-1a and PDF/X-3 flatten transparency (images with see-through
//! pixels too) before writing, PDF/X-4 keeps it, and passwords are refused.

use serde_json::{Value, json};

use super::*;

/// A document with a CMYK rectangle under a half-opaque RGB one, and a PNG with see-through
/// pixels over both.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 60, "height": 60})).unwrap();
    s.execute("paint.setFill", &json!({"color": {"model": "cmyk", "c": 1.0, "m": 0.0, "y": 0.0, "k": 0.0}})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 40, "y": 30, "width": 60, "height": 60})).unwrap();
    s.execute("paint.setFill", &json!({"color": {"model": "rgb", "r": 1.0, "g": 0.2, "b": 0.0}})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    let mut png = vec![];
    image::RgbaImage::from_fn(8, 8, |x, _| image::Rgba([0, 120, 255, if x < 4 { 0 } else { 255 }]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    s.execute("file.place", &json!({"name": "dots.png", "dataBase64": vectorcraft_format::base64_encode(&png), "at": [150, 50]})).unwrap();
    s
}

/// `document.exportPdf` with `p` (content streams readable) → (the file's text, warnings).
fn pdf(s: &mut Session, mut p: Value) -> (String, Vec<String>) {
    p["compression"] = json!({"compressText": false});
    let v = s.execute("document.exportPdf", &p).unwrap();
    let bytes = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
    let warnings = v["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect();
    (String::from_utf8_lossy(&bytes).into_owned(), warnings)
}

#[test]
fn pdfx_1a_and_x3_flatten_transparency_and_x4_keeps_it() {
    let mut s = session();
    let before = s.doc().unwrap().doc.clone();
    for (standard, header) in [("pdfX1a", "%PDF-1.3"), ("pdfX3", "%PDF-1.3")] {
        let (text, warnings) = pdf(&mut s, json!({"standard": standard}));
        assert!(text.starts_with(header), "{standard}");
        assert!(warnings.iter().any(|w| w.contains("flattened")), "{standard}: {warnings:?}");
        for no in ["/SMask", "/S/Transparency", "/ca 0.5"] {
            assert!(!text.contains(no), "{standard}: no {no}");
        }
        assert!(text.contains("/S/GTS_PDFX") && text.contains("/GTS_PDFXVersion"), "{standard}");
    }
    // PDF/X-1a: CMYK only, the flattened colours too.
    let (text, _) = pdf(&mut s, json!({"standard": "pdfX1a"}));
    assert!(!text.contains("/DeviceRGB") && !text.contains("/ICCBased"));
    assert!(!text.split_whitespace().any(|t| t == "rg"), "no RGB fills");
    // The document itself is untouched.
    assert_eq!(s.doc().unwrap().doc, before);
    // PDF/X-4 keeps transparency (and the image's soft mask).
    let (text, warnings) = pdf(&mut s, json!({"preset": "PDF/X-4:2010"}));
    assert!(text.starts_with("%PDF-1.6"));
    assert!(text.contains("/SMask") && !warnings.iter().any(|w| w.contains("flattened")), "{warnings:?}");
    assert!(text.contains("<pdfxid:GTS_PDFXVersion"));
}

#[test]
fn choosing_a_pdfx_standard_sets_its_version_and_turns_off_what_it_forbids() {
    let mut s = session();
    let settings = |s: &mut Session, p: Value| s.execute("document.pdfSettings", &p).unwrap()["settings"].clone();
    // The default preset is PDF 1.7 and keeps editing data.
    let v = settings(&mut s, json!({"standard": "pdfX4"}));
    assert_eq!((&v["compatibility"], &v["preserveEditing"]), (&json!("1.6"), &json!(false)));
    let v = settings(&mut s, json!({"standard": "pdfX1a", "createLayers": null, "preset": "Press Quality"}));
    assert_eq!((&v["compatibility"], &v["createLayers"]), (&json!("1.3"), &json!(false)));
    let v = settings(&mut s, json!({"standard": "pdfX4", "compatibility": "1.5", "createLayers": true}));
    assert_eq!((&v["compatibility"], &v["createLayers"]), (&json!("1.5"), &json!(true)));
    // What is asked for explicitly is checked, not changed.
    for p in [
        json!({"standard": "pdfX4", "compatibility": "1.7"}),
        json!({"standard": "pdfX3", "compatibility": "1.6"}),
        json!({"standard": "pdfX1a", "createLayers": true}),
        json!({"standard": "pdfX3", "preserveEditing": true}),
        json!({"standard": "pdfX4", "security": {"openPassword": "x"}}),
        json!({"standard": "pdfX1a", "security": {"permissionsPassword": "x"}}),
        json!({"standard": "pdfX1a", "output": {"outputIntent": vectorcraft_color::cms::SRGB}}),
        json!({"standard": "pdfX4", "output": {"outputIntent": "No Such Press"}}),
    ] {
        let e = s.execute("document.exportPdf", &p).unwrap_err();
        assert!(matches!(e, crate::EngineError::BadParams { .. }), "{p}: {e}");
    }
    // The built-in PDF/X presets export as they are.
    for preset in ["PDF/X-1a:2001", "PDF/X-3:2002", "PDF/X-4:2010"] {
        assert!(s.execute("document.exportPdf", &json!({"preset": preset})).is_ok(), "{preset}");
    }
}

/// A PDF/X file without the bleed the document sets exports as asked and warns; the presets and
/// options that take the document's bleed, and files that aren't PDF/X files, don't warn.
#[test]
fn a_pdfx_file_that_leaves_out_the_document_bleed_warns() {
    let left_out = |w: &Vec<String>| w.iter().any(|w| w == super::pdf::BLEED_LEFT_OUT);
    let mut s = session();
    s.execute("document.setup", &json!({"bleed": 6})).unwrap();
    for standard in ["pdfX1a", "pdfX3", "pdfX4"] {
        let (_, warnings) = pdf(&mut s, json!({"standard": standard}));
        // First: the status bar shows the first warning, and PDF/X-1a and PDF/X-3 add the flattening note.
        assert_eq!(warnings.first().map(String::as_str), Some(super::pdf::BLEED_LEFT_OUT), "{standard}: {warnings:?}");
        // The page has no bleed, as asked.
        let v = s.execute("document.exportPdf", &json!({"standard": standard})).unwrap();
        let info = s.execute("document.pdfInfo", &json!({"name": "x.pdf", "dataBase64": v["dataBase64"]})).unwrap();
        let boxes = &info["pageInfo"][0]["boxes"];
        assert!(boxes["media"].is_array() && boxes["media"] == boxes["trim"] && boxes["media"] == boxes["bleed"], "{standard}: {info}");
    }
    let v = s.execute("document.export", &json!({"format": "pdf", "standard": "pdfX4"})).unwrap();
    assert!(v["warnings"].as_array().unwrap().iter().any(|w| w == super::pdf::BLEED_LEFT_OUT), "{v}");
    // A side the settings leave out warns, and sides they all keep don't.
    let (_, warnings) = pdf(&mut s, json!({"standard": "pdfX4", "bleed": {"top": 6, "bottom": 6, "left": 6}}));
    assert!(left_out(&warnings), "{warnings:?}");
    for p in [
        json!({"preset": "PDF/X-1a:2001"}),
        json!({"preset": "PDF/X-3:2002"}),
        json!({"preset": "PDF/X-4:2010"}),
        json!({"standard": "pdfX4", "bleed": {"useDocument": true}}),
        json!({"standard": "pdfX4", "bleed": {"top": 1, "bottom": 1, "left": 1, "right": 1}}),
        json!({}),
        json!({"standard": "pdfA2b"}),
    ] {
        let (_, warnings) = pdf(&mut s, p.clone());
        assert!(!left_out(&warnings), "{p}: {warnings:?}");
    }
    // The dialog's Summary exports in memory, so it lists the warning too.
    let v = s.execute("document.pdfSettings", &json!({"standard": "pdfX4", "includeDocument": true})).unwrap();
    assert!(v["warnings"].as_array().unwrap().iter().any(|w| w == super::pdf::BLEED_LEFT_OUT), "{v}");
    // A bleed on the top alone: only that side counts.
    s.execute("document.setup", &json!({"bleed": [6, 0, 0, 0]})).unwrap();
    let (_, warnings) = pdf(&mut s, json!({"standard": "pdfX4", "bleed": {"top": 1}}));
    assert!(!left_out(&warnings), "{warnings:?}");
    let (_, warnings) = pdf(&mut s, json!({"standard": "pdfX4", "bleed": {"bottom": 1}}));
    assert!(left_out(&warnings), "{warnings:?}");
    // A document without a bleed has nothing to leave out.
    s.execute("document.setup", &json!({"bleed": 0})).unwrap();
    let (_, warnings) = pdf(&mut s, json!({"standard": "pdfX4"}));
    assert!(!left_out(&warnings), "{warnings:?}");
}

/// The ink amounts (0..1) `doc` shows at `p` in its images (the topmost image there).
fn image_inks_at(doc: &vectorcraft_doc::Document, p: vectorcraft_geom::Point) -> Option<[f32; 4]> {
    fn visit(n: &vectorcraft_doc::Node, out: &mut Vec<vectorcraft_doc::ImageObject>) {
        if let vectorcraft_doc::NodeKind::Image(im) = &n.kind {
            out.push((**im).clone());
        }
        for c in n.children().into_iter().flatten() {
            visit(c, out);
        }
    }
    let mut images = vec![];
    for l in &doc.layers {
        visit(l, &mut images);
    }
    images.iter().rev().find_map(|im| {
        let q = im.xf.inverse() * p;
        if q.x < 0.0 || q.y < 0.0 || q.x >= im.width as f64 || q.y >= im.height as f64 {
            return None;
        }
        let ink = doc.images.get(&im.key)?.sample_cmyk_object(q.x, q.y, (im.width, im.height), 1)?;
        Some(ink.map(|v| v as f32 / 255.0))
    })
}

#[test]
fn pdfx_1a_flattening_keeps_the_inks_behind_an_image_in_a_cmyk_document() {
    // #2160: a CMYK panel under a PNG with see-through pixels. The flattener rasterizes where the
    // image reaches; the panel there keeps its ink numbers instead of going through RGB.
    let _g = crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "colorMode": "cmyk"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 180, "height": 80})).unwrap();
    let teal = [1.0, 0.06, 0.41, 0.03];
    s.execute("paint.setFill", &json!({"color": {"c": teal[0], "m": teal[1], "y": teal[2], "k": teal[3]}})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let mut png = vec![];
    image::RgbaImage::from_fn(40, 40, |x, _| image::Rgba([0, 120, 255, if x < 20 { 0 } else { 255 }]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    s.execute("file.place", &json!({"name": "logos.png", "dataBase64": vectorcraft_format::base64_encode(&png), "at": [100, 50]})).unwrap();
    let v = s.execute("document.exportPdf", &json!({"standard": "pdfX1a"})).unwrap();
    let bytes = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
    let back = vectorcraft_pdf::import(&bytes).unwrap();
    // Behind the image's see-through half: the panel's own inks, K included.
    let ink = image_inks_at(&back, vectorcraft_geom::Point::new(88.0, 50.0)).expect("a CMYK image behind the see-through pixels");
    assert!(ink.iter().zip(teal).all(|(a, b)| (a - b).abs() <= 1.5 / 255.0), "{ink:?} is not {teal:?}");
}
