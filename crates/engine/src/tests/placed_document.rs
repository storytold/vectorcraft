//! Placed documents: a VectorCraft document placed linked (`file.place`'s default for one read
//! from a file), drawn and output as vectors, saved with its preview, checked, updated and
//! relinked when the file changes, and turned into an editable copy (Break Link, Expand). A PDF
//! or `.ai` page places the same way.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeKind, PlacedDocument};
use vectorcraft_geom::{Rect, shapes};

use crate::tests_links::{BLUE, Folder, RED, centre_colour, near, open, save, session, write};
use crate::*;

/// A VectorCraft document at `path`: one `w`×`h` artboard filled with `rgb` (and `extra` more
/// rectangles, which change the file's size).
fn source(path: &str, w: f64, h: f64, rgb: [u8; 3], extra: usize) {
    let mut d = Document::new(w, h);
    let layer = d.layers[0].id;
    let [r, g, b] = rgb;
    for i in 0..=extra {
        let id = d.alloc_id();
        let look = Appearance::basic(Paint::solid(Color::rgb8(r, g, b)), Paint::None, 0.0);
        d.insert(Some(layer), i, Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, w, h)), look)).unwrap();
    }
    write(path, &vectorcraft_format::save_file(&d));
}

/// A document drawing symbol `Star` (a blue 100×50 rectangle) at `path`, titled `title` (tests
/// that remove the file give it content of its own: files read are cached by content).
fn star_source(path: &str, title: &str) {
    let mut d = Document::new(100.0, 50.0);
    d.title = title.into();
    let layer = d.layers[0].id;
    let art = Node::path(vectorcraft_doc::NodeId(900), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 50.0)), look(BLUE));
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Star".into(), art: std::sync::Arc::new(art) });
    let id = d.alloc_id();
    d.insert(Some(layer), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "Star".into(), xf: vectorcraft_geom::Affine::IDENTITY })).unwrap();
    write(path, &vectorcraft_format::save_file(&d));
}

fn look(c: [u8; 3]) -> Appearance {
    Appearance::basic(Paint::solid(Color::rgb8(c[0], c[1], c[2])), Paint::None, 0.0)
}

/// The active document gets a red symbol named `Star` of its own.
fn parent_star(s: &mut Session) {
    let star = Node::path(vectorcraft_doc::NodeId(901), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), look(RED));
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).symbols.push(vectorcraft_doc::Symbol {
        dynamic: false,
        name: "Star".into(),
        art: std::sync::Arc::new(star),
    });
}

/// Place `path` (linked: the default), centred on the artboard → its id.
fn place(s: &mut Session, path: &str) -> NodeId {
    let r = s.execute("file.place", &json!({"path": path, "at": [200, 150]})).unwrap();
    assert_eq!(r["linked"], true, "{r}");
    NodeId(r["ids"][0].as_u64().unwrap())
}

fn placed(s: &Session, id: NodeId) -> PlacedDocument {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::PlacedDocument(p) => (**p).clone(),
        k => panic!("not a placed document: {k:?}"),
    }
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

fn decode(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()
}

/// Images and paths in `bytes`, a PDF, read back.
fn pdf_contents(bytes: &[u8]) -> (usize, usize) {
    // The page as drawn (not the VectorCraft document the PDF carries).
    let (doc, _) = crate::cmd::fileio::page_document(bytes, 0, &Default::default()).unwrap();
    let (mut images, mut paths) = (0, 0);
    doc.walk(|n| match n.kind {
        NodeKind::Image(_) => images += 1,
        NodeKind::Path { .. } | NodeKind::Compound { .. } => paths += 1,
        _ => {}
    });
    (images, paths)
}

/// The JSON of a `.vectorcraft` file (the data after it left out).
fn file_json(bytes: &[u8]) -> Value {
    let end = bytes.windows(vectorcraft_format::BLOB_MAGIC.len()).position(|w| w == vectorcraft_format::BLOB_MAGIC).unwrap_or(bytes.len());
    serde_json::from_slice(&bytes[..end]).unwrap()
}

#[test]
fn a_document_places_linked_by_default_at_its_artboard_size() {
    let dir = Folder::new("placed-basic");
    let src = dir.file("logo.vectorcraft");
    source(&src, 100.0, 50.0, RED, 0);
    let mut s = session();
    let id = place(&mut s, &src);
    let p = placed(&s, id);
    assert!((p.width - 100.0).abs() < 1e-6 && (p.height - 50.0).abs() < 1e-6, "{p:?}");
    assert_eq!(bounds(&s, id), Rect::new(150.0, 125.0, 250.0, 175.0));
    assert_eq!((p.link.path.as_str(), p.link.page), (src.as_str(), Some(1)));
    let n = s.doc().unwrap().doc.node(id).unwrap();
    assert_eq!((n.name.as_deref(), n.kind_label()), (Some("logo.vectorcraft"), "Placed Document"));
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
    // It moves and scales like any object.
    s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    assert_eq!(bounds(&s, id), Rect::new(160.0, 125.0, 260.0, 175.0));
    // Its art's bounds instead of the artboard.
    let mut d = Document::new(100.0, 50.0);
    let layer = d.layers[0].id;
    let id2 = d.alloc_id();
    d.insert(Some(layer), 0, Node::path(id2, shapes::rectangle(Rect::new(10.0, 10.0, 40.0, 30.0)), look(RED))).unwrap();
    let small = dir.file("small.vectorcraft");
    write(&small, &vectorcraft_format::save_file(&d));
    let r = s.execute("file.place", &json!({"path": small, "crop": "bounding", "at": [200, 150]})).unwrap();
    let p = placed(&s, NodeId(r["ids"][0].as_u64().unwrap()));
    assert!(p.bounding && (p.width - 30.0).abs() < 1e-6 && (p.height - 20.0).abs() < 1e-6, "{p:?}");
}

#[test]
fn without_link_or_a_file_a_document_places_as_an_editable_copy() {
    let dir = Folder::new("placed-copy");
    let src = dir.file("logo.vectorcraft");
    source(&src, 100.0, 50.0, RED, 0);
    let mut s = session();
    let data = vectorcraft_format::base64_encode(&std::fs::read(&src).unwrap());
    for p in [json!({"path": src, "link": false}), json!({"name": "logo.vectorcraft", "dataBase64": data})] {
        let r = s.execute("file.place", &p).unwrap();
        assert_eq!(r["linked"], false, "{r}");
        let n = s.doc().unwrap().doc.node(NodeId(r["ids"][0].as_u64().unwrap())).unwrap();
        assert_eq!(n.kind_label(), "Clip Group", "{p}");
    }
    assert!(!s.doc().unwrap().doc.has_placed());
}

#[test]
fn a_placed_document_is_vectors_in_every_output() {
    let dir = Folder::new("placed-outputs");
    let src = dir.file("logo.vectorcraft");
    source(&src, 100.0, 50.0, RED, 0);
    let mut s = session();
    place(&mut s, &src);
    let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap();
    let text = svg["text"].as_str().unwrap();
    assert!(!text.contains("<image") && (text.contains("<path") || text.contains("<rect")), "drawn as paths: {text}");
    let (images, paths) = pdf_contents(&decode(&s.execute("document.serialize", &json!({"format": "pdf"})).unwrap()));
    assert_eq!(images, 0, "no image in the PDF");
    assert!(paths > 0);
    let png = decode(&s.execute("document.serialize", &json!({"format": "png"})).unwrap());
    let [r, g, b, _] = image::load_from_memory(&png).unwrap().to_rgba8().get_pixel(200, 150).0;
    assert!(near([r, g, b], RED), "{:?}", [r, g, b]);
    for format in ["eps", "emf", "dxf"] {
        let out = s.execute("document.serialize", &json!({"format": format})).unwrap();
        assert!(out.to_string().len() > 200, "{format}: {out}");
    }
    let info = s.execute("document.info", &json!({})).unwrap();
    assert!(info.to_string().contains("\"placedDocuments\""), "{info}");
}

