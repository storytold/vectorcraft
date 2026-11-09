//! Native and `.ai` save options: each artboard to a separate file (a master file too), Include
//! Linked Files, Embed ICC Profiles, Create PDF-Compatible File and the `.ai` Use Compression.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::cms;

use crate::tests_links::{Folder, png};
use crate::*;

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
    let plain = images(&s.execute("file.saveCopy", &json!({"compress": false})).unwrap());
    let (_, image) = plain.as_object().unwrap().iter().next().unwrap();
    assert_eq!(image["proxy"], true);
    // Include Linked Files: the file's own bytes.
    let r = s.execute("file.saveCopy", &json!({"compress": false, "includeLinked": true})).unwrap();
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
    let file = bytes_of(&s.execute("file.saveCopy", &json!({"compress": false})).unwrap()["dataBase64"]);
    let v: Value = serde_json::from_slice(&file).unwrap();
    assert_eq!(vectorcraft_format::base64_decode(v["profiles"][&name]["data"].as_str().unwrap()).unwrap(), icc.to_vec());
    // Not embedded when asked not to; built-in profiles never are (they are everywhere).
    let none: Value =
        serde_json::from_slice(&bytes_of(&s.execute("file.saveCopy", &json!({"compress": false, "embedProfiles": false})).unwrap()["dataBase64"]))
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
    let file = bytes_of(&s.execute("file.saveCopy", &json!({"pdfCompatible": true})).unwrap()["dataBase64"]);
    let pdf = vectorcraft_format::pdf_content(&file).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    let info = s.execute("document.pdfInfo", &json!({"dataBase64": vectorcraft_format::base64_encode(&pdf)})).unwrap();
    assert_eq!(info["pages"], 3);
    assert!(
        vectorcraft_format::pdf_content(&bytes_of(&s.execute("file.saveCopy", &json!({"pdfCompatible": false})).unwrap()["dataBase64"])).is_none()
    );
}

/// A native save is portable to the other apps by default. The option is still an explicit
/// opt-out, and a remembered opt-out survives later saves of the document.
#[test]
fn native_saves_carry_all_artboards_as_pdf_by_default() {
    let mut s = three_boards();
    for compress in [false, true] {
        let result = s.execute("file.saveCopy", &json!({"compress": compress})).unwrap();
        let file = bytes_of(&result["dataBase64"]);
        let pdf = vectorcraft_format::pdf_content(&file).expect("the default native save carries a PDF");
        let info = s.execute("document.pdfInfo", &json!({"dataBase64": vectorcraft_format::base64_encode(&pdf)})).unwrap();
        assert_eq!(info["pages"], 3);
        assert_eq!(vectorcraft_format::load(&file).unwrap().artboards.len(), 3);
    }
    let dir = Folder::new("pdf-opt-out");
    let path = dir.file("compact.vectorcraft");
    s.execute("file.saveAs", &json!({"path": path, "pdfCompatible": false})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 60, "y": 10, "width": 10, "height": 10})).unwrap();
    s.execute("document.save", &json!({})).unwrap();
    assert!(vectorcraft_format::pdf_content(&std::fs::read(&path).unwrap()).is_none());
    let options = s.execute("file.formatOptions", &json!({"format":"vectorcraft"})).unwrap();
    assert_eq!(options["options"]["pdfCompatible"]["value"], false);
    s.execute("document.save", &json!({"compress": true})).unwrap();
    assert!(vectorcraft_format::pdf_content(&std::fs::read(&path).unwrap()).is_none());
    s.execute("document.open", &json!({"path": path})).unwrap();
    assert_eq!(s.doc().unwrap().save_options["pdfCompatible"], false);
    let options = s.execute("file.formatOptions", &json!({"format": "vectorcraft"})).unwrap();
    assert_eq!(options["options"]["pdfCompatible"]["value"], false);
    s.execute("document.save", &json!({})).unwrap();
    assert!(vectorcraft_format::pdf_content(&std::fs::read(&path).unwrap()).is_none());
    s.execute("document.save", &json!({"pdfCompatible": true})).unwrap();
    assert!(vectorcraft_format::pdf_content(&std::fs::read(&path).unwrap()).is_some());
}

