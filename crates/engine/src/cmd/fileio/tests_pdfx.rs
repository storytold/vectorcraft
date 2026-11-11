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

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// The art a clip group crops in [`cropped`].
#[derive(Clone, Copy)]
pub(super) enum Art {
    /// A 240 × 160 image, with see-through pixels or without.
    Image { see_through: bool },
    /// A rectangle with a gradient fill.
    Gradient,
    /// A rectangle with a drop shadow.
    Shadow,
}

/// Where [`cropped`] puts the cropped art, [x, y, width, height].
const FRAME: [f64; 4] = [40.0, 40.0, 240.0, 160.0];
/// The part of [`FRAME`] the clipping path of [`cropped`] shows.
pub(super) const CROP: [f64; 4] = [50.0, 50.0, 80.0, 60.0];

/// A `mode` document whose first artboard is `width` wide, with a dark square and, over it, `art`
/// filling [`FRAME`] in a clip group that shows [`CROP`] of it. The square lies inside the frame
/// and outside the crop: only the hidden part of the art covers it. → the session with the clip
/// group selected, and the group's id.
pub(super) fn cropped(mode: &str, width: f64, art: Art) -> (Session, u64) {
    cropped_by(mode, width, art, false)
}

/// [`cropped`], with the art and its clipping path in a clipping layer above the square's layer
/// when `layer`. → the session and the clip group's or the layer's id.
fn cropped_by(mode: &str, width: f64, art: Art, layer: bool) -> (Session, u64) {
    let [x, y, w, h] = FRAME;
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": width, "height": 300, "colorMode": mode}));
    run(&mut s, "shape.rectangle", json!({"x": 180, "y": 90, "width": 60, "height": 60}));
    run(&mut s, "paint.setFill", json!({"color": "#203040"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    let layer = layer.then(|| run(&mut s, "layer.new", json!({}))["id"].as_u64().unwrap());
    let art = match art {
        Art::Image { see_through } => {
            let mut png = vec![];
            image::RgbaImage::from_fn(240, 160, |x, y| {
                image::Rgba([200, (x % 256) as u8, (y % 256) as u8, if see_through && x < 60 { 128 } else { 255 }])
            })
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
            let placed =
                run(&mut s, "file.place", json!({"name": "photo.png", "dataBase64": vectorcraft_format::base64_encode(&png), "rect": FRAME}));
            placed["ids"][0].clone()
        }
        Art::Gradient | Art::Shadow => {
            let id = run(&mut s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h}))["id"].clone();
            run(&mut s, "paint.setStroke", json!({"none": true}));
            if matches!(art, Art::Gradient) {
                let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
                run(&mut s, "paint.setFill", json!({"gradient": {"kind": "linear", "stops": stops}}));
            } else {
                run(&mut s, "paint.setFill", json!({"color": "#c04020"}));
                run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "ids": [id]}));
            }
            id
        }
    };
    let [x, y, w, h] = CROP;
    let clip = run(&mut s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h}))["id"].clone();
    if let Some(layer) = layer {
        run(&mut s, "layer.clippingMask.toggle", json!({"id": layer}));
        run(&mut s, "select.set", json!({"ids": [art]}));
        return (s, layer);
    }
    run(&mut s, "select.set", json!({"ids": [art, clip]}));
    let group = run(&mut s, "object.clippingMask.make", json!({}))["id"].as_u64().unwrap();
    (s, group)
}

/// `document.exportPdf` with `p`, read back.
fn read_back(s: &mut Session, p: &Value) -> vectorcraft_doc::Document {
    let v = run(s, "document.exportPdf", p.clone());
    vectorcraft_pdf::import(&vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()).unwrap()
}

/// `doc`'s artboards drawn on white at 72 ppi.
pub(super) fn drawn(doc: &vectorcraft_doc::Document) -> vectorcraft_render::Rendered {
    let pages = doc.artboards.iter().map(|a| a.rect).reduce(|a, b| a.union(b)).unwrap();
    vectorcraft_render::Renderer::new().render_region(doc, pages, 1.0, true)
}

