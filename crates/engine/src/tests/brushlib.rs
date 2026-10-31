//! Brush libraries: the built-in libraries, saving a document's brushes as a library (User
//! Defined, or a file), reopening it and adding its brushes to another document (the patterns and
//! symbols their art uses along, one undo step, deduped, renamed on a clash, applied), other
//! documents' brushes, and files that can't be read.

use serde_json::{Value, json};
use vectorcraft_brush::BrushKind;
use vectorcraft_doc::{NodeId, NodeKind};

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

fn brushes(s: &Session) -> Vec<String> {
    vectorcraft_brush::library(doc(s)).into_iter().map(|b| b.name).collect()
}

fn brush(s: &Session, name: &str) -> vectorcraft_brush::Brush {
    vectorcraft_brush::find(doc(s), name).unwrap_or_else(|| panic!("no brush {name}"))
}

fn strs(v: &Value) -> Vec<&str> {
    v.as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect()
}

/// The patterns a brush's art fills with.
fn art_patterns(b: &vectorcraft_brush::Brush) -> Vec<String> {
    let mut out = vec![];
    for a in b.art() {
        a.walk(&mut |n| {
            for it in &n.appearance.items {
                if let vectorcraft_color::Paint::Pattern { pattern, .. } = it.paint() {
                    out.push(pattern.clone());
                }
            }
        });
    }
    out
}

/// A folder of its own under the temp folder, emptied.
fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-brushlib-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Pattern `name` (a 4 pt dot of `color` in a 10 pt tile) with its swatch, as one step.
fn add_pattern(s: &mut Session, name: &str, color: &str) {
    let color = vectorcraft_color::Color::from_hex(color).unwrap();
    s.edit("Pattern", |d, _| {
        let dot = vectorcraft_doc::Node::path(
            d.alloc_id(),
            vectorcraft_geom::shapes::ellipse(vectorcraft_geom::Rect::new(0.0, 0.0, 4.0, 4.0)),
            vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::solid(color), vectorcraft_color::Paint::None, 0.0),
        );
        let def = vectorcraft_doc::PatternDef {
            tile: vectorcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0),
            ..vectorcraft_doc::PatternDef::new(name, vec![std::sync::Arc::new(dot)])
        };
        d.swatches.push(vectorcraft_color::Swatch {
            name: name.into(),
            paint: vectorcraft_color::Paint::Pattern { pattern: name.into(), xf: Default::default() },
            global: false,
            spot: false,
        });
        d.patterns.push(def);
        Ok(())
    })
    .unwrap();
}

/// A document with brushes of its own: "My Nib" (calligraphic) and "Dotted Band" (an art brush
/// whose art is filled with pattern "Dots").
fn kit() -> Session {
    let mut s = session();
    run(&mut s, "brush.new", json!({"type": "calligraphic", "name": "My Nib", "params": {"angle": 20, "roundness": 40, "size": 7}}));
    add_pattern(&mut s, "Dots", "#ff0000");
    let band = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 8}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"swatch": "Dots"}));
    run(&mut s, "brush.new", json!({"type": "art", "name": "Dotted Band", "ids": [band]}));
    s
}

