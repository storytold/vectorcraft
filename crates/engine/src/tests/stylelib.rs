//! Graphic style libraries: the built-in libraries, adding to the document (deduped, one undo step,
//! applied, patterns brought along), saving and loading `.vcstyles` and user libraries.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Paint, Swatch};
use vectorcraft_doc::{GraphicStyle, Node, NodeId, PatternDef};
use vectorcraft_geom::{Point, Rect, shapes};

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

fn names(v: &Value) -> Vec<&str> {
    v.as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect()
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    NodeId(run(s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h}))["id"].as_u64().unwrap())
}

#[test]
fn builtin_libraries_are_listed_with_original_styles_using_catalogued_effects() {
    let mut s = Session::new();
    let list = run(&mut s, "graphicStyle.libraries", json!({}));
    let libs = list["libraries"].as_array().unwrap();
    assert!(libs.len() >= 6 && libs.iter().all(|l| l["category"] == "builtIn"), "{libs:?}");
    for l in libs {
        let id = l["id"].as_str().unwrap();
        let g = run(&mut s, "graphicStyle.library", json!({"library": id}));
        let styles = g["styles"].as_array().unwrap();
        assert!(!styles.is_empty() && l["count"].as_u64() == Some(styles.len() as u64), "{id}");
        let (_, lib) = cmd::stylelib::library(&s, id).unwrap();
        for st in &lib.styles {
            let ap = &st.appearance;
            let item_fx = ap.items.iter().flat_map(|i| i.effects());
            for e in ap.effects.iter().chain(item_fx) {
                assert!(vectorcraft_render::effects::effect_info(&e.id).is_some(), "{id} / {}: unknown effect {}", st.name, e.id);
            }
        }
    }
    // By name in any case.
    assert_eq!(run(&mut s, "graphicStyle.library", json!({"library": "shadows and glows"}))["id"], "shadows-glows");
    assert!(s.execute("graphicStyle.library", &json!({"library": "nope"})).is_err());
}

#[test]
fn add_dedupes_names_as_one_undo_step() {
    let mut s = session();
    let before = (undo_len(&s), doc(&s).graphic_styles.len());
    let r = run(&mut s, "graphicStyle.addFromLibrary", json!({"library": "shadows-glows", "names": ["Halo", "Ember", "Halo"]}));
    assert_eq!((names(&r["added"]), names(&r["existing"])), (vec!["Halo", "Ember"], vec![]));
    assert_eq!(undo_len(&s), before.0 + 1, "one undo step");
    let d = doc(&s);
    assert_eq!(d.graphic_styles.len(), before.1 + 2);
    let halo = d.graphic_style("Halo").unwrap();
    assert!(halo.id != 0 && d.graphic_styles.iter().filter(|g| g.id == halo.id).count() == 1, "a fresh id");
    // Adding again finds them and changes nothing (no undo step).
    let r = run(&mut s, "graphicStyle.addFromLibrary", json!({"library": "shadows-glows", "name": "Halo"}));
    assert_eq!((names(&r["added"]), names(&r["existing"])), (vec![], vec!["Halo"]));
    assert_eq!(undo_len(&s), before.0 + 1);
    // A name taken by another look gets a number.
    let id = rect(&mut s, 0.0, 0.0, 20.0, 20.0);
    run(&mut s, "graphicStyle.new", json!({"name": "Feathered", "id": id.0}));
    let r = run(&mut s, "graphicStyle.addFromLibrary", json!({"library": "shadows-glows", "names": ["Feathered"]}));
    assert_eq!(names(&r["added"]), ["Feathered 2"]);
    assert_eq!(doc(&s).graphic_style("Feathered 2").unwrap().appearance.effects[0].id, "stylize.feather");
    // The whole library; undo removes a whole add.
    let r = run(&mut s, "graphicStyle.addFromLibrary", json!({"library": "hand-drawn"}));
    let all = cmd::stylelib::library(&s, "hand-drawn").unwrap().1.len();
    assert_eq!(r["added"].as_array().unwrap().len(), all);
    run(&mut s, "edit.undo", json!({}));
    assert!(doc(&s).graphic_style("Sketchy Line").is_none() && doc(&s).graphic_style("Feathered 2").is_some());
    assert!(s.execute("graphicStyle.addFromLibrary", &json!({"library": "hand-drawn", "name": "Nope"})).is_err());
}

