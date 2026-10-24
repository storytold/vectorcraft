//! Raster export options: resolution (stored in the file), background, anti-aliasing and
//! interlacing, also on Export for Screens and Rasterize.

use serde_json::{Value, json};

use super::*;

fn session(width: f64, height: f64, artboards: usize) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": width, "height": height, "artboards": artboards})).unwrap();
    s
}

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

fn rgba(bytes: &[u8]) -> image::RgbaImage {
    image::load_from_memory(bytes).unwrap().to_rgba8()
}

/// A chunk's data by type (first match) in a PNG file.
fn png_chunk<'a>(file: &'a [u8], ty: &[u8; 4]) -> Option<&'a [u8]> {
    let mut i = 8;
    while i + 8 <= file.len() {
        let len = u32::from_be_bytes(file[i..i + 4].try_into().unwrap()) as usize;
        if &file[i + 4..i + 8] == ty {
            return Some(&file[i + 8..i + 8 + len]);
        }
        i += 12 + len;
    }
    None
}

#[test]
fn png_options_resolution_background_interlace_and_anti_aliasing() {
    let mut s = session(40.0, 30.0, 1);
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 5.3, "y": 4.7, "width": 25.8, "height": 21.5})).unwrap();
    let png = |s: &mut Session, p: Value| b64(&s.execute("document.export", &p).unwrap());

    let hi = png(&mut s, json!({"format": "png", "ppi": 300}));
    let ppm = (300.0f64 / 0.0254).round() as u32;
    assert_eq!(&png_chunk(&hi, b"pHYs").unwrap()[..4], ppm.to_be_bytes(), "pHYs = ppi / 0.0254");
    assert_eq!(rgba(&png(&mut s, json!({"format": "png", "ppi": 144}))).dimensions(), (80, 60), "144 ppi doubles the size");
    // scale alone stores 72 × scale.
    let scaled = png(&mut s, json!({"format": "png", "scale": 2}));
    assert_eq!(&png_chunk(&scaled, b"pHYs").unwrap()[..4], ((144.0f64 / 0.0254).round() as u32).to_be_bytes());

    let white = rgba(&png(&mut s, json!({"format": "png", "background": "white"})));
    assert!(white.pixels().all(|p| p[3] == 255), "a white background has no alpha below 255");
    assert_eq!(white.get_pixel(0, 0).0, [255, 255, 255, 255]);
    assert_eq!(rgba(&png(&mut s, json!({"format": "png", "background": "black"}))).get_pixel(0, 0).0, [0, 0, 0, 255]);
    assert_eq!(rgba(&png(&mut s, json!({"format": "png", "background": "#3366cc"}))).get_pixel(0, 0).0, [0x33, 0x66, 0xcc, 255]);
    assert_eq!(rgba(&png(&mut s, json!({"format": "png"}))).get_pixel(0, 0)[3], 0, "transparent by default");
    assert_eq!(rgba(&png(&mut s, json!({"format": "jpg", "background": "black"}))).get_pixel(0, 0)[0], 0);

    let laced = png(&mut s, json!({"format": "png", "interlaced": true}));
    assert_eq!(laced[28], 1, "IHDR interlace method: Adam7");
    assert_eq!(rgba(&laced), rgba(&png(&mut s, json!({"format": "png"}))), "the interlaced file decodes to the same image");

    let hard = rgba(&png(&mut s, json!({"format": "png", "antiAlias": "none"})));
    assert!(hard.pixels().all(|p| p[3] == 0 || p[3] == 255), "no anti-aliasing: alpha is 0 or 255");
    let soft = rgba(&png(&mut s, json!({"format": "png", "antiAlias": "art"})));
    assert!(soft.pixels().any(|p| p[3] != 0 && p[3] != 255));
    for bad in [
        json!({"antiAlias": "smooth"}),
        json!({"background": "plaid"}),
        json!({"interlaced": "yes"}),
        json!({"ppi": "high"}),
        json!({"ppi": 0}),
        json!({"ppi": -72}),
    ] {
        let mut p = bad.clone();
        p["format"] = json!("png");
        assert!(s.execute("document.export", &p).is_err(), "{bad}");
    }
}