/// How many pixels show art in `reference` and are blank in `variant`.
pub(super) fn lost(reference: &vectorcraft_render::Rendered, variant: &vectorcraft_render::Rendered) -> usize {
    assert_eq!((reference.width, reference.height), (variant.width, variant.height));
    let (reference, variant) = (reference.pixels.as_chunks::<4>().0, variant.pixels.as_chunks::<4>().0);
    reference.iter().zip(variant).filter(|(r, v)| r[..3].iter().any(|c| *c < 200) && v[..3].iter().all(|c| *c >= 250)).count()
}

/// How many pixels of `a` and `b` differ by more than 24 in a channel.
fn differ(a: &vectorcraft_render::Rendered, b: &vectorcraft_render::Rendered) -> usize {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let (a, b) = (a.pixels.as_chunks::<4>().0, b.pixels.as_chunks::<4>().0);
    a.iter().zip(b).filter(|(p, q)| p[..3].iter().zip(&q[..3]).any(|(x, y)| x.abs_diff(*y) > 24)).count()
}

/// The width and height of what each of `doc`'s images shows: its frame inside the clipping
/// paths around it.
pub(super) fn shown_image_sizes(doc: &vectorcraft_doc::Document) -> Vec<(f64, f64)> {
    use vectorcraft_geom::Rect;
    fn visit(n: &vectorcraft_doc::Node, clip: Option<Rect>, out: &mut Vec<(f64, f64)>) {
        let within = |r: Rect| clip.map_or(r, |c| c.intersect(r));
        if let vectorcraft_doc::NodeKind::Image(im) = &n.kind {
            let frame = im.xf.transform_rect_bbox(Rect::new(0.0, 0.0, im.width as f64, im.height as f64));
            let shown = within(frame);
            out.push((shown.width(), shown.height()));
        }
        let clip = if n.clips() { n.visual_bounds().map(within) } else { clip };
        for c in n.children().into_iter().flatten() {
            visit(c, clip, out);
        }
    }
    let mut out = vec![];
    for l in &doc.layers {
        visit(l, None, &mut out);
    }
    out
}

/// Checks that the PDF/X-1a, PDF/X-3 and PDF 1.3 files of `s` (with `extra` options) show all the
/// art the file with transparency shows, and that no image in them shows more than [`CROP`]. Adds
/// what is wrong to `wrong`.
fn check_flat_files(s: &mut Session, extra: &Value, what: &str, wrong: &mut Vec<String>) {
    let reference = drawn(&read_back(s, extra));
    for flat in [json!({"standard": "pdfX1a"}), json!({"standard": "pdfX3"}), json!({"compatibility": "1.3", "preserveEditing": false})] {
        let mut p = flat.clone();
        super::pdf::merge(&mut p, extra);
        let back = read_back(s, &p);
        let lost = lost(&reference, &drawn(&back));
        if lost > 0 {
            wrong.push(format!("{what}, {flat}: {lost} pixels of art went missing"));
        }
        let sizes = shown_image_sizes(&back);
        // An image reaches a pixel past its regions, and a group rasterized whole a point past
        // its visible bounds.
        if sizes.is_empty() || sizes.iter().any(|(w, h)| *w > CROP[2] + 3.0 || *h > CROP[3] + 3.0) {
            wrong.push(format!("{what}, {flat}: the images show {sizes:?}"));
        }
    }
}