#[test]
fn the_preview_is_saved_so_the_document_shows_it_without_its_file() {
    let dir = Folder::new("placed-saved");
    let src = dir.file("logo.vectorcraft");
    // Content of its own: files read are cached by content, and this one is removed.
    source(&src, 100.0, 50.0, RED, 2);
    let mut s = session();
    let id = place(&mut s, &src);
    let doc_path = dir.file("poster/poster.vectorcraft");
    std::fs::create_dir_all(dir.file("poster")).unwrap();
    save(&mut s, &doc_path);
    let saved: Value = serde_json::from_slice(&std::fs::read(&doc_path).unwrap()).unwrap();
    let kind = &saved["document"]["layers"][0]["kind"]["children"][0]["kind"];
    assert_eq!(kind["type"], "placeddocument");
    assert_eq!(kind["link"]["relative"], "../logo.vectorcraft");
    let key = kind["key"].as_str().unwrap();
    let entry = &saved["images"][key];
    assert_eq!((entry["proxy"].as_bool(), entry["mime"].as_str()), (Some(true), Some("image/jpeg")), "the preview, as JPEG (opaque)");
    // Include Linked Files keeps the file itself.
    let full = file_json(&decode(&s.execute("document.serialize", &json!({"format": "vectorcraft", "includeLinked": true})).unwrap()));
    assert_eq!((full["images"][key]["proxy"].clone(), full["images"][key]["mime"].as_str()), (Value::Null, Some("application/json")));
    std::fs::remove_file(&src).unwrap();
    let r = open(&mut s, &doc_path);
    assert_eq!(r["missingLinks"].as_array().map(Vec::len), Some(1), "{r}");
    assert_eq!(placed(&s, id).link.path, src);
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED), "shown from the saved preview");
    assert_eq!(s.execute("links.check", &json!({})).unwrap()["missing"], 1);
    // Output with the file gone: the preview, with a warning.
    let png = s.execute("document.serialize", &json!({"format": "png"})).unwrap();
    assert!(png["warnings"].to_string().contains("1 placed document output as its preview"), "{png}");
    let [r, g, b, _] = image::load_from_memory(&decode(&png)).unwrap().to_rgba8().get_pixel(200, 150).0;
    assert!(near([r, g, b], RED), "{:?}", [r, g, b]);
    // Saved for older apps: the art it shows (here its preview), as plain objects.
    let old: Value =
        serde_json::from_slice(&decode(&s.execute("document.serialize", &json!({"format": "vectorcraft", "version": 2})).unwrap())).unwrap();
    let old = old["document"]["layers"][0]["kind"]["children"][0].to_string();
    assert!(old.contains("\"image\"") && !old.contains("placeddocument"), "{old}");
}

#[test]
fn ai_export_preserves_placed_links_while_drawing_full_art_or_previews() {
    for missing in [false, true] {
        let dir = Folder::new(&format!("placed-ai-editable-{missing}"));
        let src = dir.file("logo.vectorcraft");
        star_source(&src, &format!("placed-ai-editable-{missing}"));
        let mut s = session();
        let id = place(&mut s, &src);
        let parent = dir.file("parent.vectorcraft");
        save(&mut s, &parent);
        if missing {
            std::fs::remove_file(&src).unwrap();
        }
        open(&mut s, &parent);
        let original = placed(&s, id);
        for pdf_compatible in [true, false] {
            for (command, include_linked) in
                [("document.export", false), ("document.export", true), ("document.serialize", false), ("document.serialize", true)]
            {
                let out = s.execute(command, &json!({"format": "ai", "pdfCompatible": pdf_compatible, "includeLinked": include_linked})).unwrap();
                let bytes = decode(&out);
                let editing = vectorcraft_pdf::editing(&bytes).unwrap();
                let native = vectorcraft_format::load(&editing.data).unwrap();
                assert_eq!(
                    native.images[&original.key].is_proxy(),
                    missing || !include_linked,
                    "Include Linked Files retains available source bytes"
                );
                if pdf_compatible {
                    let (images, paths) = pdf_contents(&bytes);
                    if missing {
                        assert!(images > 0, "the missing file draws its preview");
                        assert!(out["warnings"].to_string().contains("1 placed document output as its preview"));
                    } else {
                        assert_eq!(images, 0, "the PDF uses the file's vectors");
                        assert!(paths > 0);
                    }
                }
                let mut restored = session();
                let result = restored
                    .execute("document.open", &json!({"name": "export.ai", "dataBase64": vectorcraft_format::base64_encode(&bytes)}))
                    .unwrap();
                assert_eq!(result["restored"], true);
                assert_eq!(placed(&restored, id), original, "link metadata stays editable");
                assert_eq!(placed(&s, id), original, "export leaves the source intact");
                if missing {
                    assert_eq!(restored.execute("links.check", &json!({})).unwrap()["missing"], 1);
                    let replacement = dir.file("replacement.vectorcraft");
                    source(&replacement, 100.0, 50.0, BLUE, 1);
                    let relinked = restored.execute("links.relink", &json!({"ids": [id.0], "path": replacement})).unwrap();
                    assert_eq!(relinked["relinked"], json!([id.0]));
                    assert_eq!(placed(&restored, id).link.path, replacement);
                }
            }
        }
    }
}

#[test]
fn ai_export_public_aliases_preserve_missing_placed_links() {
    let dir = Folder::new("placed-ai-public-aliases-latest");
    let src = dir.file("logo.vectorcraft");
    star_source(&src, "placed-ai-public-aliases-latest-unique");
    let mut s = session();
    let id = place(&mut s, &src);
    let parent = dir.file("parent.vectorcraft");
    save(&mut s, &parent);
    std::fs::remove_file(&src).unwrap();
    open(&mut s, &parent);
    let original = placed(&s, id);
    assert!(s.doc().unwrap().doc.images[&original.key].is_proxy());

    for alias in
        ["vectorcraft", ".vectorcraft", "VECTORCRAFT", "drawcraft", ".DRAWCRAFT", "template", "vctemplate", ".VCTEMPLATE", "ai", ".ai", "AI", ".AI"]
    {
        let canonical = cmd::fileio::format(alias).unwrap().id;
        let enc = cmd::fileio::encode_all(&s.doc().unwrap().doc, alias, &json!({"pdfCompatible": false, "includeLinked": false})).unwrap();
        let bytes = &enc.files[0].1;
        let raw = if canonical == "ai" { vectorcraft_pdf::editing(bytes).unwrap().data } else { bytes.clone() };
        let native = vectorcraft_format::load(&raw).unwrap();
        assert!(
            matches!(&native.node(id).unwrap().kind, NodeKind::PlacedDocument(p) if **p == original),
            "{alias}: preparation discarded the editable placed link"
        );
        assert!(native.images[&original.key].is_proxy(), "{alias}");
        assert_eq!(placed(&s, id), original, "{alias}: source changed");
    }
}

/// The `.ai` editing copy for other apps draws a missing placed file as its preview, with the
/// pages or without them, and says so.
#[test]
fn ai_editing_copy_warns_about_missing_placed_files() {
    let dir = Folder::new("placed-ai-editing-copy-missing");
    let src = dir.file("logo.vectorcraft");
    star_source(&src, "placed-ai-editing-copy-missing-unique");
    let mut s = session();
    place(&mut s, &src);
    let parent = dir.file("parent.vectorcraft");
    save(&mut s, &parent);
    std::fs::remove_file(&src).unwrap();
    open(&mut s, &parent);
    let doc = &s.doc().unwrap().doc;
    let (_, missing) = cmd::place::document::full_documents(doc);
    assert!(!missing.is_empty());
    for compatible in [true, false] {
        let enc = cmd::fileio::encode_all(doc, "ai", &json!({"pdfCompatible": compatible})).unwrap();
        for w in &missing {
            assert!(enc.warnings.contains(w), "pdfCompatible {compatible}: {w:?} not in {:?}", enc.warnings);
        }
    }
}

/// Include Linked Files must retain relinkable objects when only their previews are available.
#[test]
fn native_included_links_keep_missing_placed_documents_editable() {
    for missing in [true, false] {
        let dir = Folder::new(&format!("native-included-links-{missing}"));
        let src = dir.file("logo.vectorcraft");
        star_source(&src, &format!("native-included-links-{missing}"));
        let source_bytes = std::fs::read(&src).unwrap();
        let mut s = session();
        let id = place(&mut s, &src);
        let parent = decode(&s.execute("document.serialize", &json!({"format": "vectorcraft"})).unwrap());
        if missing {
            std::fs::remove_file(&src).unwrap();
        }
        s.execute("document.open", &json!({"name": "parent.vectorcraft", "dataBase64": vectorcraft_format::base64_encode(&parent)})).unwrap();
        let original = placed(&s, id);
        assert!(s.doc().unwrap().doc.images[&original.key].is_proxy());
        let before = s.execute("document.json", &json!({})).unwrap();
        for format in ["vectorcraft", "template"] {
            for command in ["document.serialize", "document.export", "document.save"] {
                for include_linked in [true, false] {
                    let out = s.execute(command, &json!({"format": format, "includeLinked": include_linked, "compress": false})).unwrap();
                    let bytes = decode(&out);
                    let native = vectorcraft_format::load(&bytes).unwrap();
                    assert_eq!(file_json(&bytes)["version"], vectorcraft_format::VERSION);
                    assert!(
                        matches!(&native.node(id).unwrap().kind, NodeKind::PlacedDocument(p) if **p == original),
                        "{command}/{format}/{missing}/{include_linked}: discarded the editable link"
                    );
                    let blob = &native.images[&original.key];
                    assert_eq!(blob.is_proxy(), missing || !include_linked);
                    if !missing && include_linked {
                        assert_eq!(blob.bytes.as_slice(), source_bytes.as_slice());
                    }
                    assert_eq!(s.execute("document.json", &json!({})).unwrap(), before);
                    if missing && include_linked {
                        let mut restored = Session::new();
                        restored
                            .execute(
                                "document.open",
                                &json!({"name": "included.vectorcraft", "dataBase64": vectorcraft_format::base64_encode(&bytes)}),
                            )
                            .unwrap();
                        assert_eq!(placed(&restored, id), original);
                        assert_eq!(restored.execute("links.check", &json!({})).unwrap()["missing"], 1);
                        let replacement = dir.file("replacement.vectorcraft");
                        source(&replacement, 100.0, 50.0, BLUE, 1);
                        assert_eq!(
                            restored.execute("links.relink", &json!({"ids": [id.0], "path": replacement})).unwrap()["relinked"],
                            json!([id.0])
                        );
                        assert_eq!(placed(&restored, id).link.path, replacement);
                    }
                }
            }
        }
    }
}

