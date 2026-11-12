//! EPS export through the commands: the art's bounds or one file per artboard, flattened
//! transparency, previews, colours, options and the document the file carries.

use serde_json::{Value, json};

use super::*;

fn session(width: f64, height: f64, artboards: usize) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": width, "height": height, "artboards": artboards})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s
}

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

/// The PostScript of an EPS file (behind its preview header, if any).
fn postscript(bytes: &[u8]) -> String {
    String::from_utf8(vectorcraft_eps::sections(bytes).unwrap().0.to_vec()).unwrap()
}

fn dsc(ps: &str, key: &str) -> Option<String> {
    let prefix = format!("%%{key}:");
    ps.lines().find_map(|l| l.strip_prefix(&prefix)).map(|v| v.trim().to_string())
}

/// The first value of TIFF tag `tag` (a short or a long, in the first directory).
fn tiff_tag(tiff: &[u8], tag: u16) -> Option<u32> {
    let u16_at = |at: usize| Some(u16::from_le_bytes(tiff.get(at..at + 2)?.try_into().ok()?));
    let dir = u32::from_le_bytes(tiff.get(4..8)?.try_into().ok()?) as usize;
    let entry = (0..u16_at(dir)? as usize).map(|i| dir + 2 + i * 12).find(|e| u16_at(*e) == Some(tag))?;
    match u16_at(entry + 2)? {
        3 => u16_at(entry + 8).map(u32::from),
        _ => Some(u32::from_le_bytes(tiff.get(entry + 8..entry + 12)?.try_into().ok()?)),
    }
}

