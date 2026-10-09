use std::io::Cursor;

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;

use super::*;
use crate::cmd::parse_range;

fn session(width: f64, height: f64, artboards: usize) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": width, "height": height, "artboards": artboards})).unwrap();
    s
}

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("vc-fileio-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

fn open(s: &mut Session, name: &str, bytes: &[u8]) -> Value {
    s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)}))
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// A `w`×`h` image of one colour in `format`.
fn image_bytes(w: u32, h: u32, format: image::ImageFormat) -> Vec<u8> {
    let rgba = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 40, 255]));
    let img = if format == image::ImageFormat::Jpeg {
        image::DynamicImage::ImageRgb8(image::DynamicImage::ImageRgba8(rgba).to_rgb8())
    } else {
        rgba.into()
    };
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), format).unwrap();
    out
}

fn image_of(s: &Session) -> vectorcraft_doc::ImageObject {
    let doc = &s.doc().unwrap().doc;
    match &doc.layers[0].children().unwrap()[0].kind {
        NodeKind::Image(im) => im.clone(),
        other => panic!("expected an image, got {other:?}"),
    }
}

#[test]
fn opens_every_readable_format() {
    let mut s = session(200.0, 100.0, 1);
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    let native = b64(&s.execute("document.serialize", &json!({})).unwrap());
    let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap()["text"].as_str().unwrap().to_string();
    let pdf = b64(&s.execute("document.serialize", &json!({"format": "pdf"})).unwrap());
    let svgz = vectorcraft_svg::compress(&svg);

    for (name, bytes, format) in [
        ("a.vectorcraft", native.clone(), "vectorcraft"),
        ("a.drawcraft", native, "vectorcraft"),
        ("a.svg", svg.into_bytes(), "svg"),
        ("a.svgz", svgz, "svgz"),
        ("a.pdf", pdf.clone(), "pdf"),
        ("a.ai", pdf, "ai"),
    ] {
        let r = open(&mut s, name, &bytes);
        assert_eq!(r["format"], format, "{name}");
        assert_eq!(r["title"], if format == "vectorcraft" { "Untitled-1" } else { name }, "{name}");
        assert!(s.doc().unwrap().doc.node_count() >= 2, "{name}: the rectangle came through");
    }
    for (name, kind) in [
        ("p.png", image::ImageFormat::Png),
        ("p.jpg", image::ImageFormat::Jpeg),
        ("p.gif", image::ImageFormat::Gif),
        ("p.webp", image::ImageFormat::WebP),
        ("p.tif", image::ImageFormat::Tiff),
        ("p.bmp", image::ImageFormat::Bmp),
    ] {
        let r = open(&mut s, name, &image_bytes(5, 4, kind));
        assert_eq!(r["format"], format(name.rsplit('.').next().unwrap()).unwrap().id, "{name}");
        let im = image_of(&s);
        assert_eq!((im.width, im.height), (5, 4), "{name}");
        let doc = &s.doc().unwrap().doc;
        assert_eq!((doc.artboards[0].rect.width(), doc.artboards[0].rect.height()), (5.0, 4.0));
        // Browser-safe formats keep their bytes; TIFF and BMP are stored as PNG.
        let mime = &doc.images[&im.key].mime;
        assert_eq!(mime, if name.ends_with("tif") || name.ends_with("bmp") { "image/png" } else { format_for_name(name).unwrap().mime });
    }
    assert!(s.execute("document.open", &json!({"name": "x.txt", "dataBase64": "aGVsbG8="})).is_err());
}

#[test]
fn webp_opens_at_its_pixel_size() {
    let mut s = Session::new();
    open(&mut s, "tiny.webp", &image_bytes(3, 2, image::ImageFormat::WebP));
    let im = image_of(&s);
    assert_eq!((im.width, im.height), (3, 2));
    let ab = s.doc().unwrap().doc.artboards[0].rect;
    assert_eq!((ab.width(), ab.height()), (3.0, 2.0));
    let r = raster_image(&image_bytes(3, 2, image::ImageFormat::WebP)).unwrap();
    assert_eq!((r.width, r.height, r.blob.mime.as_str()), (3, 2, "image/webp"));
}