/// Output prepared by a caller that may not read the placed file shows its preview, not the full
/// art an earlier caller that could read it left cached.
#[test]
fn a_confined_export_does_not_reuse_full_art_read_for_another_caller() {
    let dir = Folder::new("placed-confined");
    let other = Folder::new("placed-confined-other");
    let src = dir.file("src.vectorcraft");
    source(&src, 100.0, 50.0, BLUE, 0);
    let mut s = session();
    place(&mut s, &src);
    // As a document saved with previews opens: each placed file's blob holds only its preview.
    for b in std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).images.values_mut() {
        if let Some(p) = b.proxy.clone() {
            b.bytes = p;
        }
    }
    let d = s.doc().unwrap().doc.clone();
    let preview = |d: &Document| d.images.keys().any(|k| k.starts_with(vectorcraft_doc::placed_document::RESOURCE_PREFIX) && k.ends_with("/preview"));
    assert!(!preview(&d.with_placed_art()), "the file is read in full");
    let roots = crate::file_access::AutomationRoots::new(&[&other.0], &[]).unwrap().unwrap();
    let confined = crate::file_access::confine(Some(&roots), || d.with_placed_art().into_owned());
    assert!(preview(&confined), "only the preview shows");
}

#[test]
fn a_document_saved_with_previews_outputs_its_files_art() {
    let dir = Folder::new("placed-preview-output");
    let src = dir.file("logo.vectorcraft");
    source(&src, 100.0, 50.0, RED, 0);
    let mut s = session();
    let id = place(&mut s, &src);
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    open(&mut s, &doc_path);
    let key = placed(&s, id).key;
    assert!(s.doc().unwrap().doc.images[&key].is_proxy(), "opened with the preview only");
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
    // Output reads the file again: vectors, no warning.
    let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap();
    assert_eq!(svg["warnings"], json!([]), "{}", svg["warnings"]);
    let text = svg["text"].as_str().unwrap();
    assert!(!text.contains("<image") && (text.contains("<path") || text.contains("<rect")), "vectors");
    // So does saving with Include Linked Files, and the document stays as it was.
    let full = file_json(&decode(&s.execute("document.serialize", &json!({"format": "vectorcraft", "includeLinked": true})).unwrap()));
    assert_eq!(full["images"][&key]["mime"], "application/json");
    assert!(s.doc().unwrap().doc.images[&key].is_proxy());
}

#[test]
fn a_changed_file_is_modified_and_update_links_reads_it_again() {
    let dir = Folder::new("placed-update");
    let src = dir.file("logo.vectorcraft");
    source(&src, 100.0, 50.0, RED, 0);
    let mut s = session();
    let id = place(&mut s, &src);
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    let before = bounds(&s, id);
    let c = s.execute("links.check", &json!({})).unwrap();
    assert_eq!((c["modified"].as_u64(), c["links"][0]["status"].as_str()), (Some(0), Some("ok")), "{c}");
    let list = s.execute("links.list", &json!({})).unwrap();
    let row = &list["links"][0];
    assert_eq!((row["format"].as_str(), row["document"].as_bool(), row["status"].as_str()), (Some("VectorCraft"), Some(true), Some("ok")), "{row}");
    // A blue document, wider, and a different file size.
    source(&src, 200.0, 50.0, BLUE, 1);
    assert_eq!(s.execute("links.check", &json!({})).unwrap()["links"][0]["status"], "modified");
    // Until updated, it shows (and outputs) the file as it read it.
    let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap();
    assert_eq!(svg["warnings"], json!([]));
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
    let u = s.execute("links.update", &json!({})).unwrap();
    assert_eq!(u["updated"], json!([id.0]), "{u}");
    assert_eq!(placed(&s, id).width, 200.0);
    assert_eq!(bounds(&s, id), before, "keeps its bounds");
    assert!(near(centre_colour(&s.doc().unwrap().doc), BLUE));
    assert_eq!(s.execute("links.check", &json!({})).unwrap()["modified"], 0);
    // One undo step.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
}

#[test]
fn update_links_on_open_reads_a_modified_file_again() {
    let dir = Folder::new("placed-open");
    let src = dir.file("logo.vectorcraft");
    source(&src, 100.0, 50.0, RED, 0);
    let mut s = session();
    place(&mut s, &src);
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    source(&src, 100.0, 50.0, BLUE, 1);
    // Ask When Modified (the default): reported, left as it was.
    let r = open(&mut s, &doc_path);
    assert_eq!(r["modifiedLinks"].as_array().map(Vec::len), Some(1), "{r}");
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
    // Automatically: read again as the document opens.
    s.execute("prefs.set", &json!({"key": "updateLinks", "value": "automatically"})).unwrap();
    let r = open(&mut s, &doc_path);
    assert_eq!(r["updatedLinks"].as_array().map(Vec::len), Some(1), "{r}");
    assert!(near(centre_colour(&s.doc().unwrap().doc), BLUE));
}

/// A placed document whose link keeps no size or hash and whose file's bytes are gone shows no
/// preview: resolving its links reports the file as modified and leaves the object as it is.
#[test]
fn a_placed_document_without_its_bytes_or_a_hash_is_reported_not_read() {
    let dir = Folder::new("placed-no-hash");
    let src = dir.file("logo.vectorcraft");
    source(&src, 100.0, 50.0, RED, 0);
    let mut s = session();
    let id = place(&mut s, &src);
    let key = placed(&s, id).key;
    let mut d = (*s.doc().unwrap().doc).clone();
    d.images.remove(&key);
    let Some(NodeKind::PlacedDocument(p)) = d.node_mut(id).map(|n| &mut n.kind) else { panic!("a placed document") };
    p.link.hash = None;
    p.link.size = None;
    let r = crate::cmd::links::resolve(&mut d, crate::cmd::links::LinkBase::file(None), false, false).to_json();
    assert_eq!(r["modifiedLinks"].as_array().map(Vec::len), Some(1), "{r}");
    assert!(!d.images.contains_key(&key), "not read");
}

#[test]
fn relink_points_a_placed_document_at_another_file() {
    let dir = Folder::new("placed-relink");
    let (a, b) = (dir.file("a.vectorcraft"), dir.file("b.vectorcraft"));
    source(&a, 100.0, 50.0, RED, 0);
    source(&b, 100.0, 50.0, BLUE, 0);
    let mut s = session();
    let id = place(&mut s, &a);
    let r = s.execute("links.relink", &json!({"ids": [id.0], "path": b})).unwrap();
    assert_eq!(r["relinked"], json!([id.0]), "{r}");
    assert_eq!(placed(&s, id).link.path, b);
    assert!(near(centre_colour(&s.doc().unwrap().doc), BLUE));
}

/// Missing Links from This Folder finds a missing placed document by name, except with copy, which
/// takes images only: the placed document then stays as it is.
#[test]
fn relink_missing_in_folder_finds_placed_documents_except_with_copy() {
    let dir = Folder::new("placed-missing-folder");
    let (doc_file, pic) = (dir.file("art/a.vectorcraft"), dir.file("art/photo.png"));
    write(&pic, &crate::tests_links::png(60, 30, RED));
    source(&doc_file, 100.0, 50.0, RED, 0);
    let mut s = session();
    let (placed_id, image_id) = (place(&mut s, &doc_file), place(&mut s, &pic));
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    std::fs::remove_file(&doc_file).unwrap();
    std::fs::remove_file(&pic).unwrap();
    open(&mut s, &doc_path);
    let found = dir.0.join("found");
    let chosen = found.join("renamed.png").to_string_lossy().into_owned();
    write(&chosen, &crate::tests_links::png(60, 30, BLUE));
    source(&found.join("a.vectorcraft").to_string_lossy(), 100.0, 50.0, BLUE, 0);
    let p = json!({"ids": [image_id.0], "path": chosen, "scope": "missingInFolder", "copy": true});
    let r = s.execute("links.relink", &p).unwrap();
    assert_eq!(r["relinked"], json!([image_id.0]), "{r}");
    assert_eq!(placed(&s, placed_id).link.path, doc_file, "copy leaves the placed document");
    s.execute("edit.undo", &json!({})).unwrap();
    let p = json!({"ids": [image_id.0], "path": chosen, "scope": "missingInFolder"});
    let r = s.execute("links.relink", &p).unwrap();
    assert_eq!(r["relinked"], json!([image_id.0, placed_id.0]), "{r}");
}

/// A file made from scratch at `path` in `format` (`pdf`, or `ai`: a PDF-compatible .ai): one
/// 100×50 page per color of `pages`, filled with it (`extra` more rectangles on each change the
/// file's size).
fn pdf_source(path: &str, format: &str, pages: &[[u8; 3]], extra: usize) {
    let mut d = Document::new(100.0, 50.0);
    let layer = d.layers[0].id;
    for (i, rgb) in pages.iter().enumerate() {
        let x = i as f64 * 150.0;
        let page = Rect::new(x, 0.0, x + 100.0, 50.0);
        if i > 0 {
            let mut b = d.artboards[0].clone();
            (b.id, b.rect) = (i as u32 + 1, page);
            d.artboards.push(b);
        }
        for _ in 0..=extra {
            let id = d.alloc_id();
            d.insert(Some(layer), usize::MAX, Node::path(id, shapes::rectangle(page), look(*rgb))).unwrap();
        }
    }
    let p = if format == "ai" { json!({"pdfCompatible": true}) } else { json!({"range": "all", "preserveEditing": false}) };
    write(path, &cmd::fileio::encode_all(&d, format, &p).unwrap().files[0].1);
}