#[test]
fn the_builtin_libraries_are_original_brushes_generated_in_code() {
    let mut s = Session::new();
    let list = run(&mut s, "brush.libraries", json!({}));
    let libs = list["libraries"].as_array().unwrap();
    let ids: Vec<&str> = libs.iter().map(|l| l["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["art-ribbons", "calligraphic-nibs", "pattern-borders", "scatter-shapes"]);
    assert!(libs.iter().all(|l| l["category"] == "builtIn"));
    let lib = run(&mut s, "brush.library", json!({"library": "art ribbons"}));
    assert_eq!(
        lib["brushes"].as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap()).collect::<Vec<_>>(),
        ["Notched Ribbon", "Double Line", "Wave", "Teardrop", "Striped Ribbon"]
    );
    assert_eq!(libs[0]["count"].as_u64(), Some(5));
    // One kind of brush per library, every brush drawing something.
    for (id, kind) in [("art-ribbons", "art"), ("calligraphic-nibs", "calligraphic"), ("pattern-borders", "pattern"), ("scatter-shapes", "scatter")] {
        let (_, lib) = cmd::brushlib::library(&s, id).unwrap();
        assert!(lib.len() >= 4, "{id}");
        for b in &lib.brushes {
            assert_eq!(b.kind.type_id(), kind, "{}", b.name);
            let mut line = vectorcraft_geom::BezPath::new();
            line.move_to((0.0, 0.0));
            line.curve_to((60.0, -40.0), (140.0, 40.0), (200.0, 0.0));
            let stroke = vectorcraft_doc::StrokeLayer::new(vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK), 1.0);
            assert!(!vectorcraft_brush::stroke_pieces(&lib.doc, b, &line, &stroke).is_empty(), "{} draws nothing", b.name);
        }
    }
    // The resources their art uses come with them: Striped Ribbon's pattern, Sparkles' symbol.
    let (_, ribbons) = cmd::brushlib::library(&s, "art-ribbons").unwrap();
    assert_eq!(art_patterns(ribbons.brush("Striped Ribbon").unwrap()), ["Pinstripes"]);
    assert!(ribbons.doc.pattern("Pinstripes").is_some());
    let (_, shapes) = cmd::brushlib::library(&s, "scatter-shapes").unwrap();
    let BrushKind::Scatter(sparkles) = &shapes.brush("Sparkles").unwrap().kind else { panic!("not a scatter brush") };
    assert!(matches!(&sparkles.art.kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "Sparkle"));
    assert!(shapes.doc.symbols.iter().any(|x| x.name == "Sparkle"));
    // None of them is a default brush.
    let defaults: Vec<&str> = vectorcraft_brush::defaults().iter().map(|b| b.name.as_str()).collect();
    for id in ids {
        let (_, lib) = cmd::brushlib::library(&s, id).unwrap();
        assert!(lib.brushes.iter().all(|b| !defaults.contains(&b.name.as_str())), "{id}");
    }
    assert!(s.execute("brush.library", &json!({"library": "nope"})).is_err());
}

