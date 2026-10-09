//! Native and `.ai` save options: each artboard to a separate file (a master file too), Include
//! Linked Files, Embed ICC Profiles, Create PDF-Compatible File and the `.ai` Use Compression.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::cms;

use super::*;
use crate::tests_links::{Folder, png};

/// Three artboards (named A, B and C) with a rectangle on each.
fn three_boards() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 0, "name": "A"})).unwrap();
    for (i, name) in ["B", "C"].iter().enumerate() {
        let x = 200.0 * (i + 1) as f64;
        s.execute("artboard.new", &json!({"x": x, "y": 0, "width": 100, "height": 100, "name": name})).unwrap();
    }
    for x in [10, 210, 410] {
        s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 30, "height": 30})).unwrap();
    }
    s
}

fn objects(d: &vectorcraft_doc::Document) -> usize {
    d.layers.iter().map(|l| l.count() - 1).sum()
}

fn bytes_of(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v.as_str().expect("base64")).unwrap()
}

#[test]
fn three_artboards_give_four_files() {
    let dir = Folder::new("separate");
    let mut s = three_boards();
    let path = dir.file("Poster.vectorcraft");
    let r = s.execute("document.save", &json!({"path": path, "separateArtboards": true})).unwrap();
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    assert_eq!(files, [path.clone(), dir.file("Poster-A.vectorcraft"), dir.file("Poster-B.vectorcraft"), dir.file("Poster-C.vectorcraft")]);
    // The master file has everything; each artboard's file that artboard and its art.
    let master = vectorcraft_format::load(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!((master.artboards.len(), objects(&master)), (3, 3));
    for (f, name, x) in [(files[1], "A", 0.0), (files[2], "B", 200.0), (files[3], "C", 400.0)] {
        let d = vectorcraft_format::load(&std::fs::read(f).unwrap()).unwrap();
        assert_eq!((d.artboards.len(), d.artboards[0].name.as_str(), d.artboards[0].rect.x0), (1, name, x));
        assert_eq!(objects(&d), 1, "{name}");
    }
    // The document takes the master file; Save remembers the choice.
    let st = s.doc().unwrap();
    assert_eq!((st.path.as_deref(), st.is_dirty()), (Some(path.as_str()), false));
    // A range: the master and two artboards.
    let r = s.execute("document.save", &json!({"path": path, "separateArtboards": true, "range": "2-3"})).unwrap();
    assert_eq!(r["files"].as_array().unwrap().len(), 3);
    assert!(s.execute("document.save", &json!({"path": path, "separateArtboards": true, "range": "4"})).is_err(), "no fourth artboard");
    // No path: the bytes of every file.
    let r = s.execute("file.saveCopy", &json!({"separateArtboards": true})).unwrap();
    let names: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Poster copy.vectorcraft", "Poster copy-A.vectorcraft", "Poster copy-B.vectorcraft", "Poster copy-C.vectorcraft"]);
    assert!(vectorcraft_format::sniff(&bytes_of(&r["files"][3]["dataBase64"])));
}

#[test]
fn ai_files_save_each_artboard_too_and_reopen_editable() {
    let dir = Folder::new("separate-ai");
    let mut s = three_boards();
    let path = dir.file("Set.ai");
    let r = s.execute("file.saveAs", &json!({"path": path, "separateArtboards": true, "range": "1, 3"})).unwrap();
    assert_eq!(r["files"], json!([path, dir.file("Set-A.ai"), dir.file("Set-C.ai")]));
    let r = s.execute("document.open", &json!({"path": dir.file("Set-C.ai")})).unwrap();
    assert_eq!(r["restored"], true);
    let d = &s.doc().unwrap().doc;
    assert_eq!((d.artboards.len(), d.artboards[0].name.as_str(), objects(d)), (1, "C", 1));
}

#[test]
fn a_linked_pngs_bytes_are_embedded() {
    let dir = Folder::new("include-linked");
    let pic = dir.file("photo.png");
    let bytes = png(400, 300, [200, 30, 30]);
    std::fs::write(&pic, &bytes).unwrap();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 500, "height": 500})).unwrap();
    s.execute("file.place", &json!({"path": pic, "link": true})).unwrap();
    let images = |v: &Value| -> Value {
        let text = bytes_of(&v["dataBase64"]);
        serde_json::from_slice::<Value>(&text).unwrap()["images"].clone()
    };
    // By default only the preview.
    let plain = images(&s.execute("file.saveCopy", &json!({"format": "vectorcraft", "compress": false})).unwrap());
    let (_, image) = plain.as_object().unwrap().iter().next().unwrap();
    assert_eq!(image["proxy"], true);
    // Include Linked Files: the file's own bytes.
    let r = s.execute("file.saveCopy", &json!({"format": "vectorcraft", "compress": false, "includeLinked": true})).unwrap();
    let full = images(&r);
    let (_, image) = full.as_object().unwrap().iter().next().unwrap();
    assert!(image.get("proxy").is_none());
    assert_eq!(vectorcraft_format::base64_decode(image["data"].as_str().unwrap()).unwrap(), bytes);
    // With the file gone the document still shows the full pixels.
    std::fs::remove_file(&pic).unwrap();
    let r = s.execute("document.open", &json!({"name": "copy.vectorcraft", "dataBase64": r["dataBase64"]})).unwrap();
    assert_eq!(r["missingLinks"].as_array().unwrap().len(), 1);
    let d = &s.doc().unwrap().doc;
    let blob = d.images.values().next().unwrap();
    assert_eq!(blob.bytes.as_slice(), bytes.as_slice());
    let mut linked = 0;
    d.visit_images(|_, im| linked += usize::from(im.link.is_some()));
    assert_eq!(linked, 1, "still linked");
}

