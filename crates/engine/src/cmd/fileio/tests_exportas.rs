//! Export As: `useArtboards` writes one file per artboard (all or a range) or, off, the bounds of
//! the visible art.

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

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("vc-exportas-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn use_artboards_writes_one_file_per_artboard_in_the_range() {
    let mut s = session(60.0, 40.0, 5);
    let dir = tmp_dir("range");
    let path = dir.join("Logo.png").to_string_lossy().to_string();
    let r = s.execute("document.export", &json!({"path": path, "useArtboards": true, "range": "1-3,5"})).unwrap();
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    let names: Vec<&str> = files.iter().map(|f| f.rsplit(['/', '\\']).next().unwrap()).collect();
    assert_eq!(names, ["Logo-Artboard-1.png", "Logo-Artboard-2.png", "Logo-Artboard-3.png", "Logo-Artboard-5.png"], "{r}");
    assert_eq!(r["path"], files[0]);
    for f in files {
        assert_eq!(rgba(&std::fs::read(f).unwrap()).dimensions(), (60, 40));
    }
    // Without a range: every artboard; without a path the files come back named after the document.
    let r = s.execute("document.export", &json!({"format": "jpg", "useArtboards": true})).unwrap();
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 5);
    assert_eq!(files[0]["name"], "Untitled-1-Artboard-1.jpg");
    assert_eq!(&b64(&files[4])[..2], [0xFF, 0xD8]);
    // One artboard is one file, written where asked; a PDF holds the range as pages of one file.
    let r = s.execute("document.export", &json!({"format": "webp", "useArtboards": true, "artboards": [3]})).unwrap();
    assert!(r.get("files").is_none() && !b64(&r).is_empty(), "{r}");
    let r = s.execute("document.export", &json!({"format": "pdf", "useArtboards": true, "range": "2-4"})).unwrap();
    assert_eq!(vectorcraft_pdf::import(&b64(&r)).unwrap().artboards.len(), 3);
    // Without Use Artboards a raster export holds one artboard.
    assert!(s.execute("document.export", &json!({"format": "png", "range": "1-2"})).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn per_artboard_names_keep_the_folder_and_extension() {
    let mut s = session(20.0, 20.0, 3);
    s.execute("artboard.setProps", &json!({"index": 0, "name": "Hero / Wide"})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "name": "hero---wide"})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let enc = encode_all(&doc, "jpg", &json!({"useArtboards": true})).unwrap();
    let names: Vec<String> = enc.named(&doc, "C:\\out\\shot.jpeg").into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["C:\\out\\shot-Hero---Wide.jpeg", "C:\\out\\shot-hero---wide-2.jpeg", "C:\\out\\shot-Artboard-3.jpeg"]);
}

#[test]
fn bad_artboard_choices_error() {
    let mut s = session(20.0, 20.0, 3);
    for p in [
        json!({"useArtboards": true, "range": ""}),
        json!({"useArtboards": true, "range": "4"}),
        json!({"useArtboards": true, "artboards": []}),
        json!({"useArtboards": "yes"}),
        json!({"format": "pdf", "useArtboards": "yes"}),
    ] {
        assert!(s.execute("document.export", &p).is_err(), "{p}");
    }
}

#[test]
fn without_artboards_the_export_covers_the_visible_art() {
    let mut s = session(200.0, 100.0, 1);
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 10, "width": 30, "height": 20})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 90, "y": 40, "width": 40, "height": 35})).unwrap();
    let hidden = s.execute("shape.rectangle", &json!({"x": 150, "y": 0, "width": 40, "height": 99})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [hidden]})).unwrap();
    s.execute("object.hide", &json!({})).unwrap();
    // The union of the two visible rectangles: 20..130 × 10..75.
    let png = b64(&s.execute("document.export", &json!({"format": "png", "useArtboards": false})).unwrap());
    let img = rgba(&png);
    assert_eq!(img.dimensions(), (110, 65));
    assert_eq!(img.get_pixel(0, 0)[3], 255, "the art starts at the corner");
    assert_eq!(
        rgba(&b64(&s.execute("document.export", &json!({"format": "png", "useArtboards": false, "ppi": 144})).unwrap())).dimensions(),
        (220, 130)
    );
    // A PDF takes the art's bounds too (SVG has its own `useArtboards` option).
    let pdf = vectorcraft_pdf::import(&b64(&s.execute("document.export", &json!({"format": "pdf", "useArtboards": false})).unwrap())).unwrap();
    let page = pdf.artboards[0].rect;
    assert_eq!((page.width().round(), page.height().round()), (110.0, 65.0));
    let mut empty = session(50.0, 50.0, 1);
    let e = empty.execute("document.export", &json!({"format": "png", "useArtboards": false})).unwrap_err().to_string();
    assert!(e.contains("no visible art"), "{e}");
}

#[test]
fn formats_list_use_artboards() {
    let mut s = Session::new();
    let r = s.execute("document.formats", &json!({})).unwrap();
    let opts = |id: &str| r["formats"].as_array().unwrap().iter().find(|f| f["id"] == id).unwrap()["options"].clone();
    for id in ["svg", "pdf", "png", "jpg", "webp"] {
        assert!(opts(id).get("useArtboards").is_some(), "{id} lists useArtboards");
        assert!(opts(id).get("range").is_some(), "{id} lists range");
    }
    assert!(opts("vectorcraft").get("useArtboards").is_none());
}

/// An export's reply gives what the file loses before its notes on the options, whatever order
/// the encoder gave them: the WebP encoder notes that lossy WebP isn't applied before the export
/// notes the type it drew in the fallback font. With only notes on the options, they keep their
/// order.
#[test]
fn what_the_file_loses_comes_before_a_note_on_the_options() {
    let lossy = "lossy WebP isn't available yet: the file is lossless (exact, but larger)";
    let warnings = |s: &mut Session, p: Value| -> Vec<String> {
        let r = s.execute("document.export", &p).unwrap();
        r["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).map(str::to_string).collect()
    };
    let mut s = session(60.0, 40.0, 1);
    assert_eq!(warnings(&mut s, json!({"format": "webp", "lossless": false})), [lossy]);
    s.execute("text.create", &json!({"x": 5, "y": 20, "text": "Ab", "font": "Nonexistent Sans Probe"})).unwrap();
    let w = warnings(&mut s, json!({"format": "webp", "lossless": false}));
    assert_eq!(w.len(), 2, "{w:?}");
    assert!(w[0].contains("fallback font") && w[0].contains("Nonexistent Sans Probe"), "{w:?}");
    assert_eq!(w[1], lossy);
    // document.serialize gives them in the same order.
    let r = s.execute("document.serialize", &json!({"format": "webp", "lossless": false})).unwrap();
    assert_eq!(r["warnings"], json!(w));
}

#[test]
fn note_texts_give_the_losses_first_each_kind_in_order() {
    use super::encode::{Note, note_texts};
    let notes = [Note::setting("s1"), Note::loss("l1"), Note::setting("s2"), Note::loss("l2")];
    assert_eq!(note_texts(&notes), ["l1", "l2", "s1", "s2"]);
    assert_eq!(note_texts(&[Note::setting("s1"), Note::setting("s2")]), ["s1", "s2"]);
    assert!(note_texts(&[]).is_empty());
}