/// Art cropped by a clipping path, in a PDF file without transparency: the part of the art the
/// clipping path hides is not rasterized, so art that lies under the hidden part alone still
/// shows. Each case is one way the cropped art comes to be flattened.
#[test]
fn flattening_cropped_art_for_pdfx_keeps_the_art_under_its_hidden_part() {
    let _g = crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let opaque = Art::Image { see_through: false };
    type Change = fn(&mut Session, u64);
    let cases: [(&str, Art, Change); 9] = [
        ("an image with see-through pixels", Art::Image { see_through: true }, |_, _| {}),
        ("a clip group with opacity", opaque, |s, _| {
            run(s, "transparency.set", json!({"opacity": 80}));
        }),
        ("a clip group with a blend mode", opaque, |s, _| {
            run(s, "transparency.set", json!({"blendMode": "multiply"}));
        }),
        ("a see-through object over the crop", opaque, |s, _| {
            run(s, "shape.rectangle", json!({"x": 60, "y": 60, "width": 40, "height": 30}));
            run(s, "paint.setFill", json!({"color": "#ffffff"}));
            run(s, "transparency.set", json!({"opacity": 60}));
        }),
        ("a gradient", Art::Gradient, |s, _| {
            run(s, "transparency.set", json!({"opacity": 80}));
        }),
        ("a raster effect", Art::Shadow, |_, _| {}),
        ("a clip group inside a clip group", Art::Image { see_through: true }, |s, group| {
            let outer = run(s, "shape.rectangle", json!({"x": 45, "y": 45, "width": 85, "height": 65}))["id"].clone();
            run(s, "select.set", json!({"ids": [group, outer]}));
            run(s, "object.clippingMask.make", json!({}));
        }),
        ("a clip group inside a group with opacity", opaque, |s, _| {
            run(s, "object.group", json!({}));
            run(s, "transparency.set", json!({"opacity": 80}));
        }),
        ("an even-odd compound clipping path", opaque, |s, group| {
            run(s, "transparency.set", json!({"opacity": 80}));
            let doc = s.doc_mut().unwrap();
            let mut d = (*doc.doc).clone();
            let clip = d.node(vectorcraft_doc::NodeId(group)).unwrap().children().unwrap()[0].id;
            let mut ring = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(50.0, 50.0, 130.0, 110.0));
            ring.subpaths.extend(vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(70.0, 65.0, 110.0, 95.0)).subpaths);
            if let vectorcraft_doc::NodeKind::Path { path, rule, .. } = &mut d.node_mut(clip).unwrap().kind {
                (*path, *rule) = (ring, vectorcraft_geom::FillRule::EvenOdd);
            }
            doc.doc = d.into();
        }),
    ];
    let mut wrong = vec![];
    for mode in ["cmyk", "rgb"] {
        for (what, art, change) in cases {
            let (mut s, group) = cropped(mode, 400.0, art);
            change(&mut s, group);
            check_flat_files(&mut s, &json!({}), &format!("{mode}: {what}"), &mut wrong);
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The same with the Medium and Low Resolution presets, and with a balance of 0, which rasterizes
/// the whole group of the cropped art over the group's visible bounds.
#[test]
fn flattening_cropped_art_for_pdfx_keeps_the_art_under_its_hidden_part_with_each_preset() {
    let _g = crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut wrong = vec![];
    for extra in [json!({"flattenerPreset": "medium"}), json!({"flattenerPreset": "low"}), json!({"flattener": {"balance": 0}})] {
        for (art, name) in [(Art::Image { see_through: true }, "image"), (Art::Gradient, "gradient")] {
            let (mut s, _) = cropped("cmyk", 400.0, art);
            run(&mut s, "transparency.set", json!({"opacity": 80}));
            check_flat_files(&mut s, &extra, &format!("{name}, {extra}"), &mut wrong);
        }
    }
    // A cropped rectangle with a solid fill, rasterized whole.
    let (mut s, group) = cropped("cmyk", 400.0, Art::Gradient);
    let art = s.doc().unwrap().doc.node(vectorcraft_doc::NodeId(group)).unwrap().children().unwrap()[1].id.0;
    run(&mut s, "paint.setFill", json!({"ids": [art], "color": "#c04020"}));
    run(&mut s, "transparency.set", json!({"ids": [group], "opacity": 80}));
    check_flat_files(&mut s, &json!({"flattener": {"balance": 0}}), "solid fill, balance 0", &mut wrong);
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The hidden part of cropped art on one artboard lies over the next artboard: that page keeps
/// its art.
#[test]
fn flattening_cropped_art_for_pdfx_keeps_the_next_page_under_its_hidden_part() {
    let _g = crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    // The crop is on the first artboard, and the square on the second.
    let (mut s, _) = cropped("cmyk", 150.0, Art::Image { see_through: true });
    run(&mut s, "artboard.new", json!({"x": 170, "y": 0, "width": 230, "height": 300}));
    assert_eq!(read_back(&mut s, &json!({"standard": "pdfX1a"})).artboards.len(), 2);
    let mut wrong = vec![];
    check_flat_files(&mut s, &json!({}), "two artboards", &mut wrong);
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Art cropped by a clipping layer, over a square in the layer below: the square keeps showing,
/// also with a second square in that layer under both the crop and the hidden part, or with a
/// see-through rectangle over both in a layer above.
#[test]
fn flattening_art_of_a_clipping_layer_for_pdfx_keeps_the_art_under_its_hidden_part() {
    let _g = crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    type Change = fn(&mut Session, u64);
    let cases: [(&str, Change); 3] = [
        ("alone", |_, _| {}),
        ("over a square under the crop's edge", |s, _| {
            let below = s.doc().unwrap().doc.layers[0].id.0;
            run(s, "layer.setCurrent", json!({"id": below}));
            run(s, "shape.rectangle", json!({"x": 110, "y": 60, "width": 50, "height": 30}));
            run(s, "paint.setFill", json!({"color": "#406020"}));
        }),
        ("under a see-through rectangle over the crop's edge", |s, _| {
            run(s, "layer.new", json!({}));
            run(s, "shape.rectangle", json!({"x": 110, "y": 60, "width": 50, "height": 30}));
            run(s, "paint.setFill", json!({"color": "#406020"}));
            run(s, "transparency.set", json!({"opacity": 60}));
        }),
    ];
    let mut wrong = vec![];
    for mode in ["cmyk", "rgb"] {
        for art in [Art::Image { see_through: true }, Art::Gradient] {
            for (what, change) in cases {
                let (mut s, layer) = cropped_by(mode, 400.0, art, true);
                run(&mut s, "transparency.set", json!({"opacity": 80}));
                change(&mut s, layer);
                check_flat_files(&mut s, &json!({}), &format!("{mode}: a clipping layer {what}"), &mut wrong);
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Documents in which no clip group hides art export the same bytes as they did when the
/// flattener rasterized hidden art too.
#[test]
fn flat_pdf_files_of_art_that_no_clip_group_hides_are_unchanged() {
    let _g = crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    use crate::cmd::flatten::with_clips_ignored as before;
    // Written at one time, so two files differ only by what they draw.
    let bytes = |s: &mut Session, p: &Value| super::pdf::encode_at("document.exportPdf", &s.doc().unwrap().doc, p, Some(1_700_000_000)).unwrap().0;
    let files = [
        json!({"standard": "pdfX1a"}),
        json!({"standard": "pdfX3"}),
        json!({"compatibility": "1.3", "preserveEditing": false}),
        json!({"standard": "pdfX1a", "flattener": {"balance": 0}}),
        json!({"standard": "pdfX1a", "flattenerPreset": "low"}),
    ];
    // Transparency without a clip group.
    let mut plain = session();
    // A clip group at 80% whose clipping path is as large as its art, over the edge of a square.
    let mut fitted = Session::new();
    run(&mut fitted, "file.new", json!({"width": 400, "height": 300, "colorMode": "cmyk"}));
    run(&mut fitted, "shape.rectangle", json!({"x": 180, "y": 90, "width": 160, "height": 160}));
    run(&mut fitted, "paint.setFill", json!({"color": "#203040"}));
    let mut png = vec![];
    image::RgbaImage::from_fn(240, 160, |x, y| image::Rgba([200, x as u8, y as u8, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let photo =
        run(&mut fitted, "file.place", json!({"name": "photo.png", "dataBase64": vectorcraft_format::base64_encode(&png), "rect": FRAME}))["ids"][0]
            .clone();
    let [x, y, w, h] = FRAME;
    let clip = run(&mut fitted, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h}))["id"].clone();
    run(&mut fitted, "select.set", json!({"ids": [photo, clip]}));
    run(&mut fitted, "object.clippingMask.make", json!({}));
    run(&mut fitted, "transparency.set", json!({"opacity": 80}));
    // Cropped art that is not flattened: nothing on it is see-through.
    let (mut opaque, _) = cropped("cmyk", 400.0, Art::Image { see_through: false });
    for (what, s) in [("no clip group", &mut plain), ("a clipping path as large as its art", &mut fitted), ("opaque cropped art", &mut opaque)] {
        for p in &files {
            assert!(bytes(s, p) == before(|| bytes(s, p)), "{what}, {p}");
        }
    }
    // Art that a clip group hides is what changes.
    let (mut s, _) = cropped("cmyk", 400.0, Art::Image { see_through: true });
    assert!(bytes(&mut s, &files[0]) != before(|| bytes(&mut s, &files[0])));
}

/// The options of the PDF files without transparency.
const FLAT_FILES: [&str; 3] = [r#"{"standard": "pdfX1a"}"#, r#"{"standard": "pdfX3"}"#, r#"{"compatibility": "1.3", "preserveEditing": false}"#];

/// [`cropped_by`]'s clipping layer of a gradient at 80%, with a rectangle in the layer below that
/// crosses the crop's right edge. → the session and the clipping layer's id.
fn clipping_layer_over_a_rectangle(mode: &str) -> (Session, u64) {
    let (mut s, layer) = cropped_by(mode, 400.0, Art::Gradient, true);
    run(&mut s, "transparency.set", json!({"opacity": 80}));
    let below = s.doc().unwrap().doc.layers[0].id.0;
    run(&mut s, "layer.setCurrent", json!({"id": below}));
    run(&mut s, "shape.rectangle", json!({"x": 100, "y": 60, "width": 120, "height": 40}));
    run(&mut s, "paint.setFill", json!({"color": "#406020"}));
    (s, layer)
}

/// A clipping layer over a layer that doesn't print, and a clipping layer that doesn't print
/// over one that does: the files without transparency print what the file with transparency
/// prints, and nothing of the layer that doesn't print.
#[test]
fn flattening_for_pdfx_leaves_out_the_layers_that_do_not_print() {
    let _g = crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut wrong = vec![];
    for mode in ["cmyk", "rgb"] {
        for clipping_layer_prints in [true, false] {
            let (mut s, layer) = clipping_layer_over_a_rectangle(mode);
            let below = s.doc().unwrap().doc.layers[0].id.0;
            let off = if clipping_layer_prints { below } else { layer };
            run(&mut s, "layer.setProps", json!({"id": off, "printable": false}));
            let reference = drawn(&read_back(&mut s, &json!({})));
            for flat in FLAT_FILES {
                let back = drawn(&read_back(&mut s, &serde_json::from_str(flat).unwrap()));
                let (lost, gained) = (lost(&reference, &back), lost(&back, &reference));
                if lost + gained > 0 {
                    wrong.push(format!("{mode}, clipping layer prints: {clipping_layer_prints}, {flat}: {lost} pixels lost, {gained} gained"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A clipping layer whose clipping path has a fill: the files without transparency show the art
/// over the fill, not the fill over the art.
#[test]
fn flattening_for_pdfx_keeps_the_art_of_a_clipping_layer_over_its_clipping_path_fill() {
    let _g = crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut wrong = vec![];
    for mode in ["cmyk", "rgb"] {
        let (mut s, layer) = clipping_layer_over_a_rectangle(mode);
        let clip = s.doc().unwrap().doc.node(vectorcraft_doc::NodeId(layer)).unwrap().children().unwrap()[0].id.0;
        run(&mut s, "paint.setFill", json!({"ids": [clip], "color": "#ffdd00"}));
        let reference = drawn(&read_back(&mut s, &json!({})));
        for flat in FLAT_FILES {
            let back = drawn(&read_back(&mut s, &serde_json::from_str(flat).unwrap()));
            // PDF/X-1a converts an RGB document's colors to CMYK.
            let converted = mode == "rgb" && flat.contains("pdfX1a");
            let (lost, differ) = (lost(&reference, &back), if converted { 0 } else { differ(&reference, &back) });
            if lost > 0 || differ > 100 {
                wrong.push(format!("{mode}, {flat}: {lost} pixels of art lost, {differ} pixels differ"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Transparent art in a clipping layer whose clipping path is area type holding only a space,
/// which hides it: the files without transparency are written, without the art.
#[test]
fn flattening_for_pdfx_removes_transparent_art_that_a_clipping_layer_hides_entirely() {
    let _g = crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let [x, y, w, h] = FRAME;
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300, "colorMode": "cmyk"}));
    let layer = run(&mut s, "layer.new", json!({}))["id"].as_u64().unwrap();
    run(&mut s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h}));
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
    run(&mut s, "paint.setFill", json!({"gradient": {"kind": "linear", "stops": stops}}));
    run(&mut s, "transparency.set", json!({"opacity": 80}));
    run(&mut s, "text.create", json!({"x": 50, "y": 50, "text": " ", "area": {"width": 80, "height": 60}}));
    run(&mut s, "layer.clippingMask.toggle", json!({"id": layer}));
    let reference = drawn(&read_back(&mut s, &json!({})));
    for flat in FLAT_FILES {
        let p: Value = serde_json::from_str(flat).unwrap();
        let back = match s.execute("document.exportPdf", &p) {
            Ok(v) => vectorcraft_pdf::import(&vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()).unwrap(),
            Err(e) => panic!("{flat}: {e}"),
        };
        assert_eq!(lost(&drawn(&back), &reference), 0, "{flat}: the hidden art prints");
    }
}

/// Object › Flatten Transparency of art in a clipping layer and art in the layer below that
/// reaches past the clipping path: when the layer below doesn't print, the clipping layer has
/// opacity, or the clipping path has a fill (see-through too, flattened keeping alpha, in a layer
/// with opacity or Multiply mode, or
/// over a layer with opacity, overlapping art of a layer with opacity or art an opacity mask
/// hides), the group stays in the clipping layer, the art below stays in its layer, and the page
/// looks the same.
#[test]
fn flattening_keeps_the_group_in_the_clipping_layer_when_the_layer_below_differs() {
    let mut wrong = vec![];
    let cases = [
        "the layer below doesn't print",
        "the clipping path has a fill",
        "the clipping path has a see-through fill, keeping alpha",
        "the clipping path has a see-through fill, in a layer with opacity",
        "the clipping path has a see-through fill, in a layer in Multiply mode",
        "the clipping path has a see-through fill, over a layer with opacity",
        "the clipping path has a see-through fill, over art that an opacity mask hides",
        "the clipping path has a see-through fill, over overlapping art of a layer at 50%",
        "the clipping path has no fill, in a layer with opacity",
    ];
    for case in cases {
        let mut s = Session::new();
        run(&mut s, "file.new", json!({"width": 400, "height": 300, "colorMode": "rgb"}));
        let below = s.doc().unwrap().doc.layers[0].id.0;
        let orange = run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 200, "height": 150}))["id"].clone();
        run(&mut s, "paint.setFill", json!({"color": "#ff8000"}));
        let upper = run(&mut s, "layer.new", json!({}))["id"].as_u64().unwrap();
        let blue = run(&mut s, "shape.rectangle", json!({"x": 100, "y": 80, "width": 200, "height": 150}))["id"].clone();
        run(&mut s, "paint.setFill", json!({"color": "#2040c0"}));
        run(&mut s, "transparency.set", json!({"opacity": 50}));
        let clip = run(&mut s, "shape.rectangle", json!({"x": 60, "y": 40, "width": 300, "height": 220}))["id"].clone();
        run(&mut s, "layer.clippingMask.toggle", json!({"id": upper}));
        if case.starts_with("the layer below") {
            run(&mut s, "layer.setProps", json!({"id": below, "printable": false}));
        } else if !case.contains("no fill") {
            run(&mut s, "paint.setFill", json!({"ids": [clip], "color": "#88ffcc"}));
        }
        let keeping_alpha = case.ends_with("keeping alpha");
        if case.contains("see-through fill") {
            run(&mut s, "transparency.set", json!({"ids": [clip], "opacity": 50}));
        }
        if case.ends_with("in a layer with opacity") {
            run(&mut s, "transparency.set", json!({"ids": [upper], "opacity": 80}));
        }
        if case.ends_with("over a layer with opacity") {
            run(&mut s, "transparency.set", json!({"ids": [below], "opacity": 50}));
        }
        if case.ends_with("an opacity mask hides") {
            run(&mut s, "layer.setCurrent", json!({"id": below}));
            run(&mut s, "select.set", json!({"ids": [orange]}));
            let group = run(&mut s, "object.group", json!({}))["id"].clone();
            let mask = run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 200, "height": 150}))["id"].clone();
            run(&mut s, "paint.setFill", json!({"color": "#000000"}));
            run(&mut s, "select.set", json!({"ids": [group, mask]}));
            run(&mut s, "transparency.makeOpacityMask", json!({}));
        }
        if case.ends_with("Multiply mode") {
            run(&mut s, "transparency.set", json!({"ids": [upper], "blendMode": "multiply"}));
        }
        let mut art = vec![orange.clone()];
        if case.ends_with("a layer at 50%") {
            run(&mut s, "layer.setCurrent", json!({"id": below}));
            art.push(run(&mut s, "shape.rectangle", json!({"x": 80, "y": 60, "width": 120, "height": 120}))["id"].clone());
            run(&mut s, "paint.setFill", json!({"color": "#20a040"}));
            run(&mut s, "transparency.set", json!({"ids": [below], "opacity": 50}));
        }
        art.push(blue.clone());
        // The page as it prints.
        let printed = |d: &vectorcraft_doc::Document| {
            let mut d = d.clone();
            if case.starts_with("the layer below") {
                let mut layer = (*d.layers[0]).clone();
                layer.visible = false;
                d.layers[0] = layer.into();
            }
            drawn(&d)
        };
        let before = printed(&s.doc().unwrap().doc);
        run(&mut s, "select.set", json!({"ids": art}));
        run(&mut s, "object.flattenTransparency", json!({"preserveAlpha": keeping_alpha}));
        let d = &s.doc().unwrap().doc;
        let layer_of = |id: &Value| id.as_u64().and_then(|id| d.ancestry(vectorcraft_doc::NodeId(id))).map(|a| a[0].0);
        if layer_of(&orange) != Some(below) {
            wrong.push(format!("{case}: the art below is in layer {:?}", layer_of(&orange)));
        }
        let upper_art = d.node(vectorcraft_doc::NodeId(upper)).and_then(|l| l.children()).map_or(0, Vec::len);
        if d.node(vectorcraft_doc::NodeId(blue.as_u64().unwrap())).is_some() || upper_art != 2 {
            wrong.push(format!("{case}: the clipping layer holds {upper_art} objects"));
        }
        let after = printed(d);
        // Flat regions differ from see-through art along their antialiased edges.
        let (lost, differ) = (lost(&before, &after), differ(&before, &after));
        if lost > 0 || differ > 1200 {
            wrong.push(format!("{case}: {lost} pixels of art lost, {differ} pixels differ"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