#[test]
fn screens_and_rasterize_take_anti_aliasing() {
    let mut s = session(40.0, 30.0, 1);
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let id = s.execute("shape.ellipse", &json!({"x": 5.3, "y": 4.7, "width": 25.8, "height": 21.5})).unwrap()["id"].as_u64().unwrap();
    let hard = |img: &image::RgbaImage| img.pixels().all(|p| p[3] == 0 || p[3] == 255);
    let r = s
        .execute(
            "document.exportForScreens",
            &json!({"antiAlias": "none", "formats": [{"format": "png"}, {"format": "png", "scale": 2, "antiAlias": "art"}]}),
        )
        .unwrap();
    assert!(hard(&rgba(&b64(&r["files"][0]))), "the top-level mode applies to rows without their own");
    assert!(!hard(&rgba(&b64(&r["files"][1]))), "a row's own mode wins");

    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("object.rasterize", &json!({"ppi": 72, "antiAlias": "none", "background": "transparent"})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let vectorcraft_doc::NodeKind::Image(im) = &doc.layers[0].children().unwrap()[0].kind else { panic!("rasterized to an image") };
    assert!(hard(&rgba(&doc.images[&im.key].bytes)));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("object.rasterize", &json!({"ppi": 72, "background": "black"})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let vectorcraft_doc::NodeKind::Image(im) = &doc.layers[0].children().unwrap()[0].kind else { panic!("rasterized to an image") };
    assert_eq!(rgba(&doc.images[&im.key].bytes).get_pixel(0, 0).0, [0, 0, 0, 255]);
    assert!(s.execute("object.rasterize", &json!({"antiAlias": "fuzzy"})).is_err());
}

/// Appearance of Black › Printing / Exporting (#394): Output All Blacks as Rich Black makes an
/// RGB PNG of a CMYK document draw pure black ink as RGB black; Accurately (the default) shows it
/// through the CMYK profile. `blackOnScreen` does not change the export; a rich black is the same
/// either way.
#[test]
fn black_output_rich_draws_pure_cmyk_black_as_rgb_black_in_png() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 20, "height": 20, "colorMode": "cmyk"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("paint.setFill", &json!({"color": {"c": 0, "m": 0, "y": 0, "k": 1}})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    let centre = |s: &mut Session| {
        let png = rgba(&b64(&s.execute("document.export", &json!({"format": "png", "background": "white"})).unwrap()));
        png.get_pixel(10, 10).0
    };
    let accurate = centre(&mut s);
    let shown = vectorcraft_color::Color::cmyk(0.0, 0.0, 0.0, 1.0).to_rgba8(1.0);
    assert!(accurate.iter().zip(shown).all(|(a, b)| a.abs_diff(b) <= 1), "accurate: {accurate:?}");
    assert!(accurate[0] > 10, "accurate: dark grey, not RGB black");
    s.execute("prefs.set", &json!({"key": "blackOnScreen", "value": "rich"})).unwrap();
    assert_eq!(centre(&mut s), accurate, "blackOnScreen does not change exports");
    s.execute("prefs.set", &json!({"key": "blackOutput", "value": "rich"})).unwrap();
    assert_eq!(centre(&mut s), [0, 0, 0, 255], "rich: RGB black");
    s.execute("select.all", &json!({})).unwrap();
    s.execute("paint.setFill", &json!({"color": {"c": 0.6, "m": 0.4, "y": 0.4, "k": 1}})).unwrap();
    let rich = centre(&mut s);
    s.execute("prefs.set", &json!({"key": "blackOutput", "value": "accurate"})).unwrap();
    assert_eq!(centre(&mut s), rich, "a rich black shows the same either way");
}

/// Output All Blacks as Rich Black reaches a CMYK document composed from ink planes (one with a
/// blending mode) as it does one drawn directly, and leaves CMYK-model exports alone: only RGB
/// output changes.
#[test]
fn black_output_rich_covers_ink_plane_documents_and_leaves_cmyk_exports_alone() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 40, "height": 20, "colorMode": "cmyk"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("paint.setFill", &json!({"color": {"c": 0, "m": 0, "y": 0, "k": 1}})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 30, "y": 0, "width": 10, "height": 20})).unwrap();
    s.execute("transparency.set", &json!({"blendMode": "multiply"})).unwrap();
    let png =
        |s: &mut Session| rgba(&b64(&s.execute("document.export", &json!({"format": "png", "background": "white"})).unwrap())).get_pixel(10, 10).0;
    let cmyk = |s: &mut Session, format: &str| b64(&s.execute("document.export", &json!({"format": format, "colorModel": "cmyk"})).unwrap());
    let accurate = png(&mut s);
    assert!(accurate[0] > 10, "accurate: dark grey, {accurate:?}");
    let inks = [cmyk(&mut s, "tiff"), cmyk(&mut s, "jpg")];
    s.execute("prefs.set", &json!({"key": "blackOutput", "value": "rich"})).unwrap();
    assert_eq!(png(&mut s), [0, 0, 0, 255], "rich: RGB black through the ink planes too");
    assert!(inks == [cmyk(&mut s, "tiff"), cmyk(&mut s, "jpg")], "CMYK exports keep the document's inks");
}