fn warnings(r: &Value) -> Vec<String> {
    r["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect()
}

#[test]
fn eps_covers_the_art_y_up_from_the_first_artboard_with_a_preview_and_the_document() {
    let mut s = session(200.0, 200.0, 1);
    s.execute("shape.rectangle", &json!({"x": 50, "y": 60, "width": 100, "height": 40})).unwrap();
    let r = s.execute("document.export", &json!({"format": "eps"})).unwrap();
    assert_eq!(r["format"], "eps");
    let bytes = b64(&r);
    // The default TIFF preview sits behind the binary header.
    let (_, tiff) = vectorcraft_eps::sections(&bytes).unwrap();
    let preview = image::load_from_memory_with_format(tiff.unwrap(), image::ImageFormat::Tiff).unwrap();
    assert_eq!((preview.width(), preview.height()), (100, 40));
    let ps = postscript(&bytes);
    assert_eq!(dsc(&ps, "BoundingBox").as_deref(), Some("50 100 150 140"), "60..100 down is 100..140 up");
    assert_eq!(dsc(&ps, "LanguageLevel").as_deref(), Some("3"));
    assert!(ps.contains("[1 0 0 -1 0 200] cm"));
    // RGB documents are written in CMYK by default.
    assert!(!ps.contains(" rg\n") && ps.contains(" k\n"), "{ps}");
    // The document comes back whole, and so does a thumbnail.
    let native = vectorcraft_format::load(&vectorcraft_eps::native(&bytes).unwrap()).unwrap();
    assert_eq!(native.layers[0].children().unwrap().len(), 1);
    assert!(vectorcraft_eps::thumbnail(&bytes).unwrap().starts_with(b"\x89PNG"));
    // Without the preview, thumbnail and CMYK: plain PostScript in RGB.
    let r = s.execute("document.exportEps", &json!({"previewFormat": "none", "thumbnails": false, "cmykPostScript": false, "level": 2})).unwrap();
    let bytes = b64(&r);
    assert!(bytes.starts_with(b"%!PS-Adobe-3.0 EPSF-3.0"));
    let ps = postscript(&bytes);
    assert!(ps.contains(" rg\n") && vectorcraft_eps::thumbnail(&bytes).is_none());
    assert_eq!(dsc(&ps, "LanguageLevel").as_deref(), Some("2"));
}

#[test]
fn use_artboards_writes_a_file_per_artboard_named_with_an_underscore() {
    let mut s = session(100.0, 100.0, 2);
    let second = s.doc().unwrap().doc.artboards[1].rect.x0;
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("shape.rectangle", &json!({"x": second + 10.0, "y": 10, "width": 30, "height": 30})).unwrap();
    let r = s.execute("document.export", &json!({"format": "eps", "useArtboards": true, "previewFormat": "none"})).unwrap();
    let files = r["files"].as_array().unwrap();
    let names: Vec<&str> = files.iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Untitled-1.eps", "Untitled-1_Artboard-1.eps", "Untitled-1_Artboard-2.eps"]);
    // The central file comes first: the whole document, bounded by the visible art of both artboards.
    let central = b64(&files[0]);
    assert_eq!(dsc(&postscript(&central), "BoundingBox").as_deref(), Some(format!("10 60 {} 90", second + 40.0).as_str()));
    let native = vectorcraft_format::load(&vectorcraft_eps::native(&central).unwrap()).unwrap();
    assert_eq!((native.artboards.len(), native.layers[0].children().unwrap().len()), (2, 2));
    for f in &files[1..] {
        let bytes = b64(f);
        let ps = postscript(&bytes);
        assert_eq!(dsc(&ps, "BoundingBox").as_deref(), Some("0 0 100 100"));
        // Each carries the document with its own artboard.
        let native = vectorcraft_format::load(&vectorcraft_eps::native(&bytes).unwrap()).unwrap();
        assert_eq!(native.artboards.len(), 1);
    }
    // Named artboards, two or more, get the central file too.
    let r = s.execute("document.exportEps", &json!({"artboards": [1, 0], "previewFormat": "none"})).unwrap();
    let names: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Untitled-1.eps", "Untitled-1_Artboard-2.eps", "Untitled-1_Artboard-1.eps"]);
    // A range names the artboards (useArtboards implied); one file keeps the path's name.
    let r = s.execute("document.exportEps", &json!({"range": "2", "previewFormat": "none"})).unwrap();
    assert!(r.get("files").is_none());
    assert!(postscript(&b64(&r)).contains(&format!("[1 0 0 -1 {} 100] cm", -second)));
    // useArtboards: false writes one file of the visible art, even with artboards named.
    let r = s.execute("document.exportEps", &json!({"useArtboards": false, "artboards": [0, 1], "previewFormat": "none"})).unwrap();
    assert!(r.get("files").is_none());
    assert_eq!(dsc(&postscript(&b64(&r)), "BoundingBox").as_deref(), Some(format!("10 60 {} 90", second + 40.0).as_str()));
    assert!(s.execute("document.exportEps", &json!({"artboard": 5})).is_err());
    // Without art there is nothing to bound.
    let mut empty = session(100.0, 100.0, 1);
    assert!(empty.execute("document.exportEps", &json!({})).unwrap_err().to_string().contains("nothing to export"));
    // Artboards without art still get their files, but no central file.
    let mut empty = session(100.0, 100.0, 2);
    let r = empty.execute("document.exportEps", &json!({"useArtboards": true, "previewFormat": "none"})).unwrap();
    assert_eq!(r["files"].as_array().map(Vec::len), Some(2), "{r}");
    // Named artboards without useArtboards get the same files.
    let r = empty.execute("document.exportEps", &json!({"range": "1-2", "previewFormat": "none"})).unwrap();
    assert_eq!(r["files"].as_array().map(Vec::len), Some(2), "{r}");
}

#[test]
fn use_artboards_writes_the_central_file_beside_the_artboard_files() {
    let mut s = session(100.0, 100.0, 2);
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    let dir = std::env::temp_dir().join(format!("vc-epsboards-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("logo.eps").to_string_lossy().to_string();
    s.execute("document.exportEps", &json!({"path": path, "useArtboards": true, "previewFormat": "none"})).unwrap();
    let mut written: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    written.sort();
    assert_eq!(written, ["logo.eps", "logo_Artboard-1.eps", "logo_Artboard-2.eps"]);
    let central = std::fs::read(&path).unwrap();
    assert_eq!(dsc(&postscript(&central), "BoundingBox").as_deref(), Some("10 60 40 90"));
    let _ = std::fs::remove_dir_all(dir);
    // Export for Screens still writes one EPS per artboard, with no central file.
    let rows = json!({"formats": [{"format": "eps", "useArtboards": true}], "settings": {"eps": {"useArtboards": true}}});
    let r = s.execute("document.exportForScreens", &rows).unwrap();
    assert_eq!(r["files"].as_array().map(Vec::len), Some(2), "{r}");
}

#[test]
fn transparency_is_flattened_with_the_flattener_preset() {
    let mut s = session(200.0, 200.0, 1);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 100, "height": 100})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 60, "y": 60, "width": 100, "height": 100})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    let opts = json!({"previewFormat": "none", "thumbnails": false, "cmykPostScript": false});
    let r = s.execute("document.exportEps", &opts).unwrap();
    let w = warnings(&r);
    assert!(w.iter().any(|w| w.contains("flattened")), "{w:?}");
    assert!(!w.iter().any(|w| w.contains("written opaque")), "nothing transparent reaches the writer: {w:?}");
    let ps = postscript(&b64(&r));
    // Flat regions: red alone, the half-transparent blue over red, and over white.
    let colours: Vec<&str> = ps.lines().filter(|l| l.ends_with(" rg")).collect();
    assert_eq!(colours, ["0.5 0.5 1 rg", "1 0 0 rg", "0.5 0 0.5 rg"], "{ps}");
    // The document is left as it was, and reopens with its transparency.
    let doc = &s.doc().unwrap().doc;
    assert_eq!(doc.layers[0].children().unwrap().len(), 2);
    let native = vectorcraft_format::load(&vectorcraft_eps::native(&b64(&r)).unwrap()).unwrap();
    assert!(native.layers[0].children().unwrap().iter().any(|c| c.opacity < 1.0));
    // A saved preset by name; an unknown one is refused.
    s.execute("flattener.presets.save", &json!({"name": "All Raster", "balance": 0})).unwrap();
    let r = s.execute("document.exportEps", &json!({"flattenerPreset": "All Raster", "previewFormat": "none"})).unwrap();
    assert!(postscript(&b64(&r)).contains("image"), "rasterized");
    let e = s.execute("document.exportEps", &json!({"flattenerPreset": "Nope"})).unwrap_err().to_string();
    assert!(e.contains("preset"), "{e}");
}