#[test]
fn add_with_apply_styles_the_selection_in_the_same_undo_step() {
    let mut s = session();
    let id = rect(&mut s, 10.0, 20.0, 100.0, 50.0);
    let before = undo_len(&s);
    let r = run(&mut s, "graphicStyle.addFromLibrary", json!({"library": "gradient-finishes", "name": "Spotlight", "apply": true}));
    assert_eq!(r["applied"], "Spotlight");
    assert_eq!(undo_len(&s), before + 1, "added and applied as one step");
    let (g, linked) = s.selection_graphic_style().unwrap();
    assert_eq!((g.name.as_str(), linked), ("Spotlight", true));
    // The unit-box gradient lands at the same place relative to the object's bounds.
    let Paint::Gradient(gp) = doc(&s).node(id).unwrap().appearance.fill_paint() else { panic!("a gradient") };
    let geom = gp.geom.unwrap();
    assert!((geom.start - Point::new(10.0 + 0.35 * 100.0, 20.0 + 0.3 * 50.0)).hypot() < 1e-6, "{geom:?}");
    run(&mut s, "edit.undo", json!({}));
    assert!(doc(&s).graphic_style("Spotlight").is_none());
    assert_eq!(doc(&s).node(id).unwrap().extras().graphic_style, None);
    // Without a selection it only adds; `ids` targets objects; an existing style is applied as is.
    run(&mut s, "select.none", json!({}));
    let r = run(&mut s, "graphicStyle.addFromLibrary", json!({"library": "blends-transparency", "name": "Ghosted", "apply": true}));
    assert!(r.get("applied").is_none() && doc(&s).graphic_style("Ghosted").is_some());
    let r = run(&mut s, "graphicStyle.addFromLibrary", json!({"library": "blends-transparency", "name": "Ghosted", "apply": true, "ids": [id.0]}));
    assert_eq!((names(&r["existing"]), r["applied"].as_str()), (vec!["Ghosted"], Some("Ghosted")));
    assert_eq!(doc(&s).node(id).unwrap().opacity, 0.4);
}

#[test]
fn add_with_null_ids_and_no_selection_only_adds_style() {
    let mut omitted = session();
    let mut null_ids = session();
    let params = json!({"library": "shadows-glows", "name": "Halo", "apply": true});
    let expected = run(&mut omitted, "graphicStyle.addFromLibrary", params.clone());
    let mut with_null = params;
    with_null["ids"] = Value::Null;
    let actual = run(&mut null_ids, "graphicStyle.addFromLibrary", with_null);
    assert_eq!(actual, expected);
    assert_eq!(names(&actual["added"]), ["Halo"]);
    assert!(actual.get("applied").is_none());
    assert!(doc(&null_ids).graphic_style("Halo").is_some());
    assert_eq!(undo_len(&null_ids), undo_len(&omitted));
}

#[test]
fn add_with_null_ids_applies_to_selection() {
    let mut s = session();
    let id = rect(&mut s, 10.0, 20.0, 100.0, 50.0);
    let before = undo_len(&s);
    let result = run(&mut s, "graphicStyle.addFromLibrary", json!({"library": "shadows-glows", "name": "Halo", "apply": true, "ids": null}));
    assert_eq!(result["applied"], "Halo");
    assert_eq!(undo_len(&s), before + 1);
    let (style, linked) = s.selection_graphic_style().unwrap();
    assert_eq!((style.name.as_str(), linked), ("Halo", true));
    assert_eq!(doc(&s).node(id).unwrap().extras().graphic_style, Some(style.id));
}