#[test]
fn native_saves_without_artboards_keep_the_editable_document() {
    let mut s = three_boards();
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).artboards.clear();
    let result = s.execute("file.saveCopy", &json!({})).unwrap();
    let file = bytes_of(&result["dataBase64"]);
    let restored = vectorcraft_format::load(&file).unwrap();
    assert!(restored.artboards.is_empty());
    assert_eq!(restored.node_count(), s.doc().unwrap().doc.node_count());
    assert!(vectorcraft_format::pdf_content(&file).is_none());
    s.execute("document.open", &json!({"name": "no-artboards.vectorcraft", "dataBase64": vectorcraft_format::base64_encode(&file)})).unwrap();
    assert_eq!(s.doc().unwrap().save_options["pdfCompatible"], true);
    s.execute("artboard.new", &json!({"x": 0, "y": 0, "width": 100, "height": 80})).unwrap();
    let result = s.execute("file.saveCopy", &json!({})).unwrap();
    assert!(vectorcraft_format::pdf_content(&bytes_of(&result["dataBase64"])).is_some());
}

#[test]
fn native_template_saves_keep_the_document_pdf_preference() {
    let mut s = three_boards();
    let dir = Folder::new("template-pdf-preference");
    s.execute("file.saveAs", &json!({"path": dir.file("compact.vectorcraft"), "pdfCompatible": false, "version": 3})).unwrap();
    let options = s.execute("file.formatOptions", &json!({"format": "template"})).unwrap();
    assert_eq!(options["options"]["pdfCompatible"]["value"], false, "the save dialog must offer the same retained choice");
    let r = s.execute("file.saveAsTemplate", &json!({"format": "template"})).unwrap();
    let bytes = bytes_of(&r["dataBase64"]);
    assert!(vectorcraft_format::pdf_content(&bytes).is_none());
    assert_eq!(
        vectorcraft_format::load_file(&bytes).unwrap().info.version,
        vectorcraft_format::VERSION,
        "unrelated format options are not carried over"
    );
    let r = s.execute("file.saveAsTemplate", &json!({"format": "template", "pdfCompatible": true})).unwrap();
    assert!(vectorcraft_format::pdf_content(&bytes_of(&r["dataBase64"])).is_some());
}

#[test]
fn a_default_native_save_keeps_its_pdf_after_another_document_opts_out() {
    let dir = Folder::new("default-native-pdf-choice");
    for (index, options) in [json!({}), json!({"options":{"pdfCompatible":null}})].into_iter().enumerate() {
        let mut s = three_boards();
        let path = dir.file(&format!("first-{index}.vectorcraft"));
        let mut params = options.as_object().unwrap().clone();
        params.insert("path".into(), json!(path));
        s.execute("file.saveAs", &Value::Object(params)).unwrap();
        assert!(vectorcraft_format::pdf_content(&std::fs::read(&path).unwrap()).is_some());
        s.execute("file.new", &json!({"width": 10, "height": 10})).unwrap();
        s.execute("file.saveAs", &json!({"path":dir.file(&format!("other-{index}.vectorcraft")),"pdfCompatible":false})).unwrap();
        s.execute("document.activate", &json!({"index":0})).unwrap();
        s.execute("document.save", &json!({})).unwrap();
        assert!(vectorcraft_format::pdf_content(&std::fs::read(&path).unwrap()).is_some(), "the first document keeps the PDF it was saved with");
        assert_eq!(s.doc().unwrap().save_options.get("pdfCompatible"), Some(&json!(true)));
    }
}

#[test]
fn a_null_nested_pdf_option_keeps_the_saved_opt_out() {
    let dir = Folder::new("null-nested-pdf-option");
    let mut s = three_boards();
    let path = dir.file("compact.vectorcraft");
    s.execute("file.saveAs", &json!({"path": path, "pdfCompatible": false})).unwrap();
    s.execute("document.save", &json!({"options": {"pdfCompatible": null}})).unwrap();
    assert!(vectorcraft_format::pdf_content(&std::fs::read(&path).unwrap()).is_none(), "null leaves the saved choice");
    assert_eq!(s.doc().unwrap().save_options.get("pdfCompatible"), Some(&json!(false)));
}

