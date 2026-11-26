//! Generator plug-ins (`"kind": "generator"`, docs/plugins.md › Generators) through the engine: the
//! generated art objects `plugin.generate` makes, `plugin.editGenerated` drawing them again, Expand,
//! and what a saved file keeps and says when the plug-in is missing, its fields are damaged or its
//! art no longer matches. Opening a file never runs a plug-in.
//!
//! A test binary of its own: installed plug-ins are process-wide.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_doc::Node;
use vectorcraft_doc::generated::State;
use vectorcraft_engine::Session;
use vectorcraft_plugins::wat::{self as tpl, Given};

/// A generator's art: a 10 pt square at (0, 0).
const SMALL: &str = r#"{"objects":[{"type":"path","name":"Small","path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[10,0]},{"p":[10,10]},{"p":[0,10]}]}]}}]}"#;

/// A generator's art: a 20 pt square at (0, 0) and a line.
const BIG: &str = r#"{"objects":[{"type":"path","name":"Big","path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[20,0]},{"p":[20,20]},{"p":[0,20]}]}]}},{"type":"path","name":"Line","path":{"subpaths":[{"closed":false,"anchors":[{"p":[0,30]},{"p":[20,30]}]}]}}]}"#;

/// The parameters of the test generators.
const PARAMS: &str = r#"{"size":{"type":"int","min":1,"max":100,"default":10},"label":{"type":"string","default":"a"}}"#;

/// Installs the plug-in written in WAT `src` (`replace`: in place of the one with its id).
fn install(s: &mut Session, src: &str, replace: bool) -> Value {
    let wasm = wat::parse_str(src).unwrap_or_else(|e| panic!("bad test WAT: {e}"));
    s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "replace": replace})).unwrap()
}

/// A generator `id` that draws [`BIG`] when its `size` is 20 and [`SMALL`] otherwise.
fn sizes(s: &mut Session, id: &str) {
    install(s, &tpl::when_given_else(&tpl::generator(id, PARAMS), Given::Params, r#""size":20"#, BIG, SMALL), false);
}

/// A new document with generated art from generator `id` → the object's id.
fn generated(s: &mut Session, id: &str) -> u64 {
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    let r = s.execute("plugin.generate", &json!({"id": id})).unwrap();
    assert_eq!(r["plugin"], json!(id));
    r["id"].as_u64().unwrap()
}

/// The node `id` of the active document.
fn node(s: &Session, id: u64) -> Node {
    s.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).cloned().unwrap()
}

/// The names of `id`'s children.
fn art(s: &Session, id: u64) -> Vec<String> {
    node(s, id).children().unwrap().iter().map(|c| c.name.clone().unwrap_or_default()).collect()
}

/// The undo steps of the active document.
fn steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// The active document saved as native text and opened again → the open's warnings.
fn reopen(s: &mut Session) -> Vec<String> {
    let bytes = vectorcraft_format::save(&s.doc().unwrap().doc, true);
    reopen_bytes(s, &bytes)
}

fn reopen_bytes(s: &mut Session, bytes: &[u8]) -> Vec<String> {
    let r = s.execute("document.open", &json!({"name": "Gen.vectorcraft", "dataBase64": vectorcraft_format::base64_encode(bytes)})).unwrap();
    r["warnings"].as_array().map(|w| w.iter().map(|w| w.as_str().unwrap_or_default().to_string()).collect()).unwrap_or_default()
}

/// The only generated art object of the active document → its id.
fn the_generated(s: &Session) -> u64 {
    let mut ids = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if n.extras().generated.is_some() {
            ids.push(n.id.0);
        }
    });
    assert_eq!(ids.len(), 1, "{ids:?}");
    ids[0]
}

