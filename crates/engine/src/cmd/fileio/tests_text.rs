//! Export As Text, WebP options and Save for Office Documents.

use serde_json::{Value, json};

use super::*;

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

fn text(s: &mut Session, x: f64, y: f64, t: &str) -> u64 {
    s.execute("text.create", &json!({"x": x, "y": y, "text": t})).unwrap()["id"].as_u64().unwrap()
}

#[test]
fn text_export_writes_the_stories_in_stacking_order() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    text(&mut s, 10.0, 200.0, "Back story\nsecond line");
    let front = text(&mut s, 10.0, 40.0, "Front");
    // A thread of two frames is one story, written once.
    let a =
        s.execute("text.create", &json!({"x": 10, "y": 80, "text": "Threaded words", "area": {"width": 120, "height": 40}})).unwrap()["id"].clone();
    let b = s.execute("text.create", &json!({"x": 200, "y": 80, "text": "", "area": {"width": 120, "height": 40}})).unwrap()["id"].clone();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    // Hidden objects are left out.
    let hidden = text(&mut s, 10.0, 260.0, "Hidden");
    s.execute("select.set", &json!({"ids": [hidden]})).unwrap();
    s.execute("object.hide", &json!({})).unwrap();

    let export = |s: &mut Session, p: Value| b64(&s.execute("document.export", &merge(json!({"format": "txt"}), p)).unwrap());
    let utf8 = String::from_utf8(export(&mut s, json!({}))).unwrap();
    assert_eq!(utf8, "Back story\nsecond line\nFront\nThreaded words\n", "back to front, one line ending after each line");
    let crlf = export(&mut s, json!({"lineEndings": "crlf"}));
    assert_eq!(String::from_utf8(crlf).unwrap(), utf8.replace('\n', "\r\n"));
    let utf16 = export(&mut s, json!({"encoding": "utf16"}));
    assert_eq!(&utf16[..2], [0xFF, 0xFE], "byte order mark");
    let units: Vec<u16> = utf16[2..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    assert_eq!(String::from_utf16(&units).unwrap(), utf8);
    // Only the selection.
    s.execute("select.set", &json!({"ids": [front]})).unwrap();
    assert_eq!(export(&mut s, json!({"selectionOnly": true})), b"Front\n");
    for bad in [json!({"encoding": "latin1"}), json!({"lineEndings": "cr"})] {
        assert!(s.execute("document.export", &merge(json!({"format": "txt"}), bad.clone())).is_err(), "{bad}");
    }
    // A .txt path picks the format.
    let dir = std::env::temp_dir().join(format!("vc-text-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("story.txt");
    assert_eq!(s.execute("document.export", &json!({"path": path.to_string_lossy()})).unwrap()["format"], "txt");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), utf8);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn webp_is_lossless_and_says_so_when_lossy_is_asked_for() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 30, "height": 20})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap();
    let r = s.execute("document.export", &json!({"format": "webp"})).unwrap();
    let file = b64(&r);
    assert_eq!((&file[..4], &file[8..12], &file[12..16]), (&b"RIFF"[..], &b"WEBP"[..], &b"VP8L"[..]), "lossless bitstream");
    assert_eq!(r["warnings"], json!([]));
    let r = s.execute("document.export", &json!({"format": "webp", "lossless": false, "quality": 60, "background": "black"})).unwrap();
    assert_eq!(&b64(&r)[12..16], b"VP8L");
    assert!(r["warnings"][0].as_str().unwrap().contains("lossless"), "{r}");
    assert_eq!(image::load_from_memory(&b64(&r)).unwrap().to_rgba8().get_pixel(29, 19).0, [0, 0, 0, 255]);
}

#[test]
fn save_for_office_writes_a_png_at_the_resolution() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 612, "height": 792, "artboards": 2})).unwrap();
    let r = s.execute("document.exportForOffice", &json!({"ppi": 150})).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(1275), Some(1650)), "Letter at 150 ppi");
    let img = image::load_from_memory(&b64(&r)).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (1275, 1650));
    assert_eq!(img.get_pixel(0, 0).0, [255, 255, 255, 255], "on white");
    let r = s.execute("document.exportForOffice", &json!({"ppi": 72, "transparent": true, "artboard": 1})).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(612), Some(792)));
    assert_eq!(image::load_from_memory(&b64(&r)).unwrap().to_rgba8().get_pixel(0, 0)[3], 0, "transparent");
    assert!(s.execute("document.exportForOffice", &json!({"artboard": 2})).is_err());
    assert!(s.execute("document.exportForOffice", &json!({"ppi": -3})).is_err());
    let spec = crate::cmd::find_command("document.exportForOffice").unwrap();
    assert_eq!((spec.label, spec.menu), ("Save for Office Documents…", &["File"][..]));
}

#[test]
fn save_for_office_says_when_an_image_or_a_font_is_missing() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    assert_eq!(s.execute("document.exportForOffice", &json!({"ppi": 72})).unwrap()["warnings"], json!([]));
    // The renderer can't decode the placed image, and the type's family isn't installed.
    crate::tests_menucmds::place_undecodable_png(&mut s);
    s.execute("text.create", &json!({"x": 5, "y": 20, "text": "Ab", "font": "Nonexistent Sans Probe"})).unwrap();
    let r = s.execute("document.exportForOffice", &json!({"ppi": 72})).unwrap();
    let warnings: Vec<&str> = r["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings.contains(&"an image couldn't be decoded and is missing from the picture"), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("fallback font") && w.contains("Nonexistent Sans Probe")), "{warnings:?}");
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(200), Some(100)), "the PNG is still written");
}