#[test]
fn a_saved_library_reopens_and_its_brushes_join_another_document_in_one_step() {
    let dir = folder("user");
    let mut s = kit();
    s.brush_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let r = run(&mut s, "brush.saveLibrary", json!({"user": true, "name": "Inks", "names": ["My Nib", "Dotted Band"]}));
    assert_eq!((r["library"].as_str(), r["count"].as_u64()), (Some("user/Inks.vectorcraft"), Some(2)));
    let path = r["path"].as_str().unwrap().to_string();
    let list = run(&mut s, "brush.libraries", json!({}));
    let user: Vec<&Value> = list["libraries"].as_array().unwrap().iter().filter(|l| l["category"] == "user").collect();
    assert_eq!(user.len(), 1);
    assert_eq!((user[0]["name"].as_str(), user[0]["count"].as_u64()), (Some("Inks"), Some(2)));
    // The file is a document: the two brushes and the pattern their art uses, no art.
    let saved = vectorcraft_format::load(&std::fs::read(&path).unwrap()).unwrap();
    let names: Vec<String> = vectorcraft_brush::library(&saved).into_iter().map(|b| b.name).collect();
    assert_eq!(names, ["My Nib", "Dotted Band"]);
    assert_eq!(saved.patterns.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Dots"]);
    assert!(saved.layers.iter().all(|l| l.children().is_none_or(|c| c.is_empty())));
    // Loading the file gives that User Defined library.
    let r = run(&mut s, "brush.loadLibrary", json!({ "path": path }));
    assert_eq!((r["library"].as_str(), r["name"].as_str()), (Some("user/Inks.vectorcraft"), Some("Inks")));

    // Into a new document: Dotted Band and its pattern, as one step.
    run(&mut s, "file.new", json!({"width": 100, "height": 100}));
    let defaults = brushes(&s);
    let before = undo_len(&s);
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "Inks", "name": "Dotted Band"}));
    assert_eq!((strs(&r["names"]), strs(&r["added"]), strs(&r["existing"])), (vec!["Dotted Band"], vec!["Dotted Band"], vec![]));
    assert_eq!(undo_len(&s), before + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Add to Brushes");
    assert_eq!(brushes(&s), [defaults.clone(), vec!["Dotted Band".to_string()]].concat());
    let (_, lib) = cmd::brushlib::library(&s, "Inks").unwrap();
    assert_eq!(&brush(&s, "Dotted Band"), lib.brush("Dotted Band").unwrap());
    assert!(doc(&s).pattern("Dots").is_some() && doc(&s).swatch("Dots").is_some(), "the pattern its art fills with");
    assert_eq!(run(&mut s, "brush.list", json!({}))["current"], "Dotted Band");
    // Undo takes it all back.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(brushes(&s), defaults);
    assert!(doc(&s).pattern("Dots").is_none());
    run(&mut s, "edit.redo", json!({}));
    // Again: already there, so no step and no copy.
    let before = undo_len(&s);
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "user/Inks.vectorcraft", "names": ["Dotted Band", "My Nib", "Dotted Band"]}));
    assert_eq!((strs(&r["names"]), strs(&r["existing"]), strs(&r["added"])), (vec!["Dotted Band", "My Nib"], vec!["Dotted Band"], vec!["My Nib"]));
    assert_eq!(undo_len(&s), before + 1, "My Nib was new");
    let before = undo_len(&s);
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "Inks"}));
    assert_eq!((strs(&r["existing"]).len(), undo_len(&s)), (2, before));
    assert_eq!(brushes(&s).len(), defaults.len() + 2);
    assert_eq!(doc(&s).patterns.len(), 1);
    assert_eq!(run(&mut s, "brush.list", json!({}))["current"], "My Nib", "the library's first brush");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn library_brushes_bring_their_patterns_and_symbols_rename_clashes_and_apply() {
    let mut s = session();
    // The document has a pattern "Pinstripes" and a brush "Wave" of its own.
    add_pattern(&mut s, "Pinstripes", "#00ff00");
    let mine = doc(&s).pattern("Pinstripes").unwrap().clone();
    run(&mut s, "brush.new", json!({"type": "calligraphic", "name": "Wave"}));
    let my_wave = brush(&s, "Wave");
    let path = run(&mut s, "shape.line", json!({"x1": 10, "y1": 10, "x2": 150, "y2": 60}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [path]}));
    let before = undo_len(&s);
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "art-ribbons", "names": ["Striped Ribbon", "Wave"], "apply": true}));
    assert_eq!(undo_len(&s), before + 1, "adding and applying are one step");
    assert_eq!(strs(&r["names"]), ["Striped Ribbon", "Wave 2"]);
    assert_eq!(r["applied"], json!([path]));
    assert_eq!(brush(&s, "Wave"), my_wave, "the document's own Wave stays");
    // The library's Pinstripes comes in under a free name, and the brush's art follows it.
    assert_eq!(doc(&s).pattern("Pinstripes").unwrap(), &mine);
    assert!(doc(&s).pattern("Pinstripes 2").is_some() && doc(&s).swatch("Pinstripes 2").is_some());
    assert_eq!(art_patterns(&brush(&s, "Striped Ribbon")), ["Pinstripes 2"]);
    // The first one is applied to the selected path and is the current brush.
    let n = doc(&s).node(NodeId(path)).unwrap();
    assert_eq!(n.appearance.stroke().and_then(|st| st.brush.clone()).as_deref(), Some("Striped Ribbon"));
    assert_eq!(run(&mut s, "brush.list", json!({}))["current"], "Striped Ribbon");
    // Adding it again finds it, its pattern renamed, and adds nothing.
    let before = undo_len(&s);
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "art-ribbons", "names": ["Striped Ribbon", "Wave"]}));
    assert_eq!((strs(&r["names"]), strs(&r["existing"])), (vec!["Striped Ribbon", "Wave 2"], vec!["Striped Ribbon", "Wave 2"]));
    assert_eq!(undo_len(&s), before);
    assert!(doc(&s).pattern("Pinstripes 3").is_none() && vectorcraft_brush::find(doc(&s), "Striped Ribbon 2").is_none());
    // Undo of the first add takes the brushes, the pattern and the brush stroke back.
    run(&mut s, "edit.undo", json!({}));
    assert!(vectorcraft_brush::find(doc(&s), "Striped Ribbon").is_none() && doc(&s).pattern("Pinstripes 2").is_none());
    assert!(doc(&s).node(NodeId(path)).unwrap().appearance.stroke().is_none_or(|st| st.brush.is_none()));
    // A scatter brush brings the symbol it scatters.
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "Scatter Shapes", "name": "Sparkles"}));
    assert_eq!(strs(&r["added"]), ["Sparkles"]);
    assert_eq!(doc(&s).symbols.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["Sparkle"]);
    // Applying a brush the document has still applies (one step) to the paths given, without
    // adding it again.
    run(&mut s, "select.none", json!({}));
    let before = undo_len(&s);
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "scatter-shapes", "name": "Sparkles", "apply": true, "ids": [path]}));
    assert_eq!((undo_len(&s), strs(&r["existing"]), r["applied"].clone()), (before + 1, vec!["Sparkles"], json!([path])));
    assert_eq!(doc(&s).node(NodeId(path)).unwrap().appearance.stroke().and_then(|st| st.brush.clone()).as_deref(), Some("Sparkles"));
    assert_eq!(doc(&s).symbols.len(), 1);
    assert!(s.execute("brush.addFromLibrary", &json!({"library": "art-ribbons", "name": "Nope"})).is_err());
    assert!(s.execute("brush.addFromLibrary", &json!({"name": "Wave"})).is_err());
}