#[test]
fn png_says_it_is_srgb_unless_embed_icc_is_false() {
    let mut s = session(40.0, 30.0, 1);
    s.execute("shape.rectangle", &json!({"x": 5, "y": 5, "width": 20, "height": 10})).unwrap();
    let png = |s: &mut Session, p: Value| b64(&s.execute("document.export", &p).unwrap());
    let on = png(&mut s, json!({"format": "png"}));
    assert_eq!(png_chunk(&on, b"sRGB"), Some(&[0u8][..]), "an sRGB chunk, perceptual intent");
    let off = png(&mut s, json!({"format": "png", "embedIcc": false}));
    assert!(png_chunk(&off, b"sRGB").is_none());
    assert_eq!(rgba(&on), rgba(&off), "same pixels either way");
    let r = s.execute("document.formats", &json!({})).unwrap();
    let f = r["formats"].as_array().unwrap().iter().find(|f| f["id"] == "png").unwrap();
    assert!(f["options"].get("embedIcc").is_some(), "png lists embedIcc");
}

#[test]
fn formats_list_the_raster_options() {
    let mut s = Session::new();
    let r = s.execute("document.formats", &json!({})).unwrap();
    let opts = |id: &str| r["formats"].as_array().unwrap().iter().find(|f| f["id"] == id).unwrap()["options"].clone();
    for k in ["ppi", "scale", "background", "antiAlias", "interlaced"] {
        assert!(opts("png").get(k).is_some(), "png lists {k}");
    }
    assert_eq!(opts("png")["antiAlias"]["default"], "art");
    assert_eq!(opts("png")["ppi"]["default"], 72);
    assert!(opts("jpg").get("quality").is_some() && opts("jpg").get("interlaced").is_none());
    assert!(opts("webp").get("antiAlias").is_some() && opts("svg").get("antiAlias").is_none());
}

#[test]
fn screens_rows_with_a_resolution_name_their_scale() {
    let mut s = session(40.0, 30.0, 1);
    let r = s.execute("document.exportForScreens", &json!({"formats": [{"format": "png", "ppi": 144}, {"format": "svg", "ppi": 144}]})).unwrap();
    assert_eq!((r["files"][0]["name"].as_str(), r["files"][1]["name"].as_str()), (Some("Artboard-1@2x.png"), Some("Artboard-1.svg")), "{r}");
    assert_eq!(rgba(&b64(&r["files"][0])).dimensions(), (80, 60));
}