#[test]
fn a_pdf_page_placed_without_link_is_embedded() {
    let dir = Folder::new("placed-pdf-embedded");
    let src = dir.file("flyer.pdf");
    pdf_source(&src, "pdf", &[RED], 0);
    let mut s = session();
    let r = s.execute("file.place", &json!({"path": src, "link": false, "at": [200, 150]})).unwrap();
    assert_eq!(r["linked"], false, "{r}");
    let n = s.doc().unwrap().doc.node(NodeId(r["ids"][0].as_u64().unwrap())).unwrap();
    assert_eq!(n.kind_label(), "Clip Group");
    assert!(!s.doc().unwrap().doc.has_placed());
    // Text asked for as outlines is embedded with Link too.
    let r = s.execute("file.place", &json!({"path": src, "textAs": "outlines"})).unwrap();
    assert_eq!(r["linked"], false, "{r}");
    assert!(!s.doc().unwrap().doc.has_placed());
}

#[test]
fn with_link_a_pdf_page_places_linked() {
    let dir = Folder::new("placed-pdf-linked");
    let src = dir.file("flyer.pdf");
    pdf_source(&src, "pdf", &[RED], 0);
    let mut s = session();
    let id = place(&mut s, &src);
    let p = placed(&s, id);
    assert_eq!((p.link.path.as_str(), p.link.page, p.bounding), (src.as_str(), Some(1), false), "{p:?}");
    assert!((p.width - 100.0).abs() < 1e-6 && (p.height - 50.0).abs() < 1e-6, "{p:?}");
    assert_eq!(bounds(&s, id), Rect::new(150.0, 125.0, 250.0, 175.0));
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
    let row = &s.execute("links.list", &json!({})).unwrap()["links"][0];
    assert_eq!((row["format"].as_str(), row["document"].as_bool(), row["status"].as_str()), (Some("PDF"), Some(true), Some("ok")), "{row}");
    assert_eq!(s.doc().unwrap().doc.images[&p.key].mime, "application/pdf");
    // Break Link gives the group placing it without Link gives.
    let r = s.execute("links.embed", &json!({"ids": [id.0]})).unwrap();
    assert_eq!(r["embedded"], json!([id.0]), "{r}");
    assert_eq!(s.doc().unwrap().doc.node(id).unwrap().kind_label(), "Clip Group");
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
}

#[test]
fn a_changed_pdf_is_modified_and_update_links_reads_it_again() {
    let dir = Folder::new("placed-pdf-update");
    let src = dir.file("flyer.pdf");
    pdf_source(&src, "pdf", &[RED], 0);
    let mut s = session();
    let id = place(&mut s, &src);
    pdf_source(&src, "pdf", &[BLUE], 1);
    assert_eq!(s.execute("links.check", &json!({})).unwrap()["links"][0]["status"], "modified");
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED), "shows the page as it read it");
    let u = s.execute("links.update", &json!({})).unwrap();
    assert_eq!(u["updated"], json!([id.0]), "{u}");
    assert!(near(centre_colour(&s.doc().unwrap().doc), BLUE));
    assert_eq!(s.execute("links.check", &json!({})).unwrap()["modified"], 0);
    // Relinked to another PDF, it shows that file's page.
    let other = dir.file("other.pdf");
    pdf_source(&other, "pdf", &[RED], 2);
    let r = s.execute("links.relink", &json!({"ids": [id.0], "path": other})).unwrap();
    assert_eq!(r["relinked"], json!([id.0]), "{r}");
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
    // A file it can't show is refused.
    let notes = dir.file("notes.txt");
    write(&notes, b"not a page");
    let e = s.execute("links.relink", &json!({"ids": [id.0], "path": notes})).unwrap_err().to_string();
    assert!(e.contains("isn't a VectorCraft document, a PDF or a PDF-compatible .ai file"), "{e}");
    assert_eq!(placed(&s, id).link.path, other);
}

#[test]
fn a_linked_pdf_page_outputs_as_vectors_after_the_document_reopens() {
    let dir = Folder::new("placed-pdf-outputs");
    let src = dir.file("flyer.pdf");
    pdf_source(&src, "pdf", &[RED], 3);
    let mut s = session();
    let id = place(&mut s, &src);
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    open(&mut s, &doc_path);
    let key = placed(&s, id).key;
    assert!(s.doc().unwrap().doc.images[&key].is_proxy(), "opened with the preview only");
    // Output reads the file again: the page's paths, not the preview.
    let pdf = s.execute("document.serialize", &json!({"format": "pdf"})).unwrap();
    assert_eq!(pdf["warnings"], json!([]), "{}", pdf["warnings"]);
    let (images, paths) = pdf_contents(&decode(&pdf));
    assert_eq!(images, 0, "no image in the PDF");
    assert!(paths > 0);
    let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap();
    assert!(!svg["text"].as_str().unwrap().contains("<image"), "vectors");
    // Include Linked Files keeps the PDF itself.
    let full = file_json(&decode(&s.execute("document.serialize", &json!({"format": "vectorcraft", "includeLinked": true})).unwrap()));
    assert_eq!((full["images"][&key]["proxy"].clone(), full["images"][&key]["mime"].as_str()), (Value::Null, Some("application/pdf")));
}

#[test]
fn saving_and_reopening_keeps_a_linked_pdf_pages_number_and_box() {
    let dir = Folder::new("placed-pdf-reopen");
    let src = dir.file("flyer.pdf");
    pdf_source(&src, "pdf", &[RED, BLUE], 4);
    let mut s = session();
    let r = s.execute("file.place", &json!({"path": src, "page": 2, "crop": "media", "at": [200, 150]})).unwrap();
    assert_eq!(r["linked"], true, "{r}");
    let id = NodeId(r["ids"][0].as_u64().unwrap());
    let before = placed(&s, id);
    assert_eq!((before.link.page, before.page_box), (Some(2), vectorcraft_doc::PageBox::Media), "{before:?}");
    assert!(near(centre_colour(&s.doc().unwrap().doc), BLUE), "page 2");
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    let saved = file_json(&std::fs::read(&doc_path).unwrap());
    let kind = &saved["document"]["layers"][0]["kind"]["children"][0]["kind"];
    assert_eq!((kind["type"].as_str(), kind["page_box"].as_str(), kind["link"]["page"].as_u64()), (Some("placeddocument"), Some("media"), Some(2)));
    let r = open(&mut s, &doc_path);
    assert_eq!((r["missingLinks"].clone(), r["modifiedLinks"].clone()), (json!([]), json!([])), "{r}");
    let mut reopened = placed(&s, id);
    assert_eq!(reopened.link.relative.take().as_deref(), Some("flyer.pdf"), "written on save");
    assert_eq!(reopened, before);
    assert!(near(centre_colour(&s.doc().unwrap().doc), BLUE));
    // Read again from the file, it shows the same page and box.
    pdf_source(&src, "pdf", &[RED, RED], 5);
    s.execute("links.update", &json!({})).unwrap();
    let after = placed(&s, id);
    assert_eq!((after.link.page, after.page_box), (Some(2), vectorcraft_doc::PageBox::Media));
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
}