#[test]
fn reopening_native_files_keeps_each_files_pdf_compatibility() {
    let dir = Folder::new("pdf-reopened-options");
    let mut s = three_boards();
    let pdf = dir.file("portable.vectorcraft");
    let compact = dir.file("compact.vectorcraft");
    s.execute("file.saveAs", &json!({"path": pdf, "pdfCompatible": true})).unwrap();
    s.execute("file.saveAs", &json!({"path": compact, "pdfCompatible": false})).unwrap();
    s.execute("file.new", &json!({"width": 20, "height": 20})).unwrap();
    let copy = s.execute("file.saveCopy", &json!({})).unwrap();
    assert!(vectorcraft_format::pdf_content(&bytes_of(&copy["dataBase64"])).is_none(), "new files use the last native preference");
    for (path, expected) in [(&pdf, true), (&compact, false)] {
        s.execute("document.open", &json!({"path": path})).unwrap();
        let options = s.execute("file.formatOptions", &json!({"format": "vectorcraft"})).unwrap();
        assert_eq!(options["options"]["pdfCompatible"]["value"], expected);
        s.execute("document.save", &json!({})).unwrap();
        assert_eq!(vectorcraft_format::pdf_content(&std::fs::read(path).unwrap()).is_some(), expected);
    }
}

#[test]
fn reverting_native_files_updates_their_pdf_compatibility() {
    let dir = Folder::new("pdf-revert-options");
    for saved in [false, true] {
        let mut s = three_boards();
        let path = dir.file(if saved { "with-pdf.vectorcraft" } else { "compact.vectorcraft" });
        s.execute("file.saveAs", &json!({"path": path, "pdfCompatible": saved})).unwrap();
        let replacement = s.execute("file.saveCopy", &json!({"pdfCompatible": !saved})).unwrap();
        std::fs::write(&path, bytes_of(&replacement["dataBase64"])).unwrap();
        s.execute("shape.rectangle", &json!({"x": 70, "y": 20, "width": 8, "height": 8})).unwrap();
        s.execute("file.revert", &json!({})).unwrap();
        let options = s.execute("file.formatOptions", &json!({"format": "vectorcraft"})).unwrap();
        assert_eq!(options["options"]["pdfCompatible"]["value"], !saved);
        s.execute("document.save", &json!({})).unwrap();
        assert_eq!(vectorcraft_format::pdf_content(&std::fs::read(&path).unwrap()).is_some(), !saved);
    }
}

