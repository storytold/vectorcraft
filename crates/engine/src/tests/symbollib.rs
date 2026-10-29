//! Symbol libraries: the built-in library, saving a document's symbols as a library (User
//! Defined, or a file), reopening it and adding its symbols to another document (nested symbols,
//! patterns and Symbol Options along, one undo step, deduped, renamed on a clash, placed), other
//! documents' symbols, and files that can't be read.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind, SymbolRegistration};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn doc(s: &Session) -> &vectorcraft_doc::Document {
    &s.doc().unwrap().doc
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn symbols(s: &Session) -> Vec<String> {
    doc(s).symbols.iter().map(|x| x.name.clone()).collect()
}

fn strs(v: &Value) -> Vec<&str> {
    v.as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect()
}

/// A folder of its own under the temp folder, emptied.
fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-symbollib-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A document with "Star" (a red star), "Badge" (a circle with a Star instance in it, registration
/// top left) and "Dots" (a square painted with pattern "Dots").
fn kit() -> Session {
    let mut s = session();
    run(&mut s, "shape.star", json!({"cx": 50, "cy": 50, "radius1": 20, "radius2": 8}));
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "symbol.new", json!({"name": "Star"}));
    let star = s.doc().unwrap().selection.objects.clone();
    let ring = run(&mut s, "shape.ellipse", json!({"x": 20, "y": 20, "width": 60, "height": 60}))["id"].as_u64().unwrap();
    let mut ids: Vec<u64> = star.iter().map(|i| i.0).collect();
    ids.push(ring);
    run(&mut s, "symbol.new", json!({"name": "Badge", "ids": ids, "registration": 0}));
    s.edit("Pattern", |d, _| {
        let dot = vectorcraft_doc::Node::path(
            d.alloc_id(),
            vectorcraft_geom::shapes::ellipse(vectorcraft_geom::Rect::new(0.0, 0.0, 4.0, 4.0)),
            vectorcraft_doc::Appearance::default_art(),
        );
        let def = vectorcraft_doc::PatternDef {
            tile: vectorcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0),
            ..vectorcraft_doc::PatternDef::new("Dots", vec![std::sync::Arc::new(dot)])
        };
        d.patterns.push(def);
        d.swatches.push(vectorcraft_color::Swatch {
            name: "Dots".into(),
            paint: vectorcraft_color::Paint::Pattern { pattern: "Dots".into(), xf: Default::default() },
            global: false,
            spot: false,
        });
        Ok(())
    })
    .unwrap();
    run(&mut s, "shape.rectangle", json!({"x": 120, "y": 120, "width": 40, "height": 30}));
    run(&mut s, "paint.setFill", json!({"swatch": "Dots"}));
    run(&mut s, "symbol.new", json!({"name": "Dots"}));
    s
}

