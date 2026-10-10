//! PSD export through `document.export`: the options reach the file (layers, groups, names, colour
//! model), impossible ones are refused, and `document.formats` lists the format.

use serde_json::{Value, json};

use super::*;

/// A 60 × 40 pt document: a rectangle on Layer 1, and the text "Hello" on "Words".
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 60, "height": 40})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 5, "y": 5, "width": 20, "height": 20})).unwrap();
    s.execute("layer.new", &json!({"name": "Words"})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 30, "text": "Hello"})).unwrap();
    s
}

fn export(s: &mut Session, p: Value) -> Vec<u8> {
    let r = s.execute("document.export", &merge(json!({"format": "psd"}), p)).unwrap();
    vectorcraft_format::base64_decode(r["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

/// The colour mode, the layer count and the layers' names, bottom first.
fn layers(f: &[u8]) -> (u16, i16, Vec<String>) {
    assert_eq!(&f[..4], b"8BPS");
    let be16 = |at: usize| u16::from_be_bytes([f[at], f[at + 1]]);
    let be32 = |at: usize| u32::from_be_bytes(f[at..at + 4].try_into().unwrap()) as usize;
    let mut at = 26;
    at += 4 + be32(at);
    at += 4 + be32(at);
    let count = if be32(at) == 0 { 0 } else { be16(at + 8) as i16 };
    // Every record's Unicode name block.
    let names = f
        .windows(8)
        .enumerate()
        .filter(|(_, w)| w == b"8BIMluni")
        .map(|(i, _)| {
            let n = be32(i + 12);
            String::from_utf16(&(0..n).map(|k| be16(i + 16 + k * 2)).collect::<Vec<_>>()).unwrap()
        })
        .collect();
    (be16(24), count, names)
}

#[test]
fn psd_writes_layers_groups_and_text_named_after_it() {
    let mut s = session();
    let (mode, count, names) = layers(&export(&mut s, json!({})));
    assert_eq!((mode, count, names), (3, -2, vec!["Layer 1".to_string(), "Words".to_string()]), "a pixel layer per top-level layer");

    let (_, count, names) = layers(&export(&mut s, json!({"maxEditability": true})));
    assert_eq!(count, -6);
    assert_eq!(names, ["</Layer group>", "<Rectangle>", "Layer 1", "</Layer group>", "Hello", "Words"], "groups, and text named after its text");

    let (mode, count, names) = layers(&export(&mut s, json!({"layers": false, "colorModel": "cmyk"})));
    assert_eq!((mode, count, names.len()), (4, 0, 0), "flat CMYK");
    assert_eq!(layers(&export(&mut s, json!({"colorModel": "gray"}))).0, 1);

    // A white background is a Background layer under the others.
    let (_, count, names) = layers(&export(&mut s, json!({"background": "white"})));
    assert_eq!((count, names[0].as_str()), (3, "Background"));

    // Hidden layers: left out, unless asked for.
    let words = s.doc().unwrap().doc.layers[1].id;
    s.execute("layer.setProps", &json!({"id": words.0, "visible": false})).unwrap();
    assert_eq!(layers(&export(&mut s, json!({}))).2, ["Layer 1"]);
    assert_eq!(layers(&export(&mut s, json!({"hiddenLayers": true}))).2, ["Layer 1", "Words"]);

    assert!(s.execute("document.export", &json!({"format": "psd", "colorModel": "lab"})).is_err());
}

#[test]
fn psd_is_listed_with_its_options_and_writes_one_file_per_artboard() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 30, "height": 20, "artboards": 2})).unwrap();
    let r = s.execute("document.formats", &json!({})).unwrap();
    let psd = r["formats"].as_array().unwrap().iter().find(|f| f["id"] == "psd").unwrap().clone();
    assert_eq!(
        (psd["label"].as_str(), psd["write"].as_bool(), psd["read"].as_bool(), psd["raster"].as_bool()),
        (Some("PSD"), Some(true), Some(true), Some(true))
    );
    for k in ["colorModel", "layers", "maxEditability", "hiddenLayers", "embedIcc", "ppi", "antiAlias", "background"] {
        assert!(psd["options"].get(k).is_some(), "psd takes {k}");
    }
    let r = s.execute("document.export", &json!({"format": "psd", "useArtboards": true})).unwrap();
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(files, ["Untitled-1-Artboard-1.psd", "Untitled-1-Artboard-2.psd"]);
    // One artboard, as bytes.
    let r = s.execute("document.serialize", &json!({"format": "psd", "artboard": 1})).unwrap();
    assert_eq!(&vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()[..4], b"8BPS");
    // 32000 pixels a side: within raster exports' limits, beyond the format's.
    s.execute("file.new", &json!({"width": 500, "height": 10})).unwrap();
    let err = s.execute("document.export", &json!({"format": "psd", "scale": 64})).unwrap_err().to_string();
    assert!(err.contains("too large for PSD"), "{err}");
}