#[test]
fn save_option_errors_name_the_save_command() {
    let mut s = three_boards();
    for (cmd, p) in [
        ("file.saveCopy", json!({"format": "pdf", "range": "5"})),
        ("file.saveCopy", json!({"format": "svg", "artboard": 5})),
        ("file.saveAs", json!({"format": "svg", "range": "4"})),
        ("document.save", json!({"format": "pdf", "artboards": [3]})),
        ("file.saveCopy", json!({"format": "ai", "compress": "yes"})),
        ("file.saveCopy", json!({"format": "ai", "pdfCompatible": false, "artboard": "B"})),
        ("file.saveCopy", json!({"format": "ai", "aiVersion": 7, "artboard": "B"})),
        ("file.saveCopy", json!({"format": "ait", "compress": "yes"})),
    ] {
        let e = s.execute(cmd, &p).unwrap_err().to_string();
        assert!(e.contains(&format!("`{cmd}`")) && !e.contains("document.export"), "{p}: {e}");
    }
    // A native file written to a path.
    let dir = Folder::new("save-errors");
    for (cmd, name) in [("document.save", "a.vectorcraft"), ("file.saveAs", "b.vectorcraft"), ("file.saveCopy", "c.vectorcraft")] {
        let p = json!({"path": dir.file(name), "compress": "yes"});
        let e = s.execute(cmd, &p).unwrap_err().to_string();
        assert!(e.contains(&format!("`{cmd}`")) && !e.contains("document.export"), "{p}: {e}");
    }
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
    let says = |w: &Value, text: &str| w.as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains(text));
    assert!(says(&warnings, "without PDF content") && says(&warnings, "apps that read its editing data open it"), "{warnings}");
    // Without the editing copy, other apps show empty pages.
    let (_, plain) = ai(&mut s, json!({"pdfCompatible": false, "editingData": false}));
    assert!(says(&plain, "VectorCraft opens it as before") && !says(&plain, "editing data"), "{plain}");
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
fn ai_option_errors_name_a_file_saved_without_pdf_content() {
    let mut s = three_boards();
    for p in
        [json!({"format": "ai", "pdfCompatible": false, "artboard": "2"}), json!({"format": "ai", "pdfCompatible": false, "includeLinked": "yes"})]
    {
        let e = s.execute("file.saveCopy", &p).unwrap_err().to_string();
        assert!(e.contains("non-PDF-compatible .ai options: invalid type"), "{p}: {e}");
    }
    let e = s.execute("document.export", &json!({"format": "ai", "pdfCompatible": false, "artboard": "2"})).unwrap_err().to_string();
    assert!(e.contains("`document.export`") && e.contains("non-PDF-compatible .ai options: invalid type"), "{e}");
    let p = json!({"format": "ait", "pdfCompatible": false, "includeLinked": "yes"});
    let e = s.execute("file.saveAsTemplate", &p).unwrap_err().to_string();
    assert!(e.contains("non-PDF-compatible .ait template options: invalid type"), "{e}");
    // A template saved without PDF content is written.
    let r = s.execute("file.saveAsTemplate", &json!({"format": "ait", "pdfCompatible": false})).unwrap();
    assert!(r["warnings"].to_string().contains("without PDF content"), "{r}");
    // So are the notes on native options the file doesn't read.
    let r = s.execute("file.saveCopy", &json!({"format": "ai", "pdfCompatible": false, "version": 3})).unwrap();
    assert!(r["warnings"].to_string().contains("`version` was not used: non-PDF-compatible .ai files don't read it"), "{r}");
    // A PDF-compatible file keeps its name.
    let e = s.execute("file.saveCopy", &json!({"format": "ai", "includeLinked": "yes"})).unwrap_err().to_string();
    assert!(e.contains("PDF-compatible .ai options: invalid type") && !e.contains("non-PDF"), "{e}");
    let r = s.execute("file.saveCopy", &json!({"format": "ai", "version": 3})).unwrap();
    assert!(r["warnings"].to_string().contains("`version` was not used: PDF-compatible .ai files don't read it"), "{r}");
}

/// The message of a refused command's error.
fn refusal(s: &mut Session, cmd: &str, p: &Value) -> String {
    match s.execute(cmd, p) {
        Err(EngineError::BadParams { cmd: c, msg }) if c == cmd => msg,
        other => panic!("{cmd} {p}: {:?}", other.map(|r| r["bytes"].clone())),
    }
}

#[test]
fn an_ai_save_refuses_an_artboard_the_document_does_not_have_as_an_export_does() {
    let dir = Folder::new("ai-no-artboard");
    let mut s = three_boards();
    let path = dir.file("x.ai");
    let missing = [
        json!({"artboards": [5]}),
        json!({"artboards": [0, 3]}),
        json!({"artboard": 3}),
        json!({"range": "4"}),
        json!({"separateArtboards": true, "artboards": [5]}),
    ];
    let wrong_type = [json!({"artboard": "x"}), json!({"artboards": ["x"]}), json!({"range": 42})];
    for pdf in [true, false] {
        for pick in missing.iter().chain(&wrong_type) {
            let mut p = json!({"format": "ai", "pdfCompatible": pdf});
            p.as_object_mut().unwrap().extend(pick.as_object().unwrap().clone());
            let export = refusal(&mut s, "document.export", &p);
            p["path"] = json!(path);
            for cmd in ["file.saveCopy", "file.saveAs", "document.save"] {
                assert_eq!(refusal(&mut s, cmd, &p), export, "{cmd} {p}");
            }
            // A template is not exported: its errors name the template where they name the format.
            p["format"] = json!("ait");
            let template = refusal(&mut s, "file.saveAsTemplate", &p);
            if missing.contains(pick) {
                assert_eq!(template, export, "{p}");
            } else {
                assert!(template.contains("invalid type"), "{p}: {template}");
            }
        }
    }
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0, "nothing is written");
    // An artboard the document has, an empty `artboards` list, or `artboards` that a `range`
    // overrides is saved with every artboard, as before.
    for pick in [json!({"artboards": [1]}), json!({"artboards": []}), json!({"range": "1", "artboards": [5]})] {
        let mut p = json!({"format": "ai"});
        p.as_object_mut().unwrap().extend(pick.as_object().unwrap().clone());
        let r = s.execute("file.saveCopy", &p).unwrap();
        let mut o = Session::new();
        o.execute("document.open", &json!({"name": "x.ai", "dataBase64": r["dataBase64"]})).unwrap();
        assert_eq!(o.doc().unwrap().doc.artboards.len(), 3, "{p}");
    }
}