/// A picture cropped by a clipping path, in a clip group at 80%, over a square that only the
/// hidden part of the picture covers: the file prints the square, and its image is as large as
/// the crop, with the default preset (a rectangle of image) and with High Resolution (an image
/// clipped to its regions).
#[test]
fn flattening_cropped_art_keeps_the_art_under_its_hidden_part() {
    use super::tests_pdfx::{Art, CROP, cropped, drawn, lost, shown_image_sizes};
    let _g = crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    for preset in ["medium", "high"] {
        let (mut s, _) = cropped("rgb", 400.0, Art::Image { see_through: false });
        s.execute("transparency.set", &json!({"opacity": 80})).unwrap();
        let canvas = drawn(&s.doc().unwrap().doc);
        let opts = json!({"artboards": [0], "flattenerPreset": preset, "previewFormat": "none", "thumbnails": false});
        let r = s.execute("document.exportEps", &opts).unwrap();
        // What the file prints, not the document it carries.
        let back = vectorcraft_eps::import_with(&b64(&r), false).unwrap().document;
        assert_eq!(lost(&canvas, &drawn(&back)), 0, "{preset}: art went missing");
        let sizes = shown_image_sizes(&back);
        assert!(!sizes.is_empty() && sizes.iter().all(|(w, h)| *w <= CROP[2] + 1.0 && *h <= CROP[3] + 1.0), "{preset}: the images show {sizes:?}");
    }
}

