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
    assert_eq!(names, ["Untitled-1_Artboard-1.eps", "Untitled-1_Artboard-2.eps"]);
    for f in files {
        let bytes = b64(f);
        let ps = postscript(&bytes);
        assert_eq!(dsc(&ps, "BoundingBox").as_deref(), Some("0 0 100 100"));
        // Each carries the document with its own artboard.
        let native = vectorcraft_format::load(&vectorcraft_eps::native(&bytes).unwrap()).unwrap();
        assert_eq!(native.artboards.len(), 1);
    }
    // A range names the artboards (useArtboards implied); one file keeps the path's name.
    let r = s.execute("document.exportEps", &json!({"range": "2", "previewFormat": "none"})).unwrap();
    assert!(r.get("files").is_none());
    assert!(postscript(&b64(&r)).contains(&format!("[1 0 0 -1 {} 100] cm", -second)));
    assert!(s.execute("document.exportEps", &json!({"artboard": 5})).is_err());
    // Without art there is nothing to bound.
    let mut empty = session(100.0, 100.0, 1);
    assert!(empty.execute("document.exportEps", &json!({})).unwrap_err().to_string().contains("nothing to export"));
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