/// A document with pattern `name` (a dot) and its swatch.
fn add_pattern(s: &mut Session, name: &str, radius: f64) {
    s.edit("Pattern", |d, _| {
        let dot = Node::path(d.alloc_id(), shapes::ellipse(Rect::new(0.0, 0.0, radius, radius)), vectorcraft_doc::Appearance::default_art());
        d.patterns.push(PatternDef { tile: Rect::new(0.0, 0.0, 10.0, 10.0), ..PatternDef::new(name, vec![Arc::new(dot)]) });
        d.swatches.push(Swatch { name: name.into(), paint: pattern(name), global: false, spot: false });
        Ok(())
    })
    .unwrap();
}

fn pattern(name: &str) -> Paint {
    Paint::Pattern { pattern: name.into(), xf: Default::default() }
}

#[test]
fn save_and_load_round_trip_with_patterns_and_unlinked_swatches() {
    let mut s = session();
    add_pattern(&mut s, "Dots", 4.0);
    run(&mut s, "swatch.edit", json!({"name": "Red", "global": true}));
    let id = rect(&mut s, 0.0, 0.0, 40.0, 40.0);
    run(&mut s, "paint.setFill", json!({"swatch": "Dots"}));
    run(&mut s, "paint.setStroke", json!({"swatch": "Red"}));
    run(&mut s, "transparency.set", json!({"opacity": 50, "blend": "Multiply"}));
    run(&mut s, "graphicStyle.new", json!({"name": "Dotted", "id": id.0}));
    let r = run(&mut s, "graphicStyle.saveLibrary", json!({"names": ["Dotted", "Sunshine"], "name": "Kit"}));
    assert_eq!(r["count"], 2);
    let data = r["data"].as_str().unwrap().to_string();
    assert!(data.contains("\"vcstyles\""));
    let r = run(&mut s, "graphicStyle.loadLibrary", json!({"data": data, "name": "kit.vcstyles"}));
    assert_eq!((r["name"].as_str(), r["count"].as_u64()), (Some("Kit"), Some(2)));
    let lib_id = r["library"].as_str().unwrap().to_string();
    let (_, lib) = cmd::stylelib::library(&s, &lib_id).unwrap();
    let dotted = lib.style("Dotted").unwrap();
    let mine = doc(&s).graphic_style("Dotted").unwrap();
    assert!(dotted.same_look(&mine.standalone()) && (dotted.opacity, dotted.id) == (0.5, 0));
    let red = doc(&s).swatch("Red").and_then(|w| w.paint.color()).unwrap();
    assert_eq!(mine.appearance.stroke_paint(), Paint::Solid { color: red, swatch: Some("Red".into()), tint: 1.0 });
    assert_eq!(dotted.appearance.stroke_paint(), Paint::solid(red), "unlinked from the swatch");
    assert_eq!(lib.patterns.len(), 1);
    let list = run(&mut s, "graphicStyle.libraries", json!({}));
    assert!(list["libraries"].as_array().unwrap().iter().any(|l| l["id"] == lib_id.as_str() && l["category"] == "loaded"));

    // Into another document: the pattern comes along (with its swatch), once.
    run(&mut s, "file.new", json!({"width": 100, "height": 100}));
    run(&mut s, "graphicStyle.addFromLibrary", json!({"library": lib_id, "name": "Dotted"}));
    let d = doc(&s);
    assert_eq!(d.pattern("Dots"), lib.patterns.first());
    assert!(d.swatch("Dots").is_some());
    assert_eq!(d.graphic_style("Dotted").unwrap().appearance.fill_paint(), pattern("Dots"));
    // A document whose "Dots" is another pattern gets the library's under a free name.
    run(&mut s, "file.new", json!({"width": 100, "height": 100}));
    add_pattern(&mut s, "Dots", 8.0);
    run(&mut s, "graphicStyle.addFromLibrary", json!({"library": lib_id}));
    let d = doc(&s);
    assert_eq!(d.graphic_style("Dotted").unwrap().appearance.fill_paint(), pattern("Dots 2"));
    assert_eq!(d.patterns.len(), 2);
    assert!(s.execute("graphicStyle.saveLibrary", &json!({"names": ["Nope"]})).is_err());
    assert!(s.execute("graphicStyle.saveLibrary", &json!({"user": true})).is_err(), "no user folder in a headless session");
}