/// The undo step is Add to Brushes when a brush comes in (applied or not), and Apply Brush when
/// the brushes are all there and the click only applies one; undo and redo of either restore the
/// document as it was and as the step left it.
#[test]
fn adding_library_brushes_is_add_to_brushes_and_only_applying_one_is_apply_brush() {
    let mut s = session();
    let path = run(&mut s, "shape.line", json!({"x1": 10, "y1": 10, "x2": 150, "y2": 60}))["id"].as_u64().unwrap();
    let label = |s: &Session| s.doc().unwrap().history.undo.last().unwrap().label.clone();
    let stroke_brush = |s: &Session| doc(s).node(NodeId(path)).unwrap().appearance.stroke().and_then(|st| st.brush.clone());
    run(&mut s, "brush.addFromLibrary", json!({"library": "art-ribbons", "name": "Wave", "apply": true, "ids": [path]}));
    assert_eq!((label(&s), stroke_brush(&s).as_deref()), ("Add to Brushes".to_string(), Some("Wave")));
    run(&mut s, "brush.addFromLibrary", json!({"library": "art-ribbons", "name": "Teardrop"}));
    assert_eq!(label(&s), "Add to Brushes");
    let (before, brushes_before) = (undo_len(&s), brushes(&s));
    let r = run(&mut s, "brush.addFromLibrary", json!({"library": "art-ribbons", "name": "Teardrop", "apply": true, "ids": [path]}));
    assert_eq!((strs(&r["added"]), strs(&r["existing"])), (vec![], vec!["Teardrop"]));
    assert_eq!((undo_len(&s), label(&s), brushes(&s)), (before + 1, "Apply Brush".to_string(), brushes_before.clone()));
    assert_eq!(stroke_brush(&s).as_deref(), Some("Teardrop"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((stroke_brush(&s).as_deref(), brushes(&s)), (Some("Wave"), brushes_before.clone()));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!((stroke_brush(&s).as_deref(), label(&s)), (Some("Teardrop"), "Apply Brush".to_string()));
}

#[test]
fn pasted_brushed_art_brings_the_patterns_its_brush_art_uses() {
    let mut s = kit();
    let line = run(&mut s, "shape.line", json!({"x1": 10, "y1": 100, "x2": 150, "y2": 120}))["id"].as_u64().unwrap();
    run(&mut s, "brush.apply", json!({"name": "Dotted Band", "ids": [line]}));
    run(&mut s, "select.set", json!({"ids": [line]}));
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    run(&mut s, "edit.paste", json!({}));
    assert!(vectorcraft_brush::find(doc(&s), "Dotted Band").is_some());
    assert!(doc(&s).pattern("Dots").is_some(), "the brush's art is filled with it");
}

#[test]
fn other_documents_brushes_load_and_unreadable_files_are_errors() {
    let mut s = kit();
    // Another document's brushes load as a library, without its art.
    let b64 = run(&mut s, "document.serialize", json!({}))["dataBase64"].as_str().unwrap().to_string();
    let count = brushes(&s).len();
    let r = run(&mut s, "brush.loadLibrary", json!({"dataBase64": b64, "name": "Poster.vectorcraft"}));
    assert_eq!((r["name"].as_str(), r["count"].as_u64()), (Some("Poster"), Some(count as u64)));
    let (_, lib) = cmd::brushlib::library(&s, r["library"].as_str().unwrap()).unwrap();
    assert!(lib.doc.layers.iter().all(|l| l.children().is_none_or(|c| c.is_empty())));
    assert!(lib.brush("Dotted Band").is_some() && lib.doc.pattern("Dots").is_some());
    // An SVG has the default brushes, as it does opened.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect width="10" height="10"/></svg>"##;
    let r = run(&mut s, "brush.loadLibrary", json!({"data": svg, "name": "Box.svg"}));
    assert_eq!(r["count"].as_u64(), Some(vectorcraft_brush::defaults().len() as u64), "{r}");
    // Files that aren't documents, missing files and damaged native files are errors.
    assert!(s.execute("brush.loadLibrary", &json!({"data": "hello", "name": "x.txt"})).is_err());
    assert!(s.execute("brush.loadLibrary", &json!({"path": "/no/such/Library.vectorcraft"})).is_err());
    let dir = folder("bad");
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("Broken.vectorcraft");
    std::fs::write(&bad, b"{\"format\": \"vectorcraft\", \"doc\": 7").unwrap();
    assert!(s.execute("brush.loadLibrary", &json!({"path": bad.to_string_lossy()})).is_err());
    // A damaged file in the user folder isn't listed.
    s.brush_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let list = run(&mut s, "brush.libraries", json!({}));
    assert!(list["libraries"].as_array().unwrap().iter().all(|l| l["category"] != "user"));
    s.brush_libraries.set_user_dir(None);
    let _ = std::fs::remove_dir_all(dir);
    // No such brush, or a headless session's user folder, is an error.
    assert!(s.execute("brush.saveLibrary", &json!({"names": ["Nope"]})).is_err());
    assert!(s.execute("brush.saveLibrary", &json!({"user": true})).is_err());
    // A document whose brushes were all deleted has none to save.
    let all = brushes(&s);
    run(&mut s, "brush.delete", json!({ "names": all }));
    assert!(s.execute("brush.saveLibrary", &json!({})).is_err());
    // Without a path the library comes back as the file's bytes.
    let mut s = kit();
    let r = run(&mut s, "brush.saveLibrary", json!({"names": ["My Nib"]}));
    let bytes = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
    assert_eq!(vectorcraft_brush::library(&vectorcraft_format::load(&bytes).unwrap()).len(), 1);
}