#[test]
fn a_300_ppi_export_places_back_at_its_physical_size() {
    let mut s = session(72.0, 36.0, 1);
    let png = b64(&s.execute("document.export", &json!({"format": "png", "ppi": 300, "background": "white"})).unwrap());
    assert_eq!(rgba(&png).dimensions(), (300, 150));
    assert_eq!(super::ppi::resolution(&png), Some((300.0, 300.0)), "the place reader reads the writer's pHYs");
    let r = s.execute("file.place", &json!({"name": "art.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
    assert!((r["width"].as_f64().unwrap() - 72.0).abs() < 1e-6 && (r["height"].as_f64().unwrap() - 36.0).abs() < 1e-6, "100%: {r}");
}

#[test]
fn the_document_background_is_the_default_export_background() {
    let mut s = session(40.0, 30.0, 1);
    s.execute("document.setup", &json!({"backgroundContents": "white"})).unwrap();
    let png = |s: &mut Session, p: Value| rgba(&b64(&s.execute("document.export", &p).unwrap()));
    assert_eq!(png(&mut s, json!({"format": "png"})).get_pixel(0, 0).0, [255, 255, 255, 255], "a white document exports opaque");
    assert_eq!(png(&mut s, json!({"format": "png", "background": "transparent"})).get_pixel(0, 0)[3], 0, "the option wins");
    assert_eq!(png(&mut s, json!({"format": "webp", "background": "black"})).get_pixel(0, 0).0, [0, 0, 0, 255]);
}

#[test]
fn an_image_opens_at_the_size_its_resolution_declares() {
    let mut s = session(72.0, 36.0, 1);
    let hi = b64(&s.execute("document.export", &json!({"format": "png", "ppi": 300, "background": "white"})).unwrap());
    let lo = b64(&s.execute("document.export", &json!({"format": "png", "background": "white"})).unwrap());
    let lo = super::ppi::with_png_resolution(&lo, (72.0, 72.0));
    for (png, w, h, what) in [(hi, 72.0, 36.0, "300 ppi: 300 x 150 px is 1 x 0.5 in"), (lo, 72.0, 36.0, "72 ppi: 1 px = 1 pt")] {
        s.execute("document.open", &json!({"name": "art.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
        let doc = &s.doc().unwrap().doc;
        let ab = doc.artboards[0].rect;
        assert!((ab.width() - w).abs() < 1e-6 && (ab.height() - h).abs() < 1e-6, "{what}: artboard {ab:?}");
        let art = doc.art_bounds().expect("the image");
        assert!((art.width() - w).abs() < 1e-6 && (art.height() - h).abs() < 1e-6, "{what}: the image fills it, {art:?}");
    }
}

#[test]
fn a_raster_export_says_nothing_of_images_another_thread_could_not_decode() {
    // While another thread keeps exporting a document whose image can't be decoded (two exports at
    // once, or the canvas drawing such a document), PNG exports of art with no image report
    // nothing, and every export on the other thread still reports its own image.
    use std::sync::atomic::{AtomicBool, Ordering};
    use vectorcraft_doc::{ImageBlob, ImageObject, Node, NodeKind};
    let mut broken = vectorcraft_doc::Document::new(40.0, 20.0);
    broken.images.insert("broken".into(), ImageBlob::new("image/png", b"\x89PNG\r\n\x1a\n".to_vec()));
    let im = ImageObject {
        key: "broken".into(),
        width: 2,
        height: 1,
        xf: vectorcraft_geom::Affine::scale(20.0),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let (id, layer) = (broken.alloc_id(), broken.layers[0].id);
    broken.insert(Some(layer), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
    let mut s = session(400.0, 400.0, 1);
    s.execute("shape.ellipse", &json!({"x": 20, "y": 20, "width": 360, "height": 360})).unwrap();
    let says_so = |w: &[Value]| w.iter().any(|w| w.as_str().is_some_and(|w| w.contains("couldn't be decoded")));
    let stop = AtomicBool::new(false);
    let (tx, rx) = std::sync::mpsc::channel();
    // Nothing in the scope panics, so the other thread is always told to stop.
    let (theirs, ours) = std::thread::scope(|scope| {
        let other = scope.spawn(|| {
            let mut warned = vec![];
            while !stop.load(Ordering::Relaxed) {
                let w = encode_with_warnings(&broken, "png", &json!({})).map(|(_, w)| w.into_iter().map(Value::from).collect::<Vec<_>>());
                warned.push(w.is_ok_and(|w| says_so(&w)));
                let _ = tx.send(());
            }
            warned
        });
        let started = rx.recv_timeout(std::time::Duration::from_secs(60)).is_ok();
        let ours: Vec<Option<bool>> = (0..5)
            .map(|_| s.execute("document.export", &json!({"format": "png"})).ok().map(|r| says_so(r["warnings"].as_array().map_or(&[], |w| w))))
            .collect();
        stop.store(true, Ordering::Relaxed);
        (other.join().ok(), started.then_some(ours))
    });
    let theirs = theirs.expect("the other thread's exports");
    assert!(!theirs.is_empty() && theirs.iter().all(|w| *w), "the other thread's exports: {theirs:?}");
    assert_eq!(ours.expect("the other thread started"), vec![Some(false); 5], "exports with no image");
}

#[test]
fn a_raster_export_says_when_an_image_could_not_be_decoded() {
    // A PNG whose header reads (so it places at its size) but whose pixel data is cut short:
    // the renderer can't decode it and draws nothing, which the export must say.
    let mut s = session(72.0, 36.0, 1);
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 30, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let cut = png_chunk(&png, b"IDAT").map(|d| d.as_ptr() as usize - png.as_ptr() as usize).unwrap() + 4;
    png.truncate(cut);
    s.execute("file.place", &json!({"name": "cut.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
    let r = s.execute("document.export", &json!({"format": "png", "background": "transparent"})).unwrap();
    let warnings = r["warnings"].as_array().cloned().unwrap_or_default();
    assert!(warnings.iter().any(|w| w.as_str().is_some_and(|w| w.contains("couldn't be decoded"))), "{r}");
}

#[test]
fn export_rect_writes_that_page_area_and_leaves_the_document_alone() {
    let mut s = session(200.0, 400.0, 1);
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 40, "width": 40, "height": 300})).unwrap();
    let boards = s.doc().unwrap().doc.artboards.len();
    let r = s.execute("document.export", &json!({"format": "png", "rect": [20, 40, 40, 300], "ppi": 72})).unwrap();
    let img = rgba(&b64(&r));
    assert_eq!((img.width(), img.height()), (40, 300));
    assert_eq!(img.get_pixel(20, 150).0[3], 255, "the strip is all art");
    assert_eq!(s.doc().unwrap().doc.artboards.len(), boards, "no helper artboard is left");
    // Twice the resolution, and a window that holds both art and empty page.
    let img = rgba(&b64(&s.execute("document.export", &json!({"format": "png", "rect": [0.0, 40.0, 40.0, 10.0], "scale": 2})).unwrap()));
    assert_eq!((img.width(), img.height()), (80, 20));
    assert_eq!((img.get_pixel(10, 10).0[3], img.get_pixel(60, 10).0[3]), (0, 255));
}

#[test]
fn export_rect_refuses_what_cannot_be_exported() {
    let mut s = session(200.0, 400.0, 1);
    s.execute("shape.rectangle", &json!({"x": 20, "y": 40, "width": 40, "height": 300})).unwrap();
    let bad = [
        json!({"format": "png", "rect": [0, 0, 0, 10]}),
        json!({"format": "png", "rect": [0, 0, -5, 10]}),
        json!({"format": "png", "rect": [0, 0, 10]}),
        json!({"format": "png", "rect": [0, 0, "a", 10]}),
        json!({"format": "png", "rect": "20,40,40,300"}),
        json!({"format": "png", "rect": [1e308, 0, 1e308, 10]}),
        json!({"format": "png", "rect": [0, 0, 100000, 100000]}),
        json!({"format": "png", "rect": [0, 0, 10, 10], "artboard": 0}),
        json!({"format": "png", "rect": [0, 0, 10, 10], "useArtboards": true}),
        json!({"format": "svg", "rect": [0, 0, 10, 10]}),
    ];
    for p in bad {
        assert!(s.execute("document.export", &p).is_err(), "{p}");
    }
}