/// A placed PDF page in a `.ai` file's editing data is a placed document of the page and box the
/// dictionary after it names. Its file missing, it shows the preview the file keeps, in the same
/// place. Its file found in the folder kept relative to the document, it shows that page.
#[test]
fn a_pdf_page_placed_in_an_ai_files_editing_data_opens_as_a_placed_document() {
    use vectorcraft_testkit::ai;
    let dir = Folder::new("placed-pdf-in-ai");
    let place = "%AI5_BeginPlace\n%%FileType: 1\n[0.1 0 0 -0.1 20 80] 0 0 600 600 4 4 0\n(/no/such/folder/src/flyer.pdf)`\n\
         %%IncludeFile:/no/such/folder/src/flyer.pdf\n%AI17_Begin_Content_if_version_gt:24 12\n%AI28_BeginPlacedRelativePath\n(src/)\n\
         %AI28_EndPlacedRelativePath\n%AI17_Alternate_Content\n%AI17_End_Versioned_Content\n%AI26_BeginPlacedObjectPreview\n\
         2 1 8 32 17 \x7f\x0a\x0a\x73\x7f\x0a\x0a\x73\n%AI26_EndPlacedObjectPreview\n~\n%AI5_EndPlace\n\
         %_/ArtDictionary :\n%_2 /Int (AI11 PlacedPDFPageIndex) ,\n%_4 /Int (AI11 PlacedPDFCropTo) ,\n%_;\n%_\n";
    let doc_path = dir.file("poster.ai");
    let data = ai::editing_data(200.0, 100.0, &ai::layer("Art", place));
    // The page draws what the half-covering preview draws over white.
    write(&doc_path, &ai::ai(data.as_bytes(), "0.54 0.54 0.95 rg 20 20 60 60 re f"));
    let shown = |s: &Session| {
        let img = vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, Rect::new(0.0, 0.0, 100.0, 100.0), 1.0, true);
        let [r, g, b, _] = img.pixel(50, 50);
        [r, g, b]
    };
    let one = |s: &Session| {
        let mut found = vec![];
        s.doc().unwrap().doc.visit_placed(|n, p| found.push((n.id, p.clone())));
        let [(id, p)] = &found[..] else { panic!("one placed document: {found:?}") };
        (*id, p.clone(), s.doc().unwrap().doc.images[&p.key].is_proxy())
    };
    let mut s = session();

    let r = open(&mut s, &doc_path);
    assert_eq!(r["missingLinks"][0]["name"].as_str(), Some("flyer.pdf"), "{r}");
    let (id, p, proxy) = one(&s);
    assert!(proxy, "the preview");
    assert_eq!((p.link.page, p.page_box, p.bounding), (Some(2), vectorcraft_doc::PageBox::Media, false));
    assert_eq!(p.link.relative.as_deref(), Some("src/flyer.pdf"));
    let place = bounds(&s, id);
    assert!((place.x0 - 20.0).abs() < 0.01 && (place.width() - 60.0).abs() < 0.01 && (place.height() - 60.0).abs() < 0.01, "{place:?}");
    assert!(near(shown(&s), [138, 138, 243]), "the preview shows: {:?}", shown(&s));

    pdf_source(&dir.file("src/flyer.pdf"), "pdf", &[BLUE, RED], 0);
    let r = open(&mut s, &doc_path);
    assert_eq!((r["missingLinks"].clone(), r["modifiedLinks"].clone()), (json!([]), json!([])), "{r}");
    let (id, p, proxy) = one(&s);
    assert!(!proxy, "the file's bytes, not the preview");
    assert_eq!((p.link.page, p.page_box, p.bounding), (Some(2), vectorcraft_doc::PageBox::Media, false));
    assert!(p.link.hash.is_some() && p.link.size.is_some(), "{:?}", p.link);
    assert_eq!(bounds(&s, id), place, "the same place");
    assert!(near(shown(&s), RED), "page 2 of the file shows: {:?}", shown(&s));
}

/// A `.ai` saved without PDF compatibility (its page only text) that places itself as a page is
/// read once inside itself: the copy inside keeps its link as stored, and the file opens.
#[test]
fn an_ai_file_that_places_itself_opens() {
    use vectorcraft_testkit::ai;
    let dir = Folder::new("ai-places-itself");
    let doc_path = dir.file("poster.ai");
    let at = doc_path.replace('\\', "/");
    let place = format!(
        "%AI5_BeginPlace\n%%FileType: 1\n[0.1 0 0 -0.1 20 80] 0 0 600 600 4 4 0\n({at})`\n%%IncludeFile:{at}\n%AI26_BeginPlacedObjectPreview\n\
         2 1 8 32 17 \x7f\x0a\x0a\x73\x7f\x0a\x0a\x73\n%AI26_EndPlacedObjectPreview\n~\n%AI5_EndPlace\n\
         %_/ArtDictionary :\n%_1 /Int (AI11 PlacedPDFPageIndex) ,\n%_4 /Int (AI11 PlacedPDFCropTo) ,\n%_;\n%_\n"
    );
    let data = ai::editing_data(200.0, 100.0, &ai::layer("Art", &format!("0 g\n{}f\n{place}", ai::rect(120.0, 20.0, 180.0, 80.0))));
    write(&doc_path, &ai::ai(data.as_bytes(), "BT /F1 12 Tf 10 50 Td (Saved without its PDF part) Tj ET"));
    let mut s = session();
    let r = open(&mut s, &doc_path);
    let mut found = vec![];
    s.doc().unwrap().doc.visit_placed(|_, p| found.push(p.link.path.replace('\\', "/")));
    assert_eq!(found, [at], "{r}");
}

#[test]
fn a_pdf_compatible_ai_file_places_linked_like_a_pdf() {
    let dir = Folder::new("placed-ai-linked");
    let src = dir.file("badge.ai");
    pdf_source(&src, "ai", &[RED], 0);
    let mut s = session();
    let id = place(&mut s, &src);
    assert_eq!(placed(&s, id).link.path, src);
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
    let row = &s.execute("links.list", &json!({})).unwrap()["links"][0];
    assert_eq!(row["format"], "PDF-compatible .ai", "{row}");
    pdf_source(&src, "ai", &[BLUE], 1);
    let u = s.execute("links.update", &json!({})).unwrap();
    assert_eq!(u["updated"], json!([id.0]), "{u}");
    assert!(near(centre_colour(&s.doc().unwrap().doc), BLUE));
    let (images, _) = pdf_contents(&decode(&s.execute("document.serialize", &json!({"format": "pdf"})).unwrap()));
    assert_eq!(images, 0);
}

#[test]
fn a_placed_document_inside_a_placed_document_outputs_from_its_own_file() {
    let dir = Folder::new("placed-nested");
    let (logo, card) = (dir.file("logo.vectorcraft"), dir.file("card.vectorcraft"));
    source(&logo, 100.0, 50.0, RED, 0);
    // The card places the logo, and is saved with the logo's preview only.
    let mut s = session();
    place(&mut s, &logo);
    save(&mut s, &card);
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    place(&mut s, &card);
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
    // The logo inside is read from its file: vectors, not its preview.
    let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap();
    let text = svg["text"].as_str().unwrap();
    assert!(!text.contains("<image"), "the logo's file, not its preview: {text}");
    assert_eq!(pdf_contents(&decode(&s.execute("document.serialize", &json!({"format": "pdf"})).unwrap())).0, 0);
}

#[test]
fn documents_that_place_each_other_dont_loop() {
    let dir = Folder::new("placed-loop");
    let (a, b) = (dir.file("a.vectorcraft"), dir.file("b.vectorcraft"));
    source(&a, 100.0, 50.0, RED, 0);
    // B places A; A then places B.
    let mut s = session();
    place(&mut s, &a);
    save(&mut s, &b);
    s.execute("document.open", &json!({"path": a})).unwrap();
    place(&mut s, &b);
    save(&mut s, &a);
    s.execute("document.open", &json!({"path": b})).unwrap();
    let u = s.execute("links.update", &json!({})).unwrap();
    assert_eq!(u["updated"].as_array().map(Vec::len), Some(1), "{u}");
    // Drawn and output a few levels deep, then their previews.
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
    for format in ["svg", "pdf", "png"] {
        s.execute("document.serialize", &json!({"format": format})).unwrap();
    }
}

#[test]
fn what_cant_be_placed_linked_is_an_error() {
    let dir = Folder::new("placed-errors");
    let src = dir.file("logo.vectorcraft");
    source(&src, 100.0, 50.0, RED, 0);
    let mut s = session();
    let place = |s: &mut Session, p: Value| s.execute("file.place", &p).unwrap_err().to_string();
    let e = place(&mut s, json!({"path": src, "page": 2}));
    assert!(e.contains("page 2") && e.contains("1 artboard"), "{e}");
    let e = place(&mut s, json!({"path": src, "crop": "art"}));
    assert!(e.contains("crop"), "{e}");
    let empty = dir.file("empty.vectorcraft");
    write(&empty, &vectorcraft_format::save_file(&Document::new(100.0, 50.0)));
    let e = place(&mut s, json!({"path": empty, "crop": "bounding"}));
    assert!(e.contains("no art to place"), "{e}");
    // Junk params are errors, not crashes.
    for v in [json!(null), json!(true), json!(-1), json!(1e308), json!("x"), json!([1, 2]), json!({"a": {"b": []}})] {
        for k in ["link", "page", "crop", "at", "rect"] {
            let _ = s.execute("file.place", &json!({"path": src, k: v}));
        }
    }
}

#[test]
fn a_placed_documents_resources_stay_its_own() {
    let dir = Folder::new("placed-symbols");
    let src = dir.file("badge.vectorcraft");
    star_source(&src, "star");
    let mut s = session();
    parent_star(&mut s);
    place(&mut s, &src);
    assert!(near(centre_colour(&s.doc().unwrap().doc), BLUE), "the file's own Star");
    let doc = &s.doc().unwrap().doc;
    assert_eq!(doc.symbols.len(), 1, "nothing joins the document");
    let prefix = vectorcraft_doc::placed_document::RESOURCE_PREFIX;
    assert!(doc.images.keys().all(|k| !k.starts_with(prefix)));
    let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap();
    assert!(!svg["text"].as_str().unwrap().contains("<image"));
    // Nor its saves.
    let saved = String::from_utf8(decode(&s.execute("document.serialize", &json!({"format": "vectorcraft"})).unwrap())).unwrap();
    assert!(!saved.contains(prefix), "{saved}");
    // Saved for older apps: the art and its resources, as plain objects.
    let old = vectorcraft_format::load(&decode(&s.execute("document.serialize", &json!({"format": "vectorcraft", "version": 2})).unwrap())).unwrap();
    assert!(!old.has_placed());
    assert!(near(centre_colour(&old), BLUE));
}