#[test]
fn overprints_are_kept_or_discarded() {
    let mut s = session(100.0, 100.0, 1);
    let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap();
    s.execute("attributes.set", &json!({"ids": [id], "overprintFill": true})).unwrap();
    let kept = postscript(&b64(&s.execute("document.exportEps", &json!({"previewFormat": "none"})).unwrap()));
    assert!(kept.contains("true op\n"), "{kept}");
    let dropped = postscript(&b64(&s.execute("document.exportEps", &json!({"previewFormat": "none", "overprints": "discard"})).unwrap()));
    assert!(!dropped.contains("true op"));
}

#[test]
fn placed_images_are_written_and_previews_are_black_and_white_on_request() {
    let mut s = super::tests_svg::image_session();
    let r = s.execute("document.exportEps", &json!({"previewFormat": "tiffBw", "level": 2})).unwrap();
    let bytes = b64(&r);
    let ps = postscript(&bytes);
    assert!(ps.contains("/RunLengthDecode filter >> image"), "{ps}");
    let tiff = vectorcraft_eps::sections(&bytes).unwrap().1.unwrap();
    // A bilevel TIFF: one sample of one bit, white is zero.
    assert!(tiff.starts_with(b"II*\0"));
    assert_eq!((tiff_tag(tiff, 258), tiff_tag(tiff, 277), tiff_tag(tiff, 262)), (Some(1), Some(1), Some(0)));
    let l3 = postscript(&b64(&s.execute("document.exportEps", &json!({"previewFormat": "none"})).unwrap()));
    assert!(l3.contains("/FlateDecode filter >> image"));
}

/// The PNG thumbnail of an EPS file, decoded.
fn thumbnail_png(bytes: &[u8]) -> image::RgbaImage {
    let png = vectorcraft_eps::thumbnail(bytes).expect("a thumbnail");
    image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap().to_rgba8()
}

/// The TIFF preview of an EPS file, decoded.
fn preview_tiff(bytes: &[u8]) -> image::RgbaImage {
    let tiff = vectorcraft_eps::sections(bytes).unwrap().1.expect("a preview");
    image::load_from_memory_with_format(tiff, image::ImageFormat::Tiff).unwrap().to_rgba8()
}

/// The size of a thumbnail render of the rect an EPS file's preview shows (its bounding box).
fn thumbnail_size(bytes: &[u8]) -> (u32, u32) {
    let b: Vec<f64> = dsc(&postscript(bytes), "BoundingBox").unwrap().split(' ').map(|v| v.parse().unwrap()).collect();
    let rect = vectorcraft_geom::Rect::new(b[0], b[1], b[2], b[3]);
    let long = rect.width().max(rect.height());
    vectorcraft_render::raster_size(rect, if long > 128.0 { 128.0 / long } else { 1.0 }).unwrap()
}