#[test]
fn a_generator_draws_a_generated_art_object_as_one_undo_step() {
    let mut s = Session::new();
    sizes(&mut s, "test.gen.create");
    let entry = s.execute("plugin.info", &json!({"id": "test.gen.create"})).unwrap();
    assert_eq!((&entry["kind"], &entry["permissions"]), (&json!("generator"), &json!(["doc.write"])));
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    let before = steps(&s);
    let id = s.execute("plugin.generate", &json!({"id": "test.gen.create"})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(steps(&s), before + 1);
    assert_eq!(art(&s, id), ["Small"]);
    let n = node(&s, id);
    let g = n.extras().generated.as_deref().unwrap();
    assert_eq!((g.plugin.as_str(), g.name.as_str(), g.state), ("test.gen.create", "Test test.gen.create", State::Intact));
    assert_eq!(serde_json::to_value(&g.params).unwrap(), json!({"size": 10, "label": "a"}));
    assert_eq!(s.doc().unwrap().selection.objects, [vectorcraft_doc::NodeId(id)]);
    // Given parameters are checked: a bad one is an error and nothing is added.
    for bad in [json!({"size": 500}), json!({"size": "big"}), json!({"color": 1})] {
        assert!(s.execute("plugin.generate", &json!({"id": "test.gen.create", "params": bad})).is_err(), "{bad}");
    }
    assert_eq!(steps(&s), before + 1);
    // Another kind of plug-in is not a generator, and plugin.run doesn't run a generator.
    install(&mut s, &tpl::module(&tpl::manifest("test.gen.filter", "filter", "{}"), tpl::ECHO, "", 1), false);
    let e = s.execute("plugin.generate", &json!({"id": "test.gen.filter"})).unwrap_err().to_string();
    assert!(e.contains("not a generator"), "{e}");
    let e = s.execute("plugin.run", &json!({"id": "test.gen.create"})).unwrap_err().to_string();
    assert!(e.contains("plugin.generate"), "{e}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).is_none());
}

#[test]
fn editing_the_parameters_draws_the_art_again_as_one_undo_step() {
    let mut s = Session::new();
    sizes(&mut s, "test.gen.edit");
    let id = generated(&mut s, "test.gen.edit");
    // Moved since it was drawn: the new art is drawn where the object is.
    s.execute("object.move", &json!({"dx": 100, "dy": 50})).unwrap();
    let before = steps(&s);
    let r = s.execute("plugin.editGenerated", &json!({"params": {"size": 20}})).unwrap();
    assert_eq!((&r["id"], &r["changed"]), (&json!(id), &json!(true)));
    assert_eq!(steps(&s), before + 1);
    assert_eq!(art(&s, id), ["Big", "Line"]);
    let b = s.doc().unwrap().doc.bounds_of(&[vectorcraft_doc::NodeId(id)], false).unwrap();
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (100.0, 50.0, 120.0, 80.0));
    let n = node(&s, id);
    let g = n.extras().generated.as_deref().unwrap();
    assert_eq!(serde_json::to_value(&g.params).unwrap(), json!({"size": 20, "label": "a"}));
    // The same parameters again change nothing and make no undo step.
    let r = s.execute("plugin.editGenerated", &json!({"id": id, "params": {"size": 20}})).unwrap();
    assert_eq!(r["changed"], json!(false));
    assert_eq!(steps(&s), before + 1);
    // A bad parameter is an error, and the art stays.
    for bad in [json!({"size": 0}), json!({"size": true}), json!({"other": 1})] {
        assert!(s.execute("plugin.editGenerated", &json!({"id": id, "params": bad})).is_err(), "{bad}");
    }
    assert!(s.execute("plugin.editGenerated", &json!({"id": id, "params": [1]})).is_err());
    assert_eq!((steps(&s), art(&s, id)), (before + 1, vec!["Big".to_string(), "Line".to_string()]));
    // One undo step brings back the art and the parameters it had.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(art(&s, id), ["Small"]);
    let n = node(&s, id);
    assert_eq!(serde_json::to_value(&n.extras().generated.as_deref().unwrap().params).unwrap(), json!({"size": 10, "label": "a"}));
    // Nothing selected that is generated art.
    s.execute("select.none", &json!({})).unwrap();
    let e = s.execute("plugin.editGenerated", &json!({"params": {"size": 20}})).unwrap_err().to_string();
    assert!(e.contains("select one generated art object"), "{e}");
    // The generated art object selected with another object: neither command picks it out.
    let square = s.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 10, "height": 10})).unwrap();
    s.execute("select.set", &json!({"ids": [id, square["id"]]})).unwrap();
    for (c, p) in [("plugin.editGenerated", json!({"params": {"size": 20}})), ("plugin.generatedInfo", json!({}))] {
        let e = s.execute(c, &p).unwrap_err().to_string();
        assert!(e.contains("select one generated art object"), "{c}: {e}");
    }
    assert_eq!(art(&s, id), ["Small"]);
}

/// A generator's art: a line with a 2 pt stroke.
const STROKED: &str = r#"{"objects":[{"type":"path","name":"Stroked","path":{"subpaths":[{"closed":false,"anchors":[{"p":[0,0]},{"p":[10,0]}]}]},"strokes":[{"paint":{"type":"solid","color":{"model":"rgb","r":0,"g":0,"b":0}},"width":2}]}]}"#;

/// The stroke weights of the art of generated art object `id`.
fn stroke_widths(s: &Session, id: u64) -> Vec<f64> {
    let n = node(s, id);
    let mut widths = vec![];
    for c in n.children().unwrap() {
        for it in &c.appearance.items {
            if let vectorcraft_doc::AppearanceItem::Stroke(st) = it {
                widths.push(st.width);
            }
        }
    }
    widths
}