#[test]
fn content_beats_a_wrong_extension() {
    let png = image_bytes(2, 2, image::ImageFormat::Png);
    assert_eq!(detect("photo.jpg", &png).unwrap().id, "png");
    assert_eq!(detect("noext", b"<?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\"/>").unwrap().id, "svg");
    assert_eq!(detect("x.ait", b"%PDF-1.7").unwrap().id, "ait");
    assert_eq!(detect("x.bin", b"%PDF-1.7").unwrap().id, "pdf");
    assert!(detect("x.bin", b"nothing").is_none());
}

#[test]
fn templates_open_untitled() {
    let mut s = session(100.0, 100.0, 1);
    let dir = tmp_dir("tpl");
    let path = dir.join("t.vectorcraft").to_string_lossy().to_string();
    s.execute("file.saveAsTemplate", &json!({"path": path})).unwrap();
    let r = s.execute("document.open", &json!({"path": path})).unwrap();
    assert!(r["title"].as_str().unwrap().starts_with("Untitled-"), "{r}");
    assert_eq!(s.doc().unwrap().path, None);
    assert!(!s.doc().unwrap().doc.template);

    let pdf = b64(&s.execute("document.serialize", &json!({"format": "pdf"})).unwrap());
    let ait = dir.join("t.ait").to_string_lossy().to_string();
    std::fs::write(&ait, pdf).unwrap();
    let r = s.execute("document.open", &json!({"path": ait})).unwrap();
    assert_eq!(r["format"], "ait");
    assert!(r["title"].as_str().unwrap().starts_with("Untitled-"), "{r}");
    assert_eq!(s.doc().unwrap().path, None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn open_exts_cover_every_readable_format() {
    for f in FORMATS.iter().filter(|f| f.read) {
        for e in f.extensions {
            assert!(OPEN_EXTS.contains(e), "OPEN_EXTS lacks .{e} ({})", f.label);
        }
    }
    for e in OPEN_EXTS {
        assert!(format(e).is_some_and(|f| f.read), ".{e} in OPEN_EXTS is no readable format");
    }
    let filters: Vec<_> = open_filters().collect();
    assert_eq!(filters[0], ("All readable files", OPEN_EXTS));
    assert_eq!(filters.len(), 6 + FORMATS.iter().filter(|f| f.read).count(), "and swatch libraries, flattener, PDF and print presets, plug-ins");
    assert_eq!(filters.last(), Some(&("Plug-ins", crate::cmd::plugin::EXTS)), "File › Open installs plug-ins");
}

#[test]
fn formats_query_lists_readers_writers_and_options() {
    let mut s = Session::new();
    let r = s.execute("document.formats", &json!({})).unwrap();
    let ids = |k: &str| r[k].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(
        ids("writable"),
        [
            "vectorcraft",
            "svg",
            "svgz",
            "pdf",
            "png",
            "jpg",
            "gif",
            "webp",
            "tiff",
            "bmp",
            "template",
            "png8",
            "txt",
            "dxf",
            "eps",
            "emf",
            "wmf",
            "tga",
            "psd"
        ]
    );
    assert!(ids("readable").contains(&"tiff".to_string()) && ids("readable").contains(&"ait".to_string()));
    let png = r["formats"].as_array().unwrap().iter().find(|f| f["id"] == "png").unwrap();
    assert_eq!(png["options"]["scale"]["default"], 1);
    assert_eq!(r["openExtensions"].as_array().unwrap().len(), OPEN_EXTS.len());
}

#[test]
fn ranges_parse_one_based() {
    assert_eq!(parse_range("1-3, 5", 5).unwrap(), [0, 1, 2, 4]);
    assert_eq!(parse_range("3-, 1", 4).unwrap(), [2, 3, 0]);
    assert_eq!(parse_range("-2", 4).unwrap(), [0, 1]);
    assert_eq!(parse_range("2,2,1\u{2013}2", 4).unwrap(), [1, 0]);
    for bad in ["", "0", "6", "3-1", "a", "-", "1-9"] {
        assert!(parse_range(bad, 5).is_err(), "{bad:?}");
    }
}

/// Pages in a PDF, counted by importing it (one artboard per page).
fn pdf_pages(bytes: &[u8]) -> usize {
    vectorcraft_pdf::import(bytes).unwrap().artboards.len()
}

#[test]
fn pdf_export_honours_artboard_and_range() {
    let mut s = session(100.0, 80.0, 3);
    let pages = |s: &mut Session, p: Value| pdf_pages(&b64(&s.execute("document.export", &p).unwrap()));
    assert_eq!(pages(&mut s, json!({"format": "pdf"})), 3);
    assert_eq!(pages(&mut s, json!({"format": "pdf", "artboard": 1})), 1);
    assert_eq!(pages(&mut s, json!({"format": "pdf", "artboards": [0, 2]})), 2);
    assert_eq!(pages(&mut s, json!({"format": "pdf", "range": "2-3"})), 2);
    assert_eq!(pdf_pages(&b64(&s.execute("document.serialize", &json!({"format": "pdf", "artboard": 2})).unwrap())), 1);
    assert!(s.execute("document.export", &json!({"format": "pdf", "artboard": 3})).is_err());
    assert!(s.execute("document.export", &json!({"format": "pdf", "range": "1-4"})).is_err());
    // Single-image formats take one artboard.
    assert!(s.execute("document.export", &json!({"format": "png", "range": "1-2"})).is_err());
    let png = b64(&s.execute("document.export", &json!({"format": "png", "range": "2", "scale": 0.5})).unwrap());
    assert_eq!(image::load_from_memory(&png).unwrap().width(), 50);
    assert!(s.execute("document.export", &json!({"format": "png", "scale": "big"})).is_err(), "typed options reject a bad type");
}

#[test]
fn export_without_path_returns_bytes_and_never_retargets() {
    let mut s = session(100.0, 100.0, 1);
    let dir = tmp_dir("retarget");
    let doc_path = dir.join("doc.vectorcraft").to_string_lossy().to_string();
    s.execute("document.save", &json!({"path": doc_path})).unwrap();
    let copy = dir.join("copy.vectorcraft").to_string_lossy().to_string();
    let r = s.execute("document.export", &json!({"path": copy})).unwrap();
    assert_eq!(r["format"], "vectorcraft");
    assert!(vectorcraft_format::sniff(&std::fs::read(&copy).unwrap()));
    assert_eq!(s.doc().unwrap().path.as_deref(), Some(doc_path.as_str()), "exporting the native format keeps the document's path");
    let r = s.execute("document.export", &json!({"format": "jpg"})).unwrap();
    assert_eq!(&b64(&r)[..2], [0xFF, 0xD8]);
    assert!(r.get("path").is_none());
    assert!(s.execute("document.export", &json!({"path": dir.join("x.dwg").to_string_lossy()})).is_err(), "DWG can't be written");
    // A never-saved document hands its bytes back: a .ai file (a PDF carrying the native document).
    let mut s = session(10.0, 10.0, 1);
    let r = s.execute("document.save", &json!({})).unwrap();
    assert_eq!(r["format"], "ai");
    assert!(vectorcraft_pdf::editing(&b64(&r)).is_some());
    assert_eq!(s.doc().unwrap().path, None);
    let _ = std::fs::remove_dir_all(dir);
}

/// A red square on a template layer and a blue one on a normal layer above it.
fn template_doc() -> (Session, u64, u64) {
    let mut s = session(100.0, 100.0, 1);
    let template = s.doc().unwrap().doc.layers[0].id.0;
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let red = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 100})).unwrap()["id"].as_u64().unwrap();
    s.execute("layer.new", &json!({})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    let blue = s.execute("shape.rectangle", &json!({"x": 50, "y": 0, "width": 50, "height": 100})).unwrap()["id"].as_u64().unwrap();
    s.execute("layer.setProps", &json!({"id": template, "template": true})).unwrap();
    (s, red, blue)
}