#[test]
fn a_remembered_artboard_the_document_no_longer_has_is_dropped() {
    let dir = Folder::new("remembered-artboards");
    for (name, p) in [
        ("legacy.ai", json!({"aiVersion": "7", "range": "3"})),
        ("separate.ai", json!({"separateArtboards": true, "range": "3"})),
        ("separate.vectorcraft", json!({"separateArtboards": true, "range": "1, 3"})),
        ("doc.pdf", json!({"range": "3"})),
        ("doc.svg", json!({"artboard": 2})),
    ] {
        let mut s = three_boards();
        let mut first = p.clone();
        first["path"] = json!(dir.file(name));
        s.execute("file.saveAs", &first).unwrap_or_else(|e| panic!("{name}: {e}"));
        s.execute("artboard.delete", &json!({"index": 2})).unwrap();
        let v = s.execute("file.formatOptions", &json!({})).unwrap();
        for k in ["artboard", "range"].iter().filter(|k| p.get(**k).is_some()) {
            assert_ne!(v["options"][*k]["value"], p[*k], "{name}: Save no longer uses {k}");
        }
        s.execute("document.save", &json!({})).unwrap_or_else(|e| panic!("{name}: {e}"));
        // Given to the save, it is still refused.
        let e = s.execute("document.save", &p).unwrap_err().to_string();
        assert!(e.contains("from 1 to 2") || e.contains("no artboard 2"), "{name}: {e}");
    }
    // A remembered artboard that is still there is kept.
    let mut s = three_boards();
    s.execute("file.saveAs", &json!({"path": dir.file("kept.ai"), "aiVersion": "7", "range": "2"})).unwrap();
    s.execute("artboard.delete", &json!({"index": 2})).unwrap();
    let v = s.execute("file.formatOptions", &json!({})).unwrap();
    assert_eq!(v["options"]["range"]["value"], json!("2"));
    let r = s.execute("document.save", &json!({})).unwrap();
    assert!(r["path"].as_str().is_some_and(|p| p.ends_with("kept.ai")), "{r}");
}