#[test]
fn drawing_again_keeps_the_stroke_weights_a_scale_gave_the_art() {
    let mut s = Session::new();
    install(&mut s, &tpl::returning(&tpl::generator("test.gen.strokes", PARAMS), STROKED), false);
    let id = generated(&mut s, "test.gen.strokes");
    assert_eq!(stroke_widths(&s, id), [2.0]);
    // Scaled to 200% with Scale Strokes & Effects: the stroke doubles, and stays doubled when the
    // art is drawn again.
    s.execute("object.scale", &json!({"sx": 200, "strokes": true})).unwrap();
    assert_eq!(stroke_widths(&s, id), [4.0]);
    s.execute("plugin.editGenerated", &json!({"params": {"label": "b"}})).unwrap();
    assert_eq!(stroke_widths(&s, id), [4.0]);
    // Scaled to 50% without it: the stroke keeps its weight, drawn again too.
    s.execute("object.scale", &json!({"sx": 50, "strokes": false})).unwrap();
    assert_eq!(stroke_widths(&s, id), [4.0]);
    s.execute("plugin.editGenerated", &json!({"params": {"label": "c"}})).unwrap();
    assert_eq!(stroke_widths(&s, id), [4.0]);
    // A saved file keeps what the strokes were scaled by.
    reopen(&mut s);
    let id = the_generated(&s);
    s.execute("plugin.editGenerated", &json!({"id": id, "params": {"label": "d"}})).unwrap();
    assert_eq!(stroke_widths(&s, id), [4.0]);
}

#[test]
fn a_plug_in_of_another_kind_in_place_of_the_generator_is_named_on_open() {
    let mut s = Session::new();
    sizes(&mut s, "test.gen.kind");
    generated(&mut s, "test.gen.kind");
    let bytes = vectorcraft_format::save(&s.doc().unwrap().doc, true);
    install(&mut s, &tpl::module(&tpl::manifest("test.gen.kind", "filter", "{}"), tpl::ECHO, "", 1), true);
    let warnings = reopen_bytes(&mut s, &bytes);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("`test.gen.kind` is a plug-in of kind `filter`, not a generator"), "{warnings:?}");
    let id = the_generated(&s);
    assert_eq!(art(&s, id), ["Small"]);
    assert_eq!(s.execute("plugin.generatedInfo", &json!({"id": id})).unwrap()["installed"], json!(false));
}

#[test]
fn expand_makes_generated_art_an_ordinary_group() {
    let mut s = Session::new();
    sizes(&mut s, "test.gen.expand");
    let id = generated(&mut s, "test.gen.expand");
    s.execute("object.expand", &json!({"object": true})).unwrap();
    let n = node(&s, id);
    assert!(n.extras().generated.is_none());
    assert_eq!(art(&s, id), ["Small"]);
    let e = s.execute("plugin.editGenerated", &json!({"id": id, "params": {"size": 20}})).unwrap_err().to_string();
    assert!(e.contains("select one generated art object"), "{e}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(node(&s, id).extras().generated.is_some());
}

#[test]
fn a_saved_file_keeps_the_parameters_and_the_art_and_opening_runs_no_plug_in() {
    let mut s = Session::new();
    sizes(&mut s, "test.gen.save");
    generated(&mut s, "test.gen.save");
    s.execute("plugin.editGenerated", &json!({"params": {"size": 20, "label": "b"}})).unwrap();
    // The installed generator now always draws the small square: opening must not run it.
    install(&mut s, &tpl::returning(&tpl::generator("test.gen.save", PARAMS), SMALL), true);
    let warnings = reopen(&mut s);
    assert!(warnings.is_empty(), "{warnings:?}");
    let id = the_generated(&s);
    assert_eq!(art(&s, id), ["Big", "Line"]);
    let info = s.execute("plugin.generatedInfo", &json!({"id": id})).unwrap();
    assert_eq!((&info["plugin"], &info["state"], &info["installed"]), (&json!("test.gen.save"), &json!("intact"), &json!(true)));
    assert_eq!(info["params"], json!({"size": 20, "label": "b"}));
    // An explicit edit runs it.
    s.execute("plugin.editGenerated", &json!({"id": id, "params": {"label": "c"}})).unwrap();
    assert_eq!(art(&s, id), ["Small"]);
}

#[test]
fn a_missing_plug_in_keeps_the_art_and_is_named() {
    let mut s = Session::new();
    sizes(&mut s, "test.gen.gone");
    generated(&mut s, "test.gen.gone");
    let bytes = vectorcraft_format::save(&s.doc().unwrap().doc, true);
    s.execute("plugin.remove", &json!({"id": "test.gen.gone"})).unwrap();
    let warnings = reopen_bytes(&mut s, &bytes);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("“Test test.gen.gone” (test.gen.gone) isn't installed"), "{warnings:?}");
    let id = the_generated(&s);
    assert_eq!(art(&s, id), ["Small"]);
    let info = s.execute("plugin.generatedInfo", &json!({})).unwrap_or_else(|_| s.execute("plugin.generatedInfo", &json!({"id": id})).unwrap());
    assert_eq!((&info["state"], &info["installed"]), (&json!("intact"), &json!(false)));
    let e = s.execute("plugin.editGenerated", &json!({"id": id, "params": {"size": 20}})).unwrap_err().to_string();
    assert!(e.contains("isn't installed"), "{e}");
    assert_eq!(art(&s, id), ["Small"]);
    // Saved again without the plug-in, it keeps its fields.
    let again = vectorcraft_format::save(&s.doc().unwrap().doc, true);
    sizes(&mut s, "test.gen.gone");
    assert!(reopen_bytes(&mut s, &again).is_empty());
    let id = the_generated(&s);
    s.execute("plugin.editGenerated", &json!({"id": id, "params": {"size": 20}})).unwrap();
    assert_eq!(art(&s, id), ["Big", "Line"]);
}