fn has_red(png: &[u8]) -> bool {
    image::load_from_memory(png).unwrap().to_rgba8().pixels().any(|p| p[0] > 200 && p[2] < 80 && p[3] > 0)
}

#[test]
fn raster_exports_leave_template_layers_out() {
    let (mut s, red, blue) = template_doc();
    let png = b64(&s.execute("document.export", &json!({"format": "png"})).unwrap());
    assert!(!has_red(&png), "no red pixels from the template layer");
    assert!(image::load_from_memory(&png).unwrap().to_rgba8().get_pixel(75, 50)[2] > 200, "the normal layer is exported");
    s.execute("select.set", &json!({"ids": [red, blue]})).unwrap();
    let r = s.execute("document.exportSelection", &json!({"format": "png"})).unwrap();
    assert!(!has_red(&b64(&r)));
    assert!(r["bounds"][0].as_f64().unwrap() > 40.0, "only the blue square (and its stroke) is exported: {}", r["bounds"]);
}

#[test]
fn export_for_screens_one_page_per_pdf_unique_names() {
    let mut s = session(100.0, 80.0, 3);
    s.execute("artboard.setProps", &json!({"index": 0, "name": "Icon"})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "name": "Icon"})).unwrap();
    let dir = tmp_dir("screens");
    let folder = dir.to_string_lossy().to_string();
    let r = s
        .execute(
            "document.exportForScreens",
            &json!({"folder": folder, "formats": [{"format": "pdf"}, {"format": "svg", "scale": 2}, {"format": "svg", "scale": 3}, {"format": "png", "scale": 2}]}),
        )
        .unwrap();
    let files: Vec<String> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().to_string()).collect();
    let names: Vec<&str> = files.iter().map(|f| f.rsplit('/').next().unwrap()).collect();
    assert_eq!(
        names,
        ["Icon.pdf", "Icon.svg", "Icon@2x.png", "Icon-2.pdf", "Icon-2.svg", "Icon-2@2x.png", "Artboard-3.pdf", "Artboard-3.svg", "Artboard-3@2x.png"]
    );
    for f in files.iter().filter(|f| f.ends_with(".pdf")) {
        assert_eq!(pdf_pages(&std::fs::read(f).unwrap()), 1, "{f}");
    }
    // No folder: the files come back as bytes.
    let r = s.execute("document.exportForScreens", &json!({"range": "3", "formats": [{"format": "jpg", "scale": 0.5}]})).unwrap();
    assert_eq!(r["files"][0]["name"], "Artboard-3@0.5x.jpg");
    assert_eq!(image::load_from_memory(&b64(&r["files"][0])).unwrap().width(), 50);
    assert!(s.execute("document.exportForScreens", &json!({"formats": [{"format": "vectorcraft"}]})).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn template_sublayers_are_left_out_too() {
    let mut s = session(100.0, 100.0, 1);
    let sub = s.execute("layer.newSublayer", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("layer.setCurrent", &json!({"id": sub})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let red = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 100})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(s.doc().unwrap().doc.ancestry(vectorcraft_doc::NodeId(red)).unwrap().len(), 3, "the square is on a sublayer");
    assert!(has_red(&b64(&s.execute("document.export", &json!({"format": "png"})).unwrap())));
    s.execute("layer.setProps", &json!({"id": sub, "template": true})).unwrap();
    assert!(!has_red(&b64(&s.execute("document.export", &json!({"format": "png"})).unwrap())), "a template sublayer is left out");
    s.execute("select.set", &json!({"ids": [red]})).unwrap();
    assert!(s.execute("document.exportSelection", &json!({"format": "png"})).is_err(), "nothing but template art is selected");
}

#[test]
fn export_for_screens_vector_rows_drop_scale_suffixes_and_names_ignore_case() {
    let mut s = session(100.0, 80.0, 3);
    s.execute("artboard.setProps", &json!({"index": 0, "name": "Icon"})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "name": "icon"})).unwrap();
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).artboards[2].name.clear();
    // The dialog's second row starts as PNG @2x; switched to PDF it keeps that suffix text.
    let formats =
        json!([{"format": "png", "scale": 1, "suffix": ""}, {"format": "pdf", "scale": 2, "suffix": "@2x"}, {"format": "svg", "suffix": "-web"}]);
    let r = s.execute("document.exportForScreens", &json!({"formats": formats})).unwrap();
    let names: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "Icon.png",
            "Icon.pdf",
            "Icon-web.svg",
            "icon-2.png",
            "icon-2.pdf",
            "icon-2-web.svg",
            "Artboard-3.png",
            "Artboard-3.pdf",
            "Artboard-3-web.svg"
        ]
    );
}

#[test]
fn oversized_raster_exports_are_refused() {
    let mut s = session(2000.0, 2000.0, 1);
    let e = s.execute("document.export", &json!({"format": "webp", "scale": 10})).unwrap_err().to_string();
    assert!(e.contains("too large"), "{e}");
    assert!(s.execute("document.export", &json!({"format": "png", "scale": 40})).is_err(), "80000 px a side");
}