#[test]
fn a_remembered_artboard_pick_follows_the_artboards_it_named() {
    let dir = Folder::new("remembered-pick");
    // Save each artboard of A, B and C to a file with `range`, change the artboards, then save
    // without options: the range Save now uses and the artboards it writes, by file name.
    let save = |range: &str, change: &[(&str, Value)]| -> (Value, Vec<String>) {
        let mut s = three_boards();
        let path = dir.file("x.ai");
        s.execute("file.saveAs", &json!({"path": path, "separateArtboards": true, "range": range})).unwrap();
        for (cmd, p) in change {
            s.execute(cmd, p).unwrap();
        }
        let range = s.execute("file.formatOptions", &json!({})).unwrap()["options"]["range"]["value"].clone();
        let r = s.execute("document.save", &json!({})).unwrap();
        let files = r["files"].as_array().unwrap().iter().skip(1).map(|f| f.as_str().unwrap().rsplit(['/', '\\']).next().unwrap().to_string());
        (range, files.collect())
    };
    let delete = |i: usize| ("artboard.delete", json!({"index": i}));
    assert_eq!(save("1, 3", &[delete(2)]), (json!("1"), vec!["x-A.ai".to_string()]));
    assert_eq!(save("1, 3", &[delete(0)]), (json!("2"), vec!["x-C.ai".to_string()]));
    assert_eq!(save("1-3", &[delete(1)]), (json!("1-2"), vec!["x-A.ai".to_string(), "x-C.ai".to_string()]));
    let (range, mut files) = save("1, 3", &[("artboard.reorder", json!({"index": 2, "to": 0}))]);
    files.sort();
    assert_eq!((range, files), (json!("2, 1"), vec!["x-A.ai".to_string(), "x-C.ai".to_string()]));
    // None of them left: every artboard, as when none is named.
    assert_eq!(save("1, 3", &[delete(2), delete(0)]), (Value::Null, vec!["x-B.ai".to_string()]));
    assert_eq!(save("all", &[delete(0)]), (json!("all"), vec!["x-B.ai".to_string(), "x-C.ai".to_string()]));
    // An artboard made after one was deleted is not taken for it.
    let new = ("artboard.new", json!({"x": 600, "y": 0, "width": 100, "height": 100, "name": "D"}));
    let all = ["x-A.ai", "x-B.ai", "x-D.ai"].map(String::from).to_vec();
    assert_eq!(save("3", &[delete(2), new.clone()]), (Value::Null, all));
    assert_eq!(save("1, 3", &[delete(2), new.clone()]), (json!("1"), vec!["x-A.ai".to_string()]));
    // A range open at its end keeps to the artboards it named.
    assert_eq!(save("2-", std::slice::from_ref(&new)), (json!("2-3"), vec!["x-B.ai".to_string(), "x-C.ai".to_string()]));
    // Nor is one made after undoing the artboard (and the three rectangles made after it).
    let undo = ("edit.undo", json!({}));
    let undone = [undo.clone(), undo.clone(), undo.clone(), undo, new];
    assert_eq!(save("1, 3", &undone), (json!("1"), vec!["x-A.ai".to_string()]));

    // The other saves that remember a pick: a PDF's range, an SVG's artboard, a legacy .ai's
    // range and a native file's separate files.
    for (name, p, gone, after) in [
        ("doc.pdf", json!({"range": "1, 3"}), 0, json!({"range": "2"})),
        // `range` wins over `artboard`, which a save then does not use.
        ("unused.pdf", json!({"artboard": 0, "range": "3"}), 0, json!({"range": "2"})),
        ("doc.svg", json!({"artboard": 2}), 0, json!({"artboard": 1})),
        ("legacy.ai", json!({"aiVersion": "7", "range": "1, 3"}), 2, json!({"range": "1"})),
        ("doc.vectorcraft", json!({"separateArtboards": true, "range": "1, 3"}), 0, json!({"range": "2"})),
    ] {
        let mut s = three_boards();
        let mut first = p.clone();
        first["path"] = json!(dir.file(name));
        s.execute("file.saveAs", &first).unwrap();
        s.execute("artboard.delete", &json!({"index": gone})).unwrap();
        let v = s.execute("file.formatOptions", &json!({})).unwrap();
        for (k, value) in after.as_object().unwrap() {
            assert_eq!(&v["options"][k]["value"], value, "{name}: {k}");
        }
        s.execute("document.save", &json!({})).unwrap_or_else(|e| panic!("{name}: {e}"));
        if name.ends_with(".pdf") {
            let bytes = std::fs::read(dir.file(name)).unwrap();
            let info = s.execute("document.pdfInfo", &json!({"dataBase64": vectorcraft_format::base64_encode(&bytes)})).unwrap();
            assert_eq!(info["pages"], 1, "only the artboard that was 3");
        }
    }

    // File > Revert: a PDF holds only the pages it was saved with, so its pick is left out; a
    // native file keeps every artboard, and the pick stays.
    let mut s = three_boards();
    s.execute("file.saveAs", &json!({"path": dir.file("revert.pdf"), "range": "1, 3"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 50, "width": 30, "height": 30})).unwrap();
    s.execute("file.revert", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.artboards.len(), 2);
    assert_eq!(s.execute("file.formatOptions", &json!({})).unwrap()["options"]["range"]["value"], Value::Null);
    // A PDF without the editable document reads back with new ids in page order: after a reorder,
    // the pick is left out, not taken for another reorder.
    let mut s = three_boards();
    s.execute("artboard.reorder", &json!({"index": 2, "to": 0})).unwrap();
    s.execute("file.saveAs", &json!({"path": dir.file("reordered.pdf"), "range": "1-3", "preserveEditing": false})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 50, "width": 30, "height": 30})).unwrap();
    s.execute("file.revert", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.artboards.iter().map(|a| a.id).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(s.execute("file.formatOptions", &json!({})).unwrap()["options"]["range"]["value"], Value::Null);
    let mut s = three_boards();
    s.execute("file.saveAs", &json!({"path": dir.file("revert.vectorcraft"), "separateArtboards": true, "range": "1, 3"})).unwrap();
    s.execute("artboard.delete", &json!({"index": 0})).unwrap();
    s.execute("file.revert", &json!({})).unwrap();
    assert_eq!(s.execute("file.formatOptions", &json!({})).unwrap()["options"]["range"]["value"], json!("1, 3"));
}