/// The thumbnail is the color preview made smaller, the area average of its pixels at the size a
/// 128 px render has, not a second render of the document; it shows the same picture as one.
#[test]
fn the_thumbnail_is_made_from_the_preview() {
    let mut s = session(1000.0, 700.0, 1);
    s.execute("paint.setFill", &json!({"color": "#2060c0"})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 37.3, "y": 41.9, "width": 811.7, "height": 563.1})).unwrap();
    let r = s.execute("document.exportEps", &json!({"useArtboards": true, "previewFormat": "tiffColor", "transparentPreview": false})).unwrap();
    let bytes = b64(&r);
    let preview = preview_tiff(&bytes);
    assert_eq!(preview.dimensions(), (1000, 700));
    let thumb = thumbnail_png(&bytes);
    assert_eq!(thumb.dimensions(), (128, 90));
    // An opaque preview's straight pixels are its premultiplied ones.
    let shown = vectorcraft_render::Rendered { width: 1000, height: 700, pixels: preview.into_raw() };
    assert!(thumb.as_raw() == &shown.area_resized(128, 90).unwrap().pixels, "the thumbnail isn't the preview made smaller");
    let doc = &s.doc().unwrap().doc;
    let fresh = vectorcraft_render::Renderer::new().render_region(doc, vectorcraft_geom::Rect::new(0.0, 0.0, 1000.0, 700.0), 0.128, true);
    let mut diffs: Vec<u8> = thumb.as_raw().iter().zip(&fresh.pixels).map(|(a, b)| a.abs_diff(*b)).collect();
    diffs.sort_unstable();
    let mean = diffs.iter().map(|d| f64::from(*d)).sum::<f64>() / diffs.len() as f64;
    let p95 = diffs[diffs.len() * 95 / 100];
    assert!(mean <= 8.0 && p95 <= 32, "mean {mean}, 95th percentile {p95}");
}

/// A thumbnail is opaque color on white whatever the preview: a transparent color preview's, and a
/// black-and-white preview's, which keeps the art's colors.
#[test]
fn thumbnails_of_transparent_and_black_and_white_previews_are_opaque_color() {
    let mut s = session(400.0, 200.0, 1);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let fresh = vectorcraft_render::Renderer::new().render_region(doc, vectorcraft_geom::Rect::new(0.0, 0.0, 400.0, 200.0), 0.32, true);
    let red: [u8; 4] = fresh.pixels[(32 * 128 + 10) * 4..][..4].try_into().unwrap();
    for preview in ["tiffColor", "tiffBw"] {
        let r = s.execute("document.exportEps", &json!({"useArtboards": true, "previewFormat": preview})).unwrap();
        let thumb = thumbnail_png(&b64(&r));
        assert_eq!(thumb.dimensions(), (128, 64), "{preview}");
        assert_eq!(thumb.get_pixel(10, 32).0, red, "{preview}");
        assert_eq!(thumb.get_pixel(120, 32).0, [255, 255, 255, 255], "{preview}");
        assert!(thumb.pixels().all(|p| p[3] == 255), "{preview}");
    }
}

/// A page that fits in a thumbnail gets its preview's own pixels, unresampled.
#[test]
fn a_small_page_s_thumbnail_is_its_preview() {
    let mut s = session(100.0, 60.0, 1);
    s.execute("shape.ellipse", &json!({"x": 10.5, "y": 7.25, "width": 70, "height": 41})).unwrap();
    let bytes = b64(&s.execute("document.exportEps", &json!({"useArtboards": true, "transparentPreview": false})).unwrap());
    let thumb = thumbnail_png(&bytes);
    assert_eq!(thumb.dimensions(), (100, 60));
    assert!(thumb == preview_tiff(&bytes), "the thumbnail isn't the preview");
}

/// Every file has a thumbnail at the size a thumbnail render has, with any page shape, on Save
/// too and without a preview; with thumbnails off there is none.
#[test]
fn every_file_s_thumbnail_has_the_thumbnail_size() {
    let mut s = session(333.3, 77.7, 2);
    let second = s.doc().unwrap().doc.artboards[1].rect.x0;
    s.execute("shape.rectangle", &json!({"x": 3.7, "y": 5.1, "width": 300, "height": 61.3})).unwrap();
    s.execute("shape.ellipse", &json!({"x": second + 1.0, "y": 2.0, "width": 20, "height": 70})).unwrap();
    for preview in ["tiffColor", "tiffBw", "none"] {
        let r = s.execute("document.exportEps", &json!({"useArtboards": true, "previewFormat": preview})).unwrap();
        let files = r["files"].as_array().unwrap();
        assert_eq!(files.len(), 3);
        for f in files {
            let bytes = b64(f);
            assert_eq!(thumbnail_png(&bytes).dimensions(), thumbnail_size(&bytes), "{preview} {}", f["name"]);
        }
    }
    assert_eq!(thumbnail_size(&b64(&s.execute("document.exportEps", &json!({"range": "1"})).unwrap())), (128, 30));
    let saved = b64(&s.execute("document.save", &json!({"format": "eps", "previewFormat": "tiffColor"})).unwrap());
    assert_eq!(thumbnail_png(&saved).dimensions(), thumbnail_size(&saved));
    let r = s.execute("document.exportEps", &json!({"useArtboards": true, "thumbnails": false})).unwrap();
    assert!(r["files"].as_array().unwrap().iter().all(|f| vectorcraft_eps::thumbnail(&b64(f)).is_none()));
}