#[test]
fn a_missing_generator_in_an_opacity_mask_is_named_on_open() {
    let mut s = Session::new();
    sizes(&mut s, "test.gen.mask.gone");
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    let target = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 30, "height": 30})).unwrap()["id"].clone();
    let mask = s.execute("plugin.generate", &json!({"id": "test.gen.mask.gone"})).unwrap()["id"].clone();
    s.execute("transparency.makeOpacityMask", &json!({"ids": [target, mask]})).unwrap();
    let bytes = vectorcraft_format::save(&s.doc().unwrap().doc, true);
    s.execute("plugin.remove", &json!({"id": "test.gen.mask.gone"})).unwrap();
    let warnings = reopen_bytes(&mut s, &bytes);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("test.gen.mask.gone") && warnings[0].contains("isn't installed"), "{warnings:?}");
}

/// The saved native text `bytes` with the generated art object's JSON changed by `f`.
fn tampered(bytes: &[u8], f: impl FnOnce(&mut Value)) -> Vec<u8> {
    fn find(v: &mut Value) -> Option<&mut Value> {
        match v {
            Value::Object(o) if o.contains_key("generated") => Some(v),
            Value::Object(o) => o.values_mut().find_map(find),
            Value::Array(a) => a.iter_mut().find_map(find),
            _ => None,
        }
    }
    let mut v: Value = serde_json::from_slice(bytes).unwrap();
    f(find(&mut v).unwrap());
    serde_json::to_vec(&v).unwrap()
}

#[test]
fn a_garbled_id_or_changed_art_keeps_the_art() {
    let mut s = Session::new();
    sizes(&mut s, "test.gen.garbled");
    generated(&mut s, "test.gen.garbled");
    let bytes = vectorcraft_format::save(&s.doc().unwrap().doc, true);
    for bad in [json!("test.gen.garbled/../x"), json!(""), json!(42), json!("t\u{0}x")] {
        let file = tampered(&bytes, |n| n["generated"]["plugin"] = bad.clone());
        let warnings = reopen_bytes(&mut s, &file);
        assert!(warnings.iter().any(|w| w.contains("plug-in id or parameters are damaged")), "{bad}: {warnings:?}");
        let id = the_generated(&s);
        assert_eq!(art(&s, id), ["Small"], "{bad}");
        let info = s.execute("plugin.generatedInfo", &json!({"id": id})).unwrap();
        assert_eq!((&info["state"], &info["installed"]), (&json!("damaged"), &json!(false)), "{bad}");
        let e = s.execute("plugin.editGenerated", &json!({"id": id, "params": {"size": 20}})).unwrap_err().to_string();
        assert!(e.contains("damaged"), "{bad}: {e}");
    }
    // Art edited outside the app no longer matches the hash: it is kept as it is.
    let file = tampered(&bytes, |n| {
        let child = &mut n["kind"]["children"][0];
        assert!(child.get("name").is_some(), "{child}");
        child["name"] = json!("Edited");
    });
    let warnings = reopen_bytes(&mut s, &file);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("doesn't match the hash its file gave"), "{warnings:?}");
    let id = the_generated(&s);
    assert_eq!(art(&s, id), ["Edited"]);
    assert_eq!(s.execute("plugin.generatedInfo", &json!({"id": id})).unwrap()["state"], json!("changed"));
    // An explicit edit may still draw it again with its plug-in.
    s.execute("plugin.editGenerated", &json!({"id": id, "params": {"size": 20}})).unwrap();
    assert_eq!(art(&s, id), ["Big", "Line"]);
}