#[test]
fn saved_for_older_apps_with_a_precision_the_placed_art_is_rounded_too() {
    let dir = Folder::new("placed-precision");
    let src = dir.file("triangle.vectorcraft");
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    let id = d.alloc_id();
    let points = [(3.4876474632797176, 1.0), (90.0, 2.123456789), (40.0, 80.0)].map(|(x, y)| vectorcraft_geom::Point::new(x, y));
    let path = vectorcraft_geom::PathData::single(vectorcraft_geom::path::SubPath::polyline(&points, true));
    d.insert(Some(layer), 0, Node::path(id, path, look(BLUE))).unwrap();
    write(&src, &vectorcraft_format::save_file(&d));
    let mut s = session();
    place(&mut s, &src);
    // The coordinates of every anchor and handle in a file's layers.
    fn coords(v: &Value, out: &mut Vec<f64>) {
        match v {
            Value::Object(o) => {
                for a in o.get("anchors").and_then(Value::as_array).into_iter().flatten() {
                    out.extend(["p", "in", "out"].iter().filter_map(|k| a[*k].as_array()).flatten().filter_map(Value::as_f64));
                }
                o.values().for_each(|c| coords(c, out));
            }
            Value::Array(a) => a.iter().for_each(|c| coords(c, out)),
            _ => {}
        }
    }
    let mut saved = |p: Value| {
        let file: Value = serde_json::from_slice(&decode(&s.execute("document.serialize", &p).unwrap())).unwrap();
        assert!(!file.to_string().contains("placeddocument"), "{file}");
        let mut out = vec![];
        coords(&file["document"]["layers"], &mut out);
        out
    };
    let at4 = |x: f64| (x * 1e4).round() / 1e4 == x;
    let exact = saved(json!({"format": "vectorcraft", "version": 3}));
    assert!(exact.len() >= 6 && !exact.iter().all(|x| at4(*x)), "{exact:?}");
    let rounded = saved(json!({"format": "vectorcraft", "version": 3, "precision": 4}));
    assert_eq!(rounded.len(), exact.len());
    assert!(rounded.iter().all(|x| at4(*x)), "{rounded:?}");
    // The preview shows the rounded art the older app opens, not the placed file's exact art.
    let file = decode(&s.execute("file.saveCopy", &json!({"version": 3, "precision": 0, "preview": true})).unwrap());
    let preview = vectorcraft_format::preview(&file);
    assert!(preview.is_some());
    assert_eq!(preview, crate::cmd::fileio::preview_png(&vectorcraft_format::load(&file).unwrap()).unwrap());
    assert_ne!(preview, crate::cmd::fileio::preview_png(&s.doc().unwrap().doc).unwrap());
}

#[test]
fn break_link_gives_the_editable_copy_placing_without_link_gives() {
    let dir = Folder::new("placed-break");
    let src = dir.file("badge.vectorcraft");
    star_source(&src, "break");
    let mut s = session();
    parent_star(&mut s);
    // The copy placing without Link gives.
    let r = s.execute("file.place", &json!({"path": src, "link": false, "at": [200, 150]})).unwrap();
    let copy = s.doc().unwrap().doc.node(NodeId(r["ids"][0].as_u64().unwrap())).unwrap().clone();
    s.execute("edit.undo", &json!({})).unwrap();
    let id = place(&mut s, &src);
    let r = s.execute("links.embed", &json!({"ids": [id.0]})).unwrap();
    assert_eq!(r["embedded"], json!([id.0]), "{r}");
    let doc = &s.doc().unwrap().doc;
    let n = doc.node(id).unwrap();
    assert!(!doc.has_placed() && n.kind_label() == "Clip Group", "{}", n.kind_label());
    assert_eq!((n.name.as_deref(), n.geometric_bounds()), (copy.name.as_deref(), copy.geometric_bounds()));
    let kinds = |n: &Node| {
        let mut k = vec![];
        n.walk(&mut |c| k.push(c.kind_label()));
        k
    };
    assert_eq!(kinds(n), kinds(&copy));
    assert!(near(centre_colour(doc), BLUE), "the art as it showed");
    let names: Vec<&str> = doc.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names.len(), 2, "the file's Star joined, renamed: {names:?}");
    assert_eq!(s.execute("links.list", &json!({})).unwrap()["links"], json!([]));
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Embed");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.has_placed());
    // A file that can't be read stays linked.
    std::fs::remove_file(&src).unwrap();
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    open(&mut s, &doc_path);
    let r = s.execute("links.embed", &json!({"ids": [id.0]})).unwrap();
    assert_eq!((r["embedded"].clone(), r["missing"].clone()), (json!([]), json!([id.0])), "{r}");
}

