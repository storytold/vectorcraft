//! Native files: `document.save {compress, version, preview}`, the Use Compression preference,
//! atomic writes, unused images left out, keys from newer versions and colour profiles kept.

use std::path::PathBuf;

use serde_json::{Value, json};
use vectorcraft_format::{is_compressed, sniff};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    s
}

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vc-native-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn bytes_of(r: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(r["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

/// The file `document.save` (no path) gives back with `p`.
/// A native save (new documents save as .ai by default).
fn saved(s: &mut Session, mut p: Value) -> Vec<u8> {
    p["format"] = json!("vectorcraft");
    bytes_of(&s.execute("document.save", &p).unwrap())
}

fn open(s: &mut Session, name: &str, bytes: &[u8]) -> Value {
    s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)})).unwrap()
}

/// The file's JSON (unpacked when compressed).
fn json_of(bytes: &[u8]) -> Value {
    let mut text = Vec::new();
    if is_compressed(bytes) {
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(bytes), &mut text).unwrap();
    } else {
        text = bytes.to_vec();
    }
    serde_json::from_slice(&text).unwrap()
}

#[test]
fn compressed_saves_open_by_their_content() {
    let mut s = session();
    let packed = saved(&mut s, json!({"compress": true}));
    assert!(is_compressed(&packed) && sniff(&packed));
    let plain = saved(&mut s, json!({}));
    assert!(!is_compressed(&plain) && packed.len() < plain.len());
    // Recognised by its content, whatever the name says.
    for name in ["a.vectorcraft", "a.drawcraft", "a.svgz", "a.bin"] {
        let r = open(&mut s, name, &packed);
        assert_eq!(r["format"], "vectorcraft", "{name}");
        assert_eq!(s.doc().unwrap().doc.node_count(), 2);
    }
}

#[test]
fn use_compression_preference_sets_the_default() {
    let mut s = session();
    s.execute("prefs.set", &json!({"key": "useCompression", "value": true})).unwrap();
    assert!(is_compressed(&saved(&mut s, json!({}))));
    assert!(!is_compressed(&saved(&mut s, json!({"compress": false}))), "the param wins");
    // Older versions can't read compressed files: those saves stay plain.
    assert!(!is_compressed(&saved(&mut s, json!({"version": 2}))));
    assert!(s.execute("document.save", &json!({"format": "vectorcraft", "version": 2, "compress": true})).is_err());
    assert!(is_compressed(&bytes_of(&s.execute("file.saveAsTemplate", &json!({})).unwrap())));
    // Exports write what they're told.
    let r = s.execute("document.serialize", &json!({"format": "vectorcraft"})).unwrap();
    assert!(!is_compressed(&bytes_of(&r)));
}

#[test]
fn saves_for_older_versions() {
    let mut s = session();
    for v in [1, 2] {
        let bytes = saved(&mut s, json!({"version": v}));
        let j = json_of(&bytes);
        assert_eq!((j["format"].as_str(), j["version"].as_u64()), (Some("drawcraft"), Some(v)));
        open(&mut s, "old.drawcraft", &bytes);
        assert_eq!(s.doc().unwrap().doc.node_count(), 2);
    }
    for bad in [
        json!({"format": "vectorcraft", "version": 0}),
        json!({"format": "vectorcraft", "version": 9}),
        json!({"format": "vectorcraft", "version": "two"}),
    ] {
        assert!(s.execute("document.save", &bad).is_err(), "{bad}");
    }
    let formats = s.execute("document.formats", &json!({})).unwrap();
    let native = formats["formats"].as_array().unwrap().iter().find(|f| f["id"] == "vectorcraft").unwrap();
    for o in ["compress", "version", "preview"] {
        assert!(native["options"].get(o).is_some(), "{o} documented");
    }
}

#[test]
fn preview_is_at_most_256_pixels() {
    let mut s = session();
    let bytes = saved(&mut s, json!({"preview": true, "compress": true}));
    let png = vectorcraft_format::preview(&bytes).expect("a preview");
    let img = image::load_from_memory(&png).unwrap();
    assert_eq!((img.width(), img.height()), (256, 128), "the 400×200 artboard fitted into 256 px");
    assert!(vectorcraft_format::preview(&saved(&mut s, json!({}))).is_none(), "off by default");
}

#[test]
fn saving_writes_through_a_temporary_file() {
    let d = tmp_dir("atomic");
    let path = d.join("doc.vectorcraft");
    std::fs::write(&path, b"an older version of the file").unwrap();
    let mut s = session();
    s.execute("document.save", &json!({"path": path.to_string_lossy()})).unwrap();
    assert!(sniff(&std::fs::read(&path).unwrap()));
    let names: Vec<String> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    assert_eq!(names, ["doc.vectorcraft"], "no temporary file left behind");
    assert!(!s.doc().unwrap().is_dirty());
    // A save that can't be written fails, and the document stays modified.
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    let missing = d.join("missing").join("doc.vectorcraft");
    assert!(s.execute("document.save", &json!({"path": missing.to_string_lossy()})).is_err());
    assert!(s.doc().unwrap().is_dirty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn unused_images_are_left_out_of_saves() {
    let mut s = session();
    let mut png = vec![];
    image::RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let r = s.execute("file.place", &json!({"name": "a.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
    let id = r["ids"][0].as_u64().unwrap();
    assert_eq!(json_of(&saved(&mut s, json!({})))["images"].as_object().unwrap().len(), 1);
    s.execute("edit.clear", &json!({"ids": [id]})).unwrap();
    assert_eq!(s.doc().unwrap().doc.images.len(), 1, "kept in memory (undo can bring the image back)");
    assert!(json_of(&saved(&mut s, json!({})))["images"].as_object().unwrap().is_empty());
    s.execute("edit.undo", &json!({})).unwrap();
    let bytes = saved(&mut s, json!({}));
    open(&mut s, "back.vectorcraft", &bytes);
    assert_eq!(s.doc().unwrap().doc.images.len(), 1);
}

#[test]
fn newer_keys_and_legacy_profiles_survive_open_and_save() {
    let mut s = session();
    let mut j = json_of(&saved(&mut s, json!({"version": 2})));
    j["document"]["fromTheFuture"] = json!({"x": 1});
    j["document"]["unknown"] = json!({"colorProfiles": {"rgb": "sRGB IEC61966-2.1", "cmyk": null}});
    open(&mut s, "old.vectorcraft", &serde_json::to_vec(&j).unwrap());
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.color_profiles.rgb.as_deref(), Some("sRGB IEC61966-2.1"), "profiles migrate");
    assert!(d.unknown.is_empty());
    assert_eq!(cmd::colormgmt::doc_profiles(d).0.as_deref(), Some("sRGB IEC61966-2.1"));
    let again = json_of(&saved(&mut s, json!({})));
    assert_eq!(again["document"]["fromTheFuture"], json!({"x": 1}));
    assert_eq!(again["document"]["color_profiles"]["rgb"], "sRGB IEC61966-2.1");
    // Saved for v2 again: back where v2 apps read them.
    let v2 = json_of(&saved(&mut s, json!({"version": 2})));
    assert_eq!(v2["document"]["unknown"]["colorProfiles"]["rgb"], "sRGB IEC61966-2.1");
}