#[test]
fn an_embedded_profile_is_restored() {
    let builtin = cms::profiles().into_iter().find(|p| p.kind == cms::ProfileKind::Rgb && p.builtin && p.name != cms::SRGB).unwrap();
    let icc = cms::icc_bytes(&builtin.name).unwrap();
    let name = format!("Studio RGB {}", std::process::id());
    cms::register_icc(&icc, Some(name.clone())).unwrap();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    Arc::make_mut(&mut s.doc_mut().unwrap().doc).color_profiles.rgb = Some(name.clone());
    let file = bytes_of(&s.execute("file.saveCopy", &json!({"format": "vectorcraft", "compress": false})).unwrap()["dataBase64"]);
    let v: Value = serde_json::from_slice(&file).unwrap();
    assert_eq!(vectorcraft_format::base64_decode(v["profiles"][&name]["data"].as_str().unwrap()).unwrap(), icc.to_vec());
    // Not embedded when asked not to; built-in profiles never are (they are everywhere).
    let none: Value = serde_json::from_slice(&bytes_of(
        &s.execute("file.saveCopy", &json!({"format": "vectorcraft", "compress": false, "embedProfiles": false})).unwrap()["dataBase64"],
    ))
    .unwrap();
    assert!(none.get("profiles").is_none());
    // Elsewhere (a profile name this machine lacks): opening installs it and the document keeps it.
    let elsewhere = format!("Studio RGB elsewhere {}", std::process::id());
    let moved = String::from_utf8(file).unwrap().replace(&name, &elsewhere);
    assert!(cms::profile(&elsewhere).is_none());
    let r =
        s.execute("document.open", &json!({"name": "moved.vectorcraft", "dataBase64": vectorcraft_format::base64_encode(moved.as_bytes())})).unwrap();
    assert!(r["warnings"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(cms::profile(&elsewhere).map(|p| p.kind), Some(cms::ProfileKind::Rgb));
    assert_eq!(s.doc().unwrap().doc.color_profiles.rgb.as_deref(), Some(elsewhere.as_str()));
    // Damaged profile data opens the document with a warning.
    let broken = format!("Studio RGB broken {}", std::process::id());
    let mut v: Value = serde_json::from_str(&moved.replace(&elsewhere, &broken)).unwrap();
    v["profiles"][&broken]["data"] = json!("AAAA");
    let r = s
        .execute(
            "document.open",
            &json!({"name": "b.vectorcraft", "dataBase64": vectorcraft_format::base64_encode(&serde_json::to_vec(&v).unwrap())}),
        )
        .unwrap();
    assert!(r["warnings"][0].as_str().unwrap().contains(&broken), "{r}");
}

#[test]
fn pdf_compatible_files_carry_a_pdf() {
    let mut s = three_boards();
    let file = bytes_of(&s.execute("file.saveCopy", &json!({"format": "vectorcraft", "pdfCompatible": true})).unwrap()["dataBase64"]);
    let pdf = vectorcraft_format::pdf_content(&file).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    let info = s.execute("document.pdfInfo", &json!({"dataBase64": vectorcraft_format::base64_encode(&pdf)})).unwrap();
    assert_eq!(info["pages"], 3);
    assert!(
        vectorcraft_format::pdf_content(&bytes_of(&s.execute("file.saveCopy", &json!({"format": "vectorcraft"})).unwrap()["dataBase64"])).is_none()
    );
}

#[test]
fn ai_without_pdf_content_and_compression() {
    let mut s = three_boards();
    let ai = |s: &mut Session, p: Value| -> (Vec<u8>, Value) {
        let mut p = p;
        p["format"] = json!("ai");
        let r = s.execute("file.saveCopy", &p).unwrap();
        (bytes_of(&r["dataBase64"]), r["warnings"].clone())
    };
    let (full, _) = ai(&mut s, json!({}));
    let (blank, warnings) = ai(&mut s, json!({"pdfCompatible": false}));
    assert!(warnings.as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("without PDF content")), "{warnings}");
    // The pages are empty (an import of a page finds no art), the document comes back whole.
    let page = |s: &mut Session, b: &[u8]| {
        s.execute("document.open", &json!({"name": "x.ai", "dataBase64": vectorcraft_format::base64_encode(b), "pages": "1"})).unwrap();
        objects(&s.doc().unwrap().doc)
    };
    assert!(page(&mut s, &full) > 0);
    assert_eq!(page(&mut s, &blank), 0);
    s.execute("document.open", &json!({"name": "x.ai", "dataBase64": vectorcraft_format::base64_encode(&blank)})).unwrap();
    assert_eq!(objects(&s.doc().unwrap().doc), 3);
    // Use Compression off writes the content uncompressed: a larger file.
    let mut s = three_boards();
    let (packed, _) = ai(&mut s, json!({"compress": true}));
    let (loose, _) = ai(&mut s, json!({"compress": false}));
    assert!(loose.len() > packed.len(), "{} vs {}", loose.len(), packed.len());
}

#[test]
fn format_options_list_the_save_options() {
    let mut s = three_boards();
    let v = s.execute("file.formatOptions", &json!({"format": "ai"})).unwrap();
    for (k, value) in [("separateArtboards", false), ("includeLinked", false), ("embedProfiles", true), ("pdfCompatible", true), ("compress", true)] {
        assert_eq!(v["options"][k]["value"], value, "{k}");
    }
    let v = s.execute("file.formatOptions", &json!({"format": "vectorcraft"})).unwrap();
    assert_eq!((v["options"]["pdfCompatible"]["value"].as_bool(), v["options"]["range"]["value"].is_null()), (Some(false), true));
    // Saved with them, a document remembers them for Save.
    let dir = Folder::new("remember");
    let path = dir.file("r.vectorcraft");
    s.execute("document.save", &json!({"path": path, "includeLinked": true})).unwrap();
    let v = s.execute("file.formatOptions", &json!({})).unwrap();
    assert_eq!(v["options"]["includeLinked"]["value"], true);
}