#[test]
fn format_options_list_the_save_options() {
    let mut s = three_boards();
    let v = s.execute("file.formatOptions", &json!({"format": "ai"})).unwrap();
    for (k, value) in [("separateArtboards", false), ("includeLinked", false), ("embedProfiles", true), ("pdfCompatible", true), ("compress", true)] {
        assert_eq!(v["options"][k]["value"], value, "{k}");
    }
    let v = s.execute("file.formatOptions", &json!({"format": "vectorcraft"})).unwrap();
    assert_eq!((v["options"]["pdfCompatible"]["value"].as_bool(), v["options"]["range"]["value"].is_null()), (Some(true), true));
    // Saved with them, a document remembers them for Save.
    let dir = Folder::new("remember");
    let path = dir.file("r.vectorcraft");
    s.execute("document.save", &json!({"path": path, "includeLinked": true})).unwrap();
    let v = s.execute("file.formatOptions", &json!({})).unwrap();
    assert_eq!(v["options"]["includeLinked"]["value"], true);
}

/// The anchors of the first object in the native file at `path`.
fn saved_anchors(path: &str) -> Vec<vectorcraft_geom::Point> {
    let d = vectorcraft_format::load(&std::fs::read(path).unwrap()).unwrap();
    let NodeKind::Path { path, .. } = &d.layers[0].children().unwrap()[0].kind else { panic!("not a path") };
    path.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p)).collect()
}

#[test]
fn precision_rounds_the_paths_of_that_save_only() {
    let dir = Folder::new("precision");
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 1000, "height": 1000})).unwrap();
    // 1,000 anchors at coordinates with many decimal places (a fixed pseudo-random sequence).
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (seed >> 11) as f64 / (1u64 << 53) as f64 * 1000.0
    };
    let anchors: Vec<Value> = (0..1000).map(|_| json!({"x": next(), "y": next()})).collect();
    s.execute("path.create", &json!({"anchors": anchors})).unwrap();
    let (exact, rounded) = (dir.file("exact.vectorcraft"), dir.file("rounded.vectorcraft"));
    s.execute("document.save", &json!({"path": exact, "compress": false})).unwrap();
    s.execute("document.save", &json!({"path": rounded, "compress": false, "precision": 3})).unwrap();
    let (e, r) = (std::fs::metadata(&exact).unwrap().len(), std::fs::metadata(&rounded).unwrap().len());
    assert!(r < e, "{r} bytes rounded, {e} exact");
    let (want, got) = (saved_anchors(&exact), saved_anchors(&rounded));
    assert_eq!(got.len(), 1000);
    for (w, g) in want.iter().zip(&got) {
        assert!((w.x - g.x).abs() <= 0.0005 && (w.y - g.y).abs() <= 0.0005, "{w:?} saved as {g:?}");
        assert_eq!(((g.x * 1000.0).round() / 1000.0, (g.y * 1000.0).round() / 1000.0), (g.x, g.y));
    }
    // The document keeps its exact anchors, and Save doesn't remember the precision.
    let v = s.execute("file.formatOptions", &json!({})).unwrap();
    assert!(v["options"]["precision"]["value"].is_null(), "{}", v["options"]["precision"]);
    s.execute("document.save", &json!({})).unwrap();
    assert_eq!(saved_anchors(&rounded), want);
    // A precision that isn't a whole number from 0 to 9 is refused, by the command that was given it.
    for (wrong, got) in [(json!(10), "10"), (json!(2.5), "2.5"), (json!(-1), "-1"), (json!("4"), "\"4\""), (json!(true), "true")] {
        for cmd in ["file.saveCopy", "document.save"] {
            let e = s.execute(cmd, &json!({"precision": wrong})).unwrap_err().to_string();
            assert!(e.contains(&format!("`{cmd}`")) && !e.contains("document.export"), "{cmd} {wrong}: {e}");
            assert!(e.contains(&format!("`precision` must be a whole number between 0 and 9 (got {got})")), "{cmd} {wrong}: {e}");
        }
    }
    // A type error in another native option names that command too.
    let e = s.execute("document.save", &json!({"version": "4"})).unwrap_err().to_string();
    assert!(e.contains("`document.save`") && !e.contains("document.export"), "{e}");
}