#[test]
fn expand_turns_a_placed_document_into_editable_art_with_its_resources() {
    let dir = Folder::new("placed-expand");
    let src = dir.file("badge.vectorcraft");
    star_source(&src, "star");
    let mut s = session();
    parent_star(&mut s);
    let id = place(&mut s, &src);
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    let before = bounds(&s, id);
    s.execute("object.expand", &json!({})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let n = doc.node(id).unwrap();
    assert!(n.is_container() && !doc.has_placed(), "{:?}", n.kind_label());
    let after = n.geometric_bounds().unwrap();
    assert!((after.width() - before.width()).abs() < 0.5 && (after.height() - before.height()).abs() < 0.5, "{after:?} vs {before:?}");
    assert!(near(centre_colour(doc), BLUE));
    let saved = vectorcraft_format::save_file(doc);
    assert!(near(centre_colour(&vectorcraft_format::load(&saved).unwrap()), BLUE), "saved with its resources");
    let info = s.execute("object.expand.info", &json!({})).unwrap();
    assert_eq!(info["object"], false, "nothing left to expand: {info}");
}

/// Expand also expands a placed document in the art of a selected symbol instance, in the same
/// command.
#[test]
fn expand_expands_a_placed_document_in_a_symbol_instances_art() {
    let dir = Folder::new("placed-expand-symbol");
    let src = dir.file("badge.vectorcraft");
    star_source(&src, "star");
    let mut s = session();
    let id = place(&mut s, &src);
    s.select(|_, sel| sel.set([id])).unwrap();
    let inst = NodeId(s.execute("symbol.new", &json!({"name": "Badge"})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.expand", &json!({})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let mut placed = 0;
    doc.node(inst).unwrap().walk(&mut |c| placed += usize::from(matches!(c.kind, NodeKind::PlacedDocument(_))));
    assert_eq!(placed, 0, "the placed document in the art is expanded");
}

#[test]
fn saving_a_document_refreshes_the_open_documents_that_place_it() {
    let dir = Folder::new("placed-refresh");
    let src = dir.file("logo.vectorcraft");
    source(&src, 100.0, 50.0, RED, 0);
    let mut s = session();
    let id = place(&mut s, &src);
    let parent = s.active_index().unwrap();
    // Open the file, recolour it and save it.
    s.execute("document.open", &json!({"path": src})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#1414e6"})).unwrap();
    s.execute("document.save", &json!({})).unwrap();
    let child = s.active_index().unwrap();
    assert_ne!(parent, child);
    let doc = &s.documents()[parent];
    assert_eq!(doc.history.undo.last().unwrap().label, "Update Links");
    assert!(near(centre_colour(&doc.doc), BLUE), "shows the saved version");
    assert!(matches!(doc.doc.node(id).map(|n| &n.kind), Some(NodeKind::PlacedDocument(_))));
    assert_eq!(s.active_index(), Some(child), "the saved document stays active");
}

/// Placed documents inside a symbol and a pattern of another open document are read again when
/// their file is saved, as one in the layers is.
#[test]
fn saving_a_document_refreshes_its_placed_copies_inside_symbols_and_patterns() {
    let dir = Folder::new("placed-refresh-nested");
    let src = dir.file("logo.vectorcraft");
    source(&src, 100.0, 50.0, RED, 0);
    let mut s = session();
    let id = place(&mut s, &src);
    s.execute("symbol.new", &json!({"name": "Logo", "ids": [id.0]})).unwrap();
    let id = place(&mut s, &src);
    s.execute("object.pattern.make", &json!({"ids": [id.0], "edit": false})).unwrap();
    s.execute("edit.clear", &json!({"ids": [id.0]})).unwrap();
    let parent = s.active_index().unwrap();
    let mut in_layers = 0;
    s.doc().unwrap().doc.walk(|n| in_layers += usize::from(matches!(n.kind, NodeKind::PlacedDocument(_))));
    assert_eq!(in_layers, 0, "no placed document in the layers");
    s.execute("document.open", &json!({"path": src})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#1414e6"})).unwrap();
    s.execute("document.save", &json!({})).unwrap();
    let hash = Some(vectorcraft_doc::links::hash_bytes(&std::fs::read(&src).unwrap()));
    let doc = &s.documents()[parent];
    assert_eq!(doc.history.undo.last().unwrap().label, "Update Links");
    assert!(near(centre_colour(&doc.doc), BLUE), "the symbol's instance shows the saved version");
    let mut hashes = vec![];
    doc.doc.visit_placed(|_, p| hashes.push(p.link.hash.clone()));
    assert_eq!(hashes, [hash.clone(), hash], "the symbol's, then the pattern's");
}

#[test]
fn link_info_gives_the_artboards_size_not_pixels() {
    let dir = Folder::new("placed-info");
    let src = dir.file("logo.vectorcraft");
    source(&src, 100.0, 50.0, RED, 0);
    let mut s = session();
    let id = place(&mut s, &src);
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    let i = s.execute("links.info", &json!({"id": id.0})).unwrap();
    assert_eq!((i["pageWidth"].as_f64(), i["pageHeight"].as_f64(), i["document"].as_bool()), (Some(100.0), Some(50.0), Some(true)), "{i}");
    assert!(i.get("pixelWidth").is_none() && i.get("ppi").is_none(), "no pixels: {i}");
    assert_eq!(
        (i["status"].as_str(), i["fileName"].as_str(), i["scale"].clone()),
        (Some("ok"), Some("logo.vectorcraft"), json!([200.0, 200.0])),
        "{i}"
    );
}

/// A confined caller's Link Info of a placed document whose file is outside its read roots has no
/// `link`, `path` or `location`, and keeps the file's name.
#[test]
fn link_info_leaves_out_a_placed_documents_path_outside_the_read_roots() {
    let away = Folder::new("placed-lrr-away");
    let home = Folder::new("placed-lrr-home");
    let src = away.file("logo.vectorcraft");
    source(&src, 100.0, 50.0, RED, 0);
    let mut s = session();
    let id = place(&mut s, &src);
    let roots = crate::file_access::AutomationRoots::new(&[&home.0], &[]).unwrap().unwrap();
    let i = crate::file_access::confine(Some(&roots), || s.execute("links.info", &json!({"id": id.0}))).unwrap();
    assert!(!i.to_string().contains("placed-lrr-away"), "{i}");
    assert_eq!(
        (i["fileName"].as_str(), i["document"].as_bool(), i["status"].as_str()),
        (Some("logo.vectorcraft"), Some(true), Some("missing")),
        "{i}"
    );
    let l = crate::file_access::confine(Some(&roots), || s.execute("links.list", &json!({}))).unwrap();
    assert!(!l.to_string().contains("placed-lrr-away") && l["links"][0]["name"] == "logo.vectorcraft", "{l}");
    let i = s.execute("links.info", &json!({"id": id.0})).unwrap();
    assert_eq!((&i["link"], &i["path"]), (&json!(src), &json!(src)), "unconfined: {i}");
}

/// `document.inspect` and `document.json` for a confined caller: the document's file, a placed
/// document and a linked image outside the read roots are not named by path, and the document
/// keeps its links.
#[test]
fn document_replies_leave_out_paths_outside_the_read_roots() {
    let away = Folder::new("doc-lrr-away");
    let home = Folder::new("doc-lrr-home");
    let src = away.file("logo.vectorcraft");
    source(&src, 100.0, 50.0, RED, 0);
    let pic = away.file("photo.png");
    write(&pic, &crate::tests_links::png(60, 30, BLUE));
    let mut s = session();
    let id = place(&mut s, &src);
    let r = s.execute("file.place", &json!({"path": pic, "at": [50, 50]})).unwrap();
    assert_eq!(r["linked"], true, "{r}");
    s.execute("variable.define", &json!({"name": "Photo", "kind": "image"})).unwrap();
    s.execute("dataset.new", &json!({"name": "Row", "values": {"Photo": pic}})).unwrap();
    let image = r["ids"][0].as_u64().unwrap();
    let poster = away.file("poster.vectorcraft");
    save(&mut s, &poster);
    // The folder Export for Screens wrote to, and a pattern of the linked image being edited: its
    // definition before editing is kept beside the tile art.
    let out = away.file("out");
    s.execute("document.exportForScreens", &json!({"folder": out, "formats": [{"format": "svg"}]})).unwrap();
    let pattern = s.execute("object.pattern.make", &json!({"ids": [image], "edit": false})).unwrap()["name"].clone();
    s.execute("object.pattern.edit", &json!({"name": pattern})).unwrap();
    assert!(s.doc().unwrap().doc.pattern_edit.as_ref().is_some_and(|e| e.original.is_some()));
    let ask = |s: &mut Session, root: &std::path::Path, cmd: &str| {
        let roots = crate::file_access::AutomationRoots::new(&[root], &[]).unwrap().unwrap();
        crate::file_access::confine(Some(&roots), || s.execute(cmd, &json!({}))).unwrap()
    };
    let inspect = ask(&mut s, &home.0, "document.inspect");
    assert!(!inspect.to_string().contains("doc-lrr-away"), "{inspect}");
    assert!(inspect["path"].is_null() && inspect["uid"].is_u64(), "{inspect}");
    let model = ask(&mut s, &home.0, "document.json").to_string();
    assert!(!model.contains("doc-lrr-away"), "{model}");
    for name in ["logo.vectorcraft", "photo.png"] {
        assert!(model.contains(&format!("\"path\":\"{name}\"")), "{name}: {model}");
    }
    assert!(model.contains("{\"image\":\"photo.png\"}"), "{model}");
    // The document keeps its links, and a caller whose read roots hold the files sees every path.
    assert_eq!(placed(&s, id).link.path, src);
    for model in [ask(&mut s, &away.0, "document.json"), s.execute("document.json", &json!({})).unwrap()] {
        let model = model.to_string();
        for path in [&src, &pic, &out] {
            assert!(model.contains(&json!(path).to_string()), "{path}: {model}");
        }
    }
    assert_eq!(ask(&mut s, &away.0, "document.inspect")["path"], json!(poster));
    assert_eq!(s.execute("document.inspect", &json!({})).unwrap()["path"], json!(poster));
}

/// Save As reads its defaults from the catalog; using them must retain current editable links.
#[test]
fn native_save_catalog_defaults_keep_placed_documents_editable() {
    for missing in [false, true] {
        let dir = Folder::new(if missing { "native-default-missing" } else { "native-default-linked" });
        let src = dir.file("logo.vectorcraft");
        star_source(&src, if missing { "native-default-missing" } else { "native-default-linked" });
        let mut s = session();
        let id = place(&mut s, &src);
        let original = placed(&s, id);
        if missing {
            std::fs::remove_file(&src).unwrap();
        }
        let options = s.execute("file.formatOptions", &json!({"format": "vectorcraft"})).unwrap();
        let saved = dir.file("default-options.vectorcraft");
        s.execute("document.save", &json!({"path": saved, "version": options["options"]["version"]["value"]})).unwrap();
        let bytes = std::fs::read(&saved).unwrap();
        let native = vectorcraft_format::load(&bytes).unwrap();
        let mut written = original.clone();
        // Native saves record a sibling link relative to the destination as well as its
        // absolute path. This is relocation metadata, not a change to the placed object.
        written.link.relative = Some("logo.vectorcraft".into());
        assert!(matches!(&native.node(id).unwrap().kind, NodeKind::PlacedDocument(p) if **p == written), "source missing: {missing}");
        assert_eq!(file_json(&bytes)["version"], vectorcraft_format::VERSION);
        assert_eq!(placed(&s, id), original, "saving does not rewrite the live link");
        let mut reopened = Session::new();
        open(&mut reopened, &saved);
        assert_eq!(placed(&reopened, id).link.path, src);
        assert_eq!(reopened.execute("links.check", &json!({})).unwrap()["missing"], usize::from(missing));
    }
}

/// Generic and dedicated PDF exports share editable links and prepared drawn pages.
#[test]
fn pdf_editing_keeps_placed_links_separate_from_drawn_previews() {
    for missing in [true, false] {
        let dir = Folder::new(&format!("pdf-editable-links-{missing}"));
        let src = dir.file("logo.vectorcraft");
        star_source(&src, &format!("pdf-editable-links-{missing}"));
        let mut s = session();
        parent_star(&mut s);
        let id = place(&mut s, &src);
        s.execute("artboard.new", &json!({"x":600,"y":0,"width":100,"height":100})).unwrap();
        let parent = decode(&s.execute("document.serialize", &json!({"format": "vectorcraft"})).unwrap());
        if missing {
            std::fs::remove_file(&src).unwrap();
        }
        s.execute("document.open", &json!({"name": "parent.vectorcraft", "dataBase64": vectorcraft_format::base64_encode(&parent)})).unwrap();
        let original = placed(&s, id);
        assert!(s.doc().unwrap().doc.images[&original.key].is_proxy());
        let before = s.execute("document.json", &json!({})).unwrap();
        for command in ["document.serialize", "document.export", "document.save", "document.exportPdf"] {
            let out = s.execute(command, &json!({"format":"pdf","preserveEditing":true,"range":"all","thumbnails":false})).unwrap();
            let bytes = decode(&out);
            let editing = vectorcraft_pdf::editing(&bytes).unwrap();
            assert!(editing.intact);
            let native = vectorcraft_format::load(&editing.data).unwrap();
            assert!(matches!(&native.node(id).unwrap().kind, NodeKind::PlacedDocument(p) if **p == original), "{command}: discarded editable link");
            let (images, paths) = pdf_contents(&bytes);
            assert_eq!(images > 0, missing, "{command}: pages still draw previews or vectors");
            if !missing {
                assert!(paths > 0);
            }
            assert_eq!(
                out["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("1 placed document output as its preview")),
                missing,
                "{out}"
            );
            let mut restored = Session::new();
            assert_eq!(
                restored.execute("document.open", &json!({"name":"editable.pdf","dataBase64":vectorcraft_format::base64_encode(&bytes)})).unwrap()["restored"],
                true
            );
            assert_eq!(placed(&restored, id), original);
            assert_eq!(restored.execute("links.check", &json!({})).unwrap()["missing"], usize::from(missing));
            assert_eq!(s.execute("document.json", &json!({})).unwrap(), before);
            if missing {
                let replacement = dir.file("replacement.vectorcraft");
                source(&replacement, 100.0, 50.0, BLUE, 1);
                assert_eq!(restored.execute("links.relink", &json!({"ids":[id.0],"path":replacement})).unwrap()["relinked"], json!([id.0]));
            }
        }
        for alias in ["PDF", ".pdf"] {
            let enc = cmd::fileio::encode_all(&s.doc().unwrap().doc, alias, &json!({"preserveEditing":true})).unwrap();
            let editing = vectorcraft_pdf::editing(&enc.files[0].1).unwrap();
            let native = vectorcraft_format::load(&editing.data).unwrap();
            assert!(matches!(&native.node(id).unwrap().kind, NodeKind::PlacedDocument(p) if **p == original), "{alias}");
        }
        for params in [json!({"format":"pdf","preserveEditing":false}), json!({"format":"pdf","preserveEditing":true,"range":"1"})] {
            let out = s.execute("document.serialize", &params).unwrap();
            let bytes = decode(&out);
            assert!(vectorcraft_pdf::editing(&bytes).is_none(), "{params}");
            assert_eq!(pdf_contents(&bytes).0 > 0, missing);
        }
    }
}

/// EPS draws hydrated artwork but carries the original link metadata with requested source bytes.
#[test]
fn eps_editing_keeps_placed_links_and_requested_source_bytes() {
    for missing in [true, false] {
        let dir = Folder::new(&format!("eps-editable-links-{missing}"));
        let src = dir.file("logo.vectorcraft");
        star_source(&src, &format!("eps-editable-links-{missing}"));
        let source_bytes = std::fs::read(&src).unwrap();
        let mut s = session();
        parent_star(&mut s);
        let id = place(&mut s, &src);
        let parent = decode(&s.execute("document.serialize", &json!({"format":"vectorcraft"})).unwrap());
        if missing {
            std::fs::remove_file(&src).unwrap();
        }
        s.execute("document.open", &json!({"name":"parent.vectorcraft","dataBase64":vectorcraft_format::base64_encode(&parent)})).unwrap();
        let original = placed(&s, id);
        assert!(s.doc().unwrap().doc.images[&original.key].is_proxy());
        let before = s.execute("document.json", &json!({})).unwrap();
        for include_linked in [false, true] {
            let params = json!({"format":"eps","includeLinkedFiles":include_linked,"previewFormat":"none","thumbnails":false});
            for command in ["document.serialize", "document.export", "document.exportEps"] {
                let out = s.execute(command, &params).unwrap();
                let bytes = decode(&out);
                let raw = vectorcraft_eps::native(&bytes).unwrap();
                let native = vectorcraft_format::load(&raw).unwrap();
                assert!(
                    matches!(&native.node(id).unwrap().kind, NodeKind::PlacedDocument(p) if **p == original),
                    "{command}/{missing}/{include_linked}: editable link lost"
                );
                let blob = &native.images[&original.key];
                assert_eq!(blob.is_proxy(), missing || !include_linked);
                if !missing && include_linked {
                    assert_eq!(blob.bytes.as_slice(), source_bytes.as_slice());
                }
                assert_eq!(
                    out["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("1 placed document output as its preview")),
                    missing,
                    "{out}"
                );
                // Read the printed page, independently of the native editing attachment.
                let drawn = vectorcraft_eps::import_with(&bytes, false).unwrap();
                assert!(!drawn.preview);
                let (mut images, mut paths) = (0, 0);
                drawn.document.walk(|n| match n.kind {
                    NodeKind::Image(_) => images += 1,
                    NodeKind::Path { .. } | NodeKind::Compound { .. } => paths += 1,
                    _ => {}
                });
                assert_eq!(images > 0, missing, "{command}: drawn output uses vectors or the preview");
                if !missing {
                    assert!(paths > 0);
                }
                let mut restored = Session::new();
                assert_eq!(
                    restored
                        .execute("document.open", &json!({"name":"editable.eps","dataBase64":vectorcraft_format::base64_encode(&bytes)}))
                        .unwrap()["restored"],
                    true
                );
                assert_eq!(placed(&restored, id), original);
                assert_eq!(restored.execute("links.check", &json!({})).unwrap()["missing"], usize::from(missing));
                assert_eq!(s.execute("document.json", &json!({})).unwrap(), before);
                if missing {
                    let replacement = dir.file("replacement.vectorcraft");
                    source(&replacement, 100.0, 50.0, BLUE, 1);
                    assert_eq!(restored.execute("links.relink", &json!({"ids":[id.0],"path":replacement})).unwrap()["relinked"], json!([id.0]));
                }
            }
        }
        for alias in ["EPS", ".eps"] {
            let enc = cmd::fileio::encode_all(&s.doc().unwrap().doc, alias, &json!({"previewFormat":"none","thumbnails":false})).unwrap();
            let native = vectorcraft_format::load(&vectorcraft_eps::native(&enc.files[0].1).unwrap()).unwrap();
            assert!(matches!(&native.node(id).unwrap().kind, NodeKind::PlacedDocument(p) if **p == original), "{alias}");
        }
    }
}

/// An SVG's editing attachment keeps links even when its visible output must use previews.
#[test]
fn svg_editing_keeps_placed_links_separate_from_drawn_previews() {
    for missing in [true, false] {
        let dir = Folder::new(&format!("svg-editable-links-{missing}"));
        let src = dir.file("logo.vectorcraft");
        star_source(&src, &format!("svg-editable-links-{missing}"));
        let mut s = session();
        parent_star(&mut s);
        let id = place(&mut s, &src);
        let parent = decode(&s.execute("document.serialize", &json!({"format": "vectorcraft"})).unwrap());
        if missing {
            std::fs::remove_file(&src).unwrap();
        }
        s.execute("document.open", &json!({"name": "parent.vectorcraft", "dataBase64": vectorcraft_format::base64_encode(&parent)})).unwrap();
        let original = placed(&s, id);
        assert!(s.doc().unwrap().doc.images[&original.key].is_proxy());
        let before = s.execute("document.json", &json!({})).unwrap();
        for format in ["svg", "svgz"] {
            for command in ["document.serialize", "document.export", "document.save"] {
                let out = s.execute(command, &json!({"format": format, "svg": {"preserveEditing": true, "range": "1"}})).unwrap();
                let bytes = out
                    .get("dataBase64")
                    .map(|b| vectorcraft_format::base64_decode(b.as_str().unwrap()).unwrap())
                    .unwrap_or_else(|| out["text"].as_str().unwrap().as_bytes().to_vec());
                let text = vectorcraft_svg::text_of(&bytes).unwrap();
                assert_eq!(text.contains("<image"), missing, "{format}/{command}: {text}");
                assert_eq!(
                    out["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("1 placed document output as its preview")),
                    missing,
                    "{out}"
                );
                let mut restored = Session::new();
                let opened = restored
                    .execute("document.open", &json!({"name": format!("editable.{format}"), "dataBase64": vectorcraft_format::base64_encode(&bytes)}))
                    .unwrap();
                assert_eq!(opened["restored"], true, "{opened}");
                eprintln!("SVG_EDITING_RESTORED_READY missing={missing} format={format} command={command}");
                assert_eq!(placed(&restored, id), original, "{format}/{command}: link or transform lost");
                assert_eq!(restored.execute("links.check", &json!({})).unwrap()["missing"], usize::from(missing));
                assert_eq!(s.execute("document.json", &json!({})).unwrap(), before, "source unchanged");
                if missing {
                    let replacement = dir.file("replacement.vectorcraft");
                    source(&replacement, 100.0, 50.0, BLUE, 1);
                    assert_eq!(restored.execute("links.relink", &json!({"ids": [id.0], "path": replacement})).unwrap()["relinked"], json!([id.0]));
                    assert_eq!(placed(&restored, id).link.path, replacement);
                }
            }
            let out = s.execute("document.serialize", &json!({"format": format, "preserveEditing": false})).unwrap();
            let bytes = out
                .get("dataBase64")
                .map(|b| vectorcraft_format::base64_decode(b.as_str().unwrap()).unwrap())
                .unwrap_or_else(|| out["text"].as_str().unwrap().as_bytes().to_vec());
            assert_eq!(vectorcraft_svg::text_of(&bytes).unwrap().contains("<image"), missing);
            let mut reopened = Session::new();
            assert_eq!(
                reopened
                    .execute("document.open", &json!({"name": format!("plain.{format}"), "dataBase64": vectorcraft_format::base64_encode(&bytes)}))
                    .unwrap()["restored"],
                false
            );
        }
        // Dot and case aliases must reach the same original/prepared document boundary.
        for alias in ["SVG", ".svg", "SVGZ"] {
            let enc = cmd::fileio::encode_all(&s.doc().unwrap().doc, alias, &json!({"preserveEditing": true})).unwrap();
            let format = cmd::fileio::format(alias).unwrap().id;
            let mut restored = Session::new();
            restored
                .execute(
                    "document.open",
                    &json!({"name": format!("alias.{format}"), "dataBase64": vectorcraft_format::base64_encode(&enc.files[0].1)}),
                )
                .unwrap();
            assert_eq!(placed(&restored, id), original, "{alias}");
        }
    }
}