/// Two placed images whose pixel data can't be decoded are left out of the EPS, and `warnings`
/// holds the note raster exports give, once, without naming either image's key.
#[test]
fn images_that_cant_be_decoded_give_the_raster_note_once() {
    const NOTE: &str = "an image couldn't be decoded and is missing from the picture";
    let mut s = session(200.0, 100.0, 1);
    crate::tests_menucmds::place_undecodable_png(&mut s);
    crate::tests_menucmds::place_undecodable_png_sized(&mut s, 32, 16);
    let keys: Vec<String> = s.doc().unwrap().doc.images.keys().cloned().collect();
    assert_eq!(keys.len(), 2);
    let r = s.execute("document.exportEps", &json!({"previewFormat": "none"})).unwrap();
    let w = warnings(&r);
    assert_eq!(w.iter().filter(|w| *w == NOTE).count(), 1, "{w:?}");
    assert!(!w.iter().any(|w| keys.iter().any(|k| w.contains(k.as_str()))), "{w:?}");
    assert!(!postscript(&b64(&r)).contains("filter >> image"), "no image is written");
}

#[test]
fn bad_eps_options_are_refused_and_formats_list_eps() {
    let mut s = session(100.0, 100.0, 1);
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    for p in [
        json!({"level": 1}),
        json!({"level": "4"}),
        json!({"previewFormat": "pict"}),
        json!({"overprints": "simulate"}),
        json!({"flattener": {"balance": 400}}),
        json!({"thumbnails": "yes"}),
    ] {
        let e = s.execute("document.exportEps", &p).unwrap_err().to_string();
        assert!(e.contains("EPS"), "{p}: {e}");
    }
    // Export for Screens writes an EPS per artboard (a document of that artboard alone).
    let r = s.execute("document.exportForScreens", &json!({"formats": [{"format": "eps"}]})).unwrap();
    assert!(r["files"][0]["name"].as_str().unwrap().ends_with(".eps"), "{r}");
    let r = s.execute("document.formats", &json!({})).unwrap();
    let eps = r["formats"].as_array().unwrap().iter().find(|f| f["id"] == "eps").unwrap().clone();
    // Readable too since EPS import (tests_epsimport.rs).
    assert_eq!((eps["read"].as_bool(), eps["write"].as_bool(), eps["mime"].as_str()), (Some(true), Some(true), Some("application/postscript")));
    for o in [
        "level",
        "previewFormat",
        "transparentPreview",
        "overprints",
        "flattenerPreset",
        "flattener",
        "embedFonts",
        "includeLinkedFiles",
        "thumbnails",
        "cmykPostScript",
        "compatibleGradients",
        "selectedOnly",
        "useArtboards",
    ] {
        assert!(eps["options"].get(o).is_some(), "{o}");
    }
}

#[test]
fn selected_only_writes_the_selection() {
    let mut s = session(200.0, 200.0, 1);
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    let b = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 50, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    let ps = postscript(&b64(&s.execute("document.exportEps", &json!({"selectedOnly": true, "previewFormat": "none"})).unwrap()));
    assert_eq!(dsc(&ps, "BoundingBox").as_deref(), Some("100 80 150 100"));
}