#[test]
fn user_libraries_and_other_documents_styles() {
    let dir = std::env::temp_dir().join(format!("vc-stylelib-user-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = session();
    s.style_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let r = run(&mut s, "graphicStyle.saveLibrary", json!({"user": true, "name": "My: Looks"}));
    assert_eq!(r["library"], "user/My- Looks.vcstyles");
    let path = r["path"].as_str().unwrap().to_string();
    let list = run(&mut s, "graphicStyle.libraries", json!({}));
    let user: Vec<&Value> = list["libraries"].as_array().unwrap().iter().filter(|l| l["category"] == "user").collect();
    assert_eq!(user.len(), 1);
    assert_eq!((user[0]["name"].as_str(), user[0]["count"].as_u64()), (Some("My: Looks"), Some(doc(&s).graphic_styles.len() as u64)));
    assert_eq!(list["userFolder"].as_str(), Some(dir.to_string_lossy().as_ref()));
    // Opening a file of the user folder gives its User Defined library, and so does a file with the
    // same bytes in another folder.
    assert_eq!(run(&mut s, "graphicStyle.loadLibrary", json!({ "path": path }))["library"], "user/My- Looks.vcstyles");
    let elsewhere = dir.with_file_name(format!("vc-stylelib-elsewhere-{}", std::process::id()));
    std::fs::create_dir_all(&elsewhere).unwrap();
    let copy = elsewhere.join("Looks.vcstyles");
    std::fs::copy(&path, &copy).unwrap();
    assert_eq!(run(&mut s, "graphicStyle.loadLibrary", json!({ "path": copy.to_string_lossy() }))["library"], "user/My- Looks.vcstyles");
    let _ = std::fs::remove_dir_all(elsewhere);
    let _ = std::fs::remove_dir_all(dir);

    // Another document's graphic styles load as a library.
    let id = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    run(&mut s, "graphicStyle.new", json!({"name": "Poster Look", "id": id.0}));
    let b64 = run(&mut s, "document.serialize", json!({}))["dataBase64"].as_str().unwrap().to_string();
    let r = run(&mut s, "graphicStyle.loadLibrary", json!({"dataBase64": b64, "name": "Poster.vectorcraft"}));
    assert_eq!(r["name"], "Poster");
    let (_, lib) = cmd::stylelib::library(&s, r["library"].as_str().unwrap()).unwrap();
    assert!(lib.style("Poster Look").is_some_and(|g: &GraphicStyle| g.id == 0));
    assert!(s.execute("graphicStyle.loadLibrary", &json!({"data": "hello", "name": "x.txt"})).is_err());
}

#[test]
fn every_builtin_style_paints_visibly_on_a_rectangle() {
    let mut s = session();
    let mut renderer = vectorcraft_render::Renderer::new();
    for b in vectorcraft_doc::style_libs::STYLE_LIBRARIES {
        let (_, lib) = cmd::stylelib::library(&s, b.id).unwrap();
        for g in &lib.styles {
            run(&mut s, "file.new", json!({"width": 100, "height": 100}));
            let id = rect(&mut s, 25.0, 30.0, 50.0, 40.0);
            run(&mut s, "graphicStyle.addFromLibrary", json!({"library": b.id, "name": g.name, "apply": true}));
            assert_eq!(doc(&s).node(id).unwrap().extras().graphic_style, doc(&s).graphic_style(&g.name).map(|x| x.id), "{}", g.name);
            let img = renderer.render_region(doc(&s), Rect::new(0.0, 0.0, 100.0, 100.0), 0.5, true);
            let painted = img.pixels.as_chunks::<4>().0.iter().filter(|p| p[..3] != [255, 255, 255]).count();
            assert!(painted > 40, "{} / {} paints {painted} pixels", b.id, g.name);
        }
    }
}