#[test]
fn precision_is_not_remembered_for_new_documents() {
    let dir = Folder::new("precision-preferences");
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("document.save", &json!({"path": dir.file("rounded.vectorcraft"), "precision": 3})).unwrap();
    assert!(!s.prefs.native_save_options.contains_key("precision"));

    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let options = s.execute("file.formatOptions", &json!({"format": "vectorcraft"})).unwrap();
    assert!(options["options"]["precision"]["value"].is_null(), "{}", options["options"]["precision"]);
}

#[test]
fn a_ai_file_ignores_precision_as_save_does() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("path.create", &json!({"anchors": [{"x": 10.123456789, "y": 20.987654321}, {"x": 60.5, "y": 70.25}]})).unwrap();
    let want: Vec<_> = {
        let NodeKind::Path { path, .. } = &s.doc().unwrap().doc.layers[0].children().unwrap()[0].kind else { panic!("not a path") };
        path.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p)).collect()
    };
    for cmd in ["document.export", "file.saveCopy"] {
        for precision in [json!(3), json!(10)] {
            let r = s.execute(cmd, &json!({"format": "ai", "precision": precision})).unwrap();
            let mut t = Session::new();
            t.execute("document.open", &json!({"name": "x.ai", "dataBase64": r["dataBase64"]})).unwrap();
            let NodeKind::Path { path, .. } = &t.doc().unwrap().doc.layers[0].children().unwrap()[0].kind else { panic!("not a path") };
            let got: Vec<_> = path.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p)).collect();
            assert_eq!(got, want, "{cmd} {precision}");
        }
    }
}

/// The anchors of every path in `n` and the art it holds.
fn all_anchors(n: &vectorcraft_doc::Node, out: &mut Vec<vectorcraft_geom::Point>) {
    if let NodeKind::Path { path, .. } = &n.kind {
        out.extend(path.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p)));
    }
    for c in n.children().into_iter().flatten() {
        all_anchors(c, out);
    }
}

#[test]
fn the_preview_and_pdf_of_a_save_at_a_precision_show_the_rounded_art() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let anchors = json!([{"x": 10.4, "y": 20.3}, {"x": 80.6, "y": 30.7}, {"x": 45.35, "y": 85.65}]);
    s.execute("path.create", &json!({"anchors": anchors, "closed": true})).unwrap();
    let file = bytes_of(&s.execute("file.saveCopy", &json!({"precision": 0, "pdfCompatible": true, "preview": true})).unwrap()["dataBase64"]);
    // The embedded PDF, opened, has the path on whole points.
    let pdf = vectorcraft_format::pdf_content(&file).unwrap();
    let mut t = Session::new();
    t.execute("document.open", &json!({"name": "x.pdf", "dataBase64": vectorcraft_format::base64_encode(&pdf)})).unwrap();
    let mut got = vec![];
    for l in &t.doc().unwrap().doc.layers {
        all_anchors(l, &mut got);
    }
    for (x, y) in [(10.0, 20.0), (81.0, 31.0), (45.0, 86.0)] {
        assert!(got.iter().any(|p| (p.x - x).abs() < 1e-3 && (p.y - y).abs() < 1e-3), "({x}, {y}) not in {got:?}");
    }
    assert!(got.iter().all(|p| (p.x - p.x.round()).abs() < 1e-3 && (p.y - p.y.round()).abs() < 1e-3), "{got:?}");
    // The preview is the one the saved document draws, not the exact art's.
    let preview = vectorcraft_format::preview(&file);
    assert!(preview.is_some());
    assert_eq!(preview, crate::cmd::fileio::preview_png(&vectorcraft_format::load(&file).unwrap()).unwrap());
    assert_ne!(preview, crate::cmd::fileio::preview_png(&s.doc().unwrap().doc).unwrap());
}