#[test]
fn the_builtin_library_is_original_art_generated_in_code() {
    let mut s = Session::new();
    let list = run(&mut s, "symbol.libraries", json!({}));
    let libs = list["libraries"].as_array().unwrap();
    assert_eq!(libs.len(), 1, "{libs:?}");
    assert_eq!((libs[0]["id"].as_str(), libs[0]["category"].as_str()), (Some("basic-shapes"), Some("builtIn")));
    let lib = run(&mut s, "symbol.library", json!({"library": "basic shapes"}));
    let names: Vec<&str> = lib["symbols"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Circle", "Square", "Rounded Square", "Triangle", "Diamond", "Hexagon", "Star", "Burst", "Arrow"]);
    assert_eq!(libs[0]["count"].as_u64(), Some(names.len() as u64));
    let arrow = lib["symbols"].as_array().unwrap().iter().find(|x| x["name"] == "Arrow").unwrap();
    assert_eq!(arrow["size"], json!([48.0, 32.0]));
    assert!(s.execute("symbol.library", &json!({"library": "nope"})).is_err());
}

#[test]
fn a_saved_library_reopens_and_its_symbols_join_another_document_in_one_step() {
    let dir = folder("user");
    let mut s = kit();
    s.symbol_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    // Saving the Badge brings the Star it nests, not Dots.
    let r = run(&mut s, "symbol.saveLibrary", json!({"user": true, "name": "Signs", "names": ["Badge"]}));
    assert_eq!((r["library"].as_str(), r["count"].as_u64()), (Some("user/Signs.vectorcraft"), Some(2)));
    let path = r["path"].as_str().unwrap().to_string();
    let list = run(&mut s, "symbol.libraries", json!({}));
    let user: Vec<&Value> = list["libraries"].as_array().unwrap().iter().filter(|l| l["category"] == "user").collect();
    assert_eq!(user.len(), 1);
    assert_eq!((user[0]["name"].as_str(), user[0]["count"].as_u64()), (Some("Signs"), Some(2)));
    // The file is a document: its symbols, no art.
    let bytes = std::fs::read(&path).unwrap();
    let saved = vectorcraft_format::load(&bytes).unwrap();
    assert_eq!(saved.symbols.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["Star", "Badge"]);
    assert!(saved.layers.iter().all(|l| l.children().is_none_or(|c| c.is_empty())));
    // Loading the file gives that User Defined library.
    let r = run(&mut s, "symbol.loadLibrary", json!({ "path": path }));
    assert_eq!((r["library"].as_str(), r["name"].as_str()), (Some("user/Signs.vectorcraft"), Some("Signs")));

    // Into a new document: Badge and the Star in it, with Badge's registration, as one step.
    run(&mut s, "file.new", json!({"width": 100, "height": 100}));
    let before = undo_len(&s);
    let r = run(&mut s, "symbol.addFromLibrary", json!({"library": "Signs", "name": "Badge"}));
    assert_eq!((strs(&r["names"]), strs(&r["added"]), strs(&r["existing"])), (vec!["Badge"], vec!["Badge"], vec![]));
    assert_eq!(undo_len(&s), before + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Add to Symbols");
    assert_eq!(symbols(&s), ["Star", "Badge"]);
    assert_eq!(doc(&s).symbol_registration("Badge"), SymbolRegistration(0));
    let (_, lib) = cmd::symbollib::library(&s, "Signs").unwrap();
    assert_eq!(doc(&s).symbols, lib.doc.symbols);
    assert_eq!(doc(&s).unknown.get("symbolSizes").and_then(|m| m.get("Badge")), lib.doc.unknown.get("symbolSizes").and_then(|m| m.get("Badge")));
    assert_eq!(run(&mut s, "symbol.list", json!({}))["current"], "Badge");
    // Undo takes it all back.
    run(&mut s, "edit.undo", json!({}));
    assert!(symbols(&s).is_empty());
    run(&mut s, "edit.redo", json!({}));
    // Again: already there, so no step.
    let before = undo_len(&s);
    let r = run(&mut s, "symbol.addFromLibrary", json!({"library": "user/Signs.vectorcraft", "names": ["Badge", "Star", "Badge"]}));
    assert_eq!((strs(&r["names"]), strs(&r["existing"])), (vec!["Badge", "Star"], vec!["Badge", "Star"]));
    assert_eq!((undo_len(&s), symbols(&s).len()), (before, 2));
    assert_eq!(run(&mut s, "symbol.list", json!({}))["current"], "Badge");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_library_file_brings_patterns_renames_clashes_and_places_an_instance() {
    let dir = folder("file");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Kit.vectorcraft");
    let mut s = kit();
    let r = run(&mut s, "symbol.saveLibrary", json!({"path": file.to_string_lossy()}));
    assert_eq!(r["count"].as_u64(), Some(3));
    let lib = run(&mut s, "symbol.loadLibrary", json!({"path": file.to_string_lossy()}));
    assert_eq!((lib["name"].as_str(), lib["count"].as_u64()), (Some("Kit"), Some(3)));
    let id = lib["library"].as_str().unwrap().to_string();
    assert!(id.starts_with("loaded/"));

    // A document with another "Star" of its own.
    run(&mut s, "file.new", json!({"width": 100, "height": 100}));
    run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}));
    run(&mut s, "symbol.new", json!({"name": "Star"}));
    let mine = doc(&s).symbols[0].clone();
    let before = undo_len(&s);
    let r = run(&mut s, "symbol.addFromLibrary", json!({"library": id, "names": ["Star", "Dots"], "place": true, "x": 30, "y": 40}));
    assert_eq!(undo_len(&s), before + 1, "adding and placing are one step");
    assert_eq!(strs(&r["names"]), ["Star 2", "Dots"]);
    assert_eq!(doc(&s).symbols[0], mine, "the document's own Star stays");
    // The pattern came along, with its swatch; the placed instance is the first one, selected.
    assert!(doc(&s).pattern("Dots").is_some() && doc(&s).swatch("Dots").is_some());
    let placed = NodeId(r["placed"].as_u64().unwrap());
    assert_eq!(s.doc().unwrap().selection.objects, [placed]);
    let NodeKind::SymbolInstance { symbol, .. } = &doc(&s).node(placed).unwrap().kind else { panic!("not an instance") };
    assert_eq!(symbol, "Star 2");
    let b = doc(&s).node(placed).unwrap().geometric_bounds().unwrap();
    assert!((b.center().x - 30.0).abs() < 0.01 && (b.center().y - 40.0).abs() < 0.01, "{b:?}");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(symbols(&s), ["Star"]);
    assert!(doc(&s).node(placed).is_none() && doc(&s).pattern("Dots").is_none());
    // Placing a symbol the document has still places (one step), without adding it again.
    run(&mut s, "edit.redo", json!({}));
    let before = undo_len(&s);
    let r = run(&mut s, "symbol.addFromLibrary", json!({"library": id, "name": "Dots", "place": true}));
    assert_eq!((undo_len(&s), strs(&r["existing"])), (before + 1, vec!["Dots"]));
    assert!(r["placed"].is_u64());
    assert!(s.execute("symbol.addFromLibrary", &json!({"library": id, "name": "Nope"})).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn other_documents_symbols_load_and_unreadable_files_are_errors() {
    let mut s = kit();
    // Another document's symbols load as a library, without its art.
    let b64 = run(&mut s, "document.serialize", json!({}))["dataBase64"].as_str().unwrap().to_string();
    let r = run(&mut s, "symbol.loadLibrary", json!({"dataBase64": b64, "name": "Poster.vectorcraft"}));
    assert_eq!((r["name"].as_str(), r["count"].as_u64()), (Some("Poster"), Some(3)));
    let (_, lib) = cmd::symbollib::library(&s, r["library"].as_str().unwrap()).unwrap();
    assert!(lib.doc.layers.iter().all(|l| l.children().is_none_or(|c| c.is_empty())));
    // So do an SVG's.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="100" height="100"><defs><symbol id="dot" viewBox="0 0 10 10"><circle cx="5" cy="5" r="5"/></symbol></defs><use xlink:href="#dot" width="10" height="10"/></svg>"##;
    let r = run(&mut s, "symbol.loadLibrary", json!({"data": svg, "name": "Dots.svg"}));
    assert_eq!(r["count"].as_u64(), Some(1), "{r}");
    // Files that aren't documents, missing files and damaged native files are errors.
    assert!(s.execute("symbol.loadLibrary", &json!({"data": "hello", "name": "x.txt"})).is_err());
    assert!(s.execute("symbol.loadLibrary", &json!({"path": "/no/such/Library.vectorcraft"})).is_err());
    let dir = folder("bad");
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("Broken.vectorcraft");
    std::fs::write(&bad, b"{\"format\": \"vectorcraft\", \"doc\": 7").unwrap();
    assert!(s.execute("symbol.loadLibrary", &json!({"path": bad.to_string_lossy()})).is_err());
    // A damaged file in the user folder isn't listed.
    s.symbol_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let list = run(&mut s, "symbol.libraries", json!({}));
    assert!(list["libraries"].as_array().unwrap().iter().all(|l| l["category"] != "user"));
    s.symbol_libraries.set_user_dir(None);
    let _ = std::fs::remove_dir_all(dir);
    // Nothing to save, or no such symbol, is an error; a headless session has no user folder.
    assert!(s.execute("symbol.saveLibrary", &json!({"names": ["Nope"]})).is_err());
    assert!(s.execute("symbol.saveLibrary", &json!({"user": true})).is_err());
    run(&mut s, "file.new", json!({"width": 100, "height": 100}));
    assert!(s.execute("symbol.saveLibrary", &json!({})).is_err());
    // Without a path the library comes back as the file's bytes.
    let mut s = kit();
    let r = run(&mut s, "symbol.saveLibrary", json!({"names": ["Star"]}));
    let bytes = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
    assert_eq!(vectorcraft_format::load(&bytes).unwrap().symbols.len(), 1);
}
