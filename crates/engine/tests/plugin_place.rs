//! Importer plug-ins whose manifests set `place` (docs/plugins.md › Importers) through `file.place`:
//! which files they are given, the group their objects become in the open document, and what happens
//! when they fail, decline or are canceled.
//!
//! A test binary of its own: installed plug-ins are process-wide, and the unit tests that check the
//! exact list of formats would see these ones.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_engine::cmd::fileio;
use vectorcraft_engine::{EngineError, Session};
use vectorcraft_plugins::wat::{self as tpl, Given};

/// An importer's output: a red rectangle "A" from (10, 20) to (110, 70) under a line "B" from
/// (10, 20) to (60, 20), and a warning.
const ART: &str = r#"{"objects":[{"type":"path","name":"A","path":{"subpaths":[{"closed":true,"anchors":[{"p":[10,20]},{"p":[110,20]},{"p":[110,70]},{"p":[10,70]}]}]},"fills":[{"type":"solid","color":{"model":"rgb","r":1,"g":0,"b":0}}]},{"type":"path","name":"B","path":{"subpaths":[{"closed":false,"anchors":[{"p":[10,20]},{"p":[60,20]}]}]}}],"warnings":["1 layer merged"]}"#;

/// An importer's output: the rectangle "A" of [`ART`] and a guide "G" from (500, 500) to (900, 500).
const GUIDED: &str = r#"{"objects":[{"type":"path","name":"A","path":{"subpaths":[{"closed":true,"anchors":[{"p":[10,20]},{"p":[110,20]},{"p":[110,70]},{"p":[10,70]}]}]},"fills":[{"type":"solid","color":{"model":"rgb","r":1,"g":0,"b":0}}]},{"type":"path","name":"G","guide":true,"path":{"subpaths":[{"closed":false,"anchors":[{"p":[500,500]},{"p":[900,500]}]}]}}]}"#;

/// The `vc_run` body of an importer that never returns.
const SPINS: &str = "(loop $l (br $l)) (i64.const 0)";

/// The manifest of an importer `id` reading `extensions` (a JSON list) that places its files.
fn placing(id: &str, extensions: &str) -> String {
    format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"importer","extensions":{extensions},"place":true}}"#)
}

/// Installs the plug-in written in WAT `src` → its `plugin.list` entry.
fn install(s: &mut Session, src: &str) -> Value {
    let wasm = wat::parse_str(src).unwrap_or_else(|e| panic!("bad test WAT: {e}"));
    s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap()
}

/// `file.place` of a file `name` holding `bytes`, with more params `extra`.
fn place(s: &mut Session, name: &str, bytes: &[u8], extra: Value) -> Result<Value, EngineError> {
    let mut p = json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)});
    if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(extra.clone());
    }
    s.execute("file.place", &p)
}

/// A session with a new document holding one rectangle, selected.
fn with_document() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 400.0, "y": 400.0, "width": 20.0, "height": 10.0})).unwrap();
    s
}

/// The active document's undo and redo steps, layers and content.
fn state(s: &mut Session) -> (usize, usize, usize, Value) {
    let content = s.execute("document.inspect", &json!({})).unwrap();
    let d = s.doc().unwrap();
    (d.history.undo.len(), d.history.redo.len(), d.doc.layers.len(), content)
}

#[test]
fn placing_importer_checks_the_transformed_bounds_before_editing() {
    let image = json!({"objects": [{
        "type": "image", "pixelSize": [1, 1], "pixelsBase64": vectorcraft_format::base64_encode(&[255, 0, 0, 255]),
        "transform": [20, 0, 0, 10, 0, 0],
    }]})
    .to_string();
    for (index, output, half_width) in [(0, ART, 50.0), (1, image.as_str(), 10.0)] {
        let mut s = with_document();
        let mut manifest: Value = serde_json::from_str(&placing(&format!("test.place.canvas{index}"), r#"["tpc"]"#)).unwrap();
        if index == 1 {
            manifest["api"] = json!(4);
            manifest["outputKinds"] = json!(["image"]);
        }
        install(&mut s, &tpl::returning(&manifest.to_string(), output));
        let importer = format!("plugin.test.place.canvas{index}");
        let before = state(&mut s);
        for at in [[4_000_000.0, 400.0], [-4_000_000.0, 400.0], [400.0, 4_000_000.0], [400.0, -4_000_000.0]] {
            for template in [false, true] {
                assert!(
                    place(&mut s, "Art.tpc", b"?", json!({"at": at, "template": template, "importer": importer})).is_err(),
                    "{index}: {at:?}, template: {template}"
                );
                assert_eq!(state(&mut s), before);
            }
        }
        let inside = place(&mut s, "Art.tpc", b"?", json!({"at": [4_000_000.0 - half_width, 400.0], "importer": importer}));
        assert!(inside.is_ok(), "{index}: {inside:?}");
        assert_eq!(s.doc().unwrap().history.undo.len(), before.0 + 1);
    }
}

#[test]
fn cropped_svg_placement_checks_art_outside_its_frame_before_editing() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path d="M-20 -20 L120 -20 L120 120 L-20 120 Z"/></svg>"#;
    for at in [[3_999_940.0, 400.0], [-3_999_940.0, 400.0], [400.0, 3_999_940.0], [400.0, -3_999_940.0]] {
        for template in [false, true] {
            let mut s = with_document();
            let before = state(&mut s);
            assert!(
                place(&mut s, "cropped.svg", svg, json!({"at": at, "crop": "crop", "template": template})).is_err(),
                "{at:?}, template: {template}"
            );
            assert_eq!(state(&mut s), before);
        }
    }
    let mut s = with_document();
    assert!(place(&mut s, "cropped.svg", svg, json!({"at": [3_999_930.0, 400.0], "crop": "crop"})).is_ok());
}

/// A placed group: its name, the names of its children, and its bounds as `(x0, y0, x1, y1)`.
type Placed = (Option<String>, Vec<Option<String>>, (f64, f64, f64, f64));

/// The group `id` of the active document.
fn object(s: &Session, id: u64) -> Placed {
    let d = &s.doc().unwrap().doc;
    let n = d.node(NodeId(id)).expect("placed");
    let kids = match &n.kind {
        NodeKind::Group { children, clip: false } => children.iter().map(|c| c.name.clone()).collect(),
        other => panic!("not a group: {other:?}"),
    };
    let b = n.geometric_bounds_with(&d.symbol_boxes()).unwrap();
    (n.name.clone(), kids, (b.x0, b.y0, b.x1, b.y1))
}

/// A file only a placing importer reads is placed as one group named after the file, its objects in
/// the order the importer returned them and centered on `at`, embedded (also with `link: true`), as
/// one undo step that undo takes back whole.
#[test]
fn a_placing_importer_places_one_group_as_one_undo_step() {
    let mut s = with_document();
    let entry = install(&mut s, &tpl::returning(&placing("test.place.one", r#"["tpo"]"#), ART));
    assert_eq!((&entry["place"], &entry["permissions"]), (&json!(true), &json!(["import.read"])));
    let formats = s.execute("document.formats", &json!({})).unwrap();
    let listed = formats["formats"].as_array().unwrap().iter().find(|f| f["id"] == "plugin.test.place.one").expect("listed");
    assert_eq!(listed["place"], json!(true));

    let before = state(&mut s);
    let r = place(&mut s, "Shapes.tpo", b"?", json!({"at": [300, 400], "link": true})).unwrap();
    assert_eq!(
        (&r["format"], &r["linked"], &r["warnings"], &r["name"]),
        (&json!("plugin.test.place.one"), &json!(false), &json!(["1 layer merged"]), &json!("Shapes.tpo"))
    );
    let ids = r["ids"].as_array().unwrap();
    assert_eq!(ids.len(), 1);
    let id = ids[0].as_u64().unwrap();
    let (name, kids, bounds) = object(&s, id);
    assert_eq!((name.as_deref(), kids), (Some("Shapes.tpo"), vec![Some("A".to_string()), Some("B".to_string())]));
    assert_eq!(bounds, (250.0, 375.0, 350.0, 425.0));
    assert_eq!(s.doc().unwrap().selection.objects, [NodeId(id)]);
    let after = state(&mut s);
    assert_eq!(after.0, before.0 + 1, "one undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    let undone = state(&mut s);
    assert_eq!((undone.0, undone.1, undone.2), (before.0, 1, before.2));
    assert_eq!((&undone.3["layers"], &undone.3["history"]), (&before.3["layers"], &before.3["history"]));
}

/// `rect` fits the group in with its aspect, Template puts it on a new template layer, Replace swaps
/// the selected object at its center and stacking place, and `file.place.info` and the place cursor
/// read the file the same way.
#[test]
fn placing_follows_rect_template_and_replace() {
    let mut s = with_document();
    install(&mut s, &tpl::returning(&placing("test.place.ways", r#"["tpw"]"#), ART));
    let r = place(&mut s, "a.tpw", b"?", json!({"rect": [0, 0, 50, 50]})).unwrap();
    assert_eq!(object(&s, r["ids"][0].as_u64().unwrap()).2, (0.0, 12.5, 50.0, 37.5));

    let layers = s.doc().unwrap().doc.layers.len();
    let r = place(&mut s, "b.tpw", b"?", json!({"template": true})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers.len(), layers + 1);
    let layer = d.layer_of(NodeId(r["ids"][0].as_u64().unwrap())).and_then(|l| d.node(l)).unwrap();
    assert!(matches!(layer.kind, NodeKind::Layer { template: true, .. }) && layer.locked);

    let rect = s.execute("shape.rectangle", &json!({"x": 400.0, "y": 100.0, "width": 20.0, "height": 10.0})).unwrap();
    let old = NodeId(rect["id"].as_u64().unwrap());
    let (parent, index, _) = s.doc().unwrap().doc.position(old).unwrap();
    let r = place(&mut s, "c.tpw", b"?", json!({"replace": true})).unwrap();
    let id = NodeId(r["ids"][0].as_u64().unwrap());
    let d = &s.doc().unwrap().doc;
    assert!(d.node(old).is_none());
    assert_eq!(d.position(id).map(|(p, i, _)| (p, i)), Some((parent, index)));
    assert_eq!(object(&s, id.0).2, (360.0, 80.0, 460.0, 130.0));

    let info = s.execute("file.place.info", &json!({"name": "d.tpw", "dataBase64": vectorcraft_format::base64_encode(b"?")})).unwrap();
    assert_eq!((&info["format"], &info["width"], &info["height"]), (&json!("plugin.test.place.ways"), &json!(100.0), &json!(50.0)));
    let q = s.execute("file.place.queue", &json!({"files": [{"name": "e.tpw", "dataBase64": vectorcraft_format::base64_encode(b"?")}]})).unwrap();
    assert_eq!((&q["count"], &q["files"][0]["format"], &q["skipped"]), (&json!(1), &json!("plugin.test.place.ways"), &json!([])));
}

/// Guides the importer returns are left out of the placed group, and its box is that of the art
/// left, so a guide beside the art doesn't move it; output of guides alone places nothing.
#[test]
fn guides_are_left_out_of_what_is_placed() {
    let mut s = with_document();
    install(&mut s, &tpl::returning(&placing("test.place.guided", r#"["tpg"]"#), GUIDED));
    let r = place(&mut s, "a.tpg", b"?", json!({"at": [300, 400]})).unwrap();
    let (_, kids, bounds) = object(&s, r["ids"][0].as_u64().unwrap());
    assert_eq!((kids, bounds), (vec![Some("A".to_string())], (250.0, 375.0, 350.0, 425.0)));
    let info = s.execute("file.place.info", &json!({"name": "b.tpg", "dataBase64": vectorcraft_format::base64_encode(b"?")})).unwrap();
    assert_eq!((&info["width"], &info["height"]), (&json!(100.0), &json!(50.0)));

    let guides = r#"{"objects":[{"type":"path","guide":true,"path":{"subpaths":[{"closed":false,"anchors":[{"p":[0,0]},{"p":[100,0]}]}]}}]}"#;
    install(&mut s, &tpl::returning(&placing("test.place.guides", r#"["tpq"]"#), guides));
    let before = state(&mut s);
    let e = place(&mut s, "c.tpq", b"?", json!({})).unwrap_err().to_string();
    assert!(e.contains("`c.tpq` has no art to place"), "{e}");
    assert_eq!(state(&mut s), before);
}

/// An importer at plug-in API level 4 places its type and images with its paths: the objects of its
/// visible layers go in the group, in order, the objects of a hidden layer are left out, and its
/// artboards leave the document's as they were.
#[test]
fn an_api_4_importer_places_type_and_images_of_its_visible_layers() {
    let mut s = with_document();
    let manifest = r#"{"id":"test.place.rich","name":"Test test.place.rich","version":"1","kind":"importer","api":4,"extensions":["tpr"],"place":true,"outputKinds":["type","image","layer","artboard"]}"#;
    let rect: Value = serde_json::from_str(ART).unwrap();
    let output = json!({
        "objects": [
            {"type": "type", "text": "Hello", "fontFamily": "No Such Family P9129", "fontSize": 24, "transform": [1, 0, 0, 1, 10, 40], "name": "Title"},
            {"type": "image", "pixelSize": [2, 1], "pixelsBase64": vectorcraft_format::base64_encode(&[255, 0, 0, 255, 0, 0, 255, 128]), "transform": [10, 0, 0, 10, 0, 100], "name": "Pic"},
            rect["objects"][0],
        ],
        "layers": [{"name": "Shown", "objects": [0, 1]}, {"name": "Hidden", "visible": false, "objects": [2]}],
        "artboards": [{"name": "Front", "rect": [0, 0, 200, 100]}],
    });
    install(&mut s, &tpl::returning(manifest, &output.to_string()));
    let boards = s.doc().unwrap().doc.artboards.clone();
    let r = place(&mut s, "Rich.tpr", b"?", json!({"at": [300, 400]})).unwrap();
    let id = NodeId(r["ids"][0].as_u64().unwrap());
    let d = &s.doc().unwrap().doc;
    let NodeKind::Group { children, .. } = &d.node(id).expect("placed").kind else { panic!("not a group") };
    let kinds: Vec<(Option<&str>, bool, bool)> = children
        .iter()
        .map(|c| (c.name.as_deref(), matches!(c.kind, NodeKind::Text(_)), matches!(&c.kind, NodeKind::Image(im) if d.images.contains_key(&im.key))))
        .collect();
    assert_eq!(kinds, [(Some("Title"), true, false), (Some("Pic"), false, true)]);
    assert_eq!(d.artboards, boards);
}

/// The importer is told it places the file (`_context.mode`), with the file's extension and name,
/// and is given its parameters.
#[test]
fn the_importer_is_told_it_places() {
    let mut s = with_document();
    let params = r#"{"density":{"type":"number","min":0,"max":10,"default":4}}"#;
    let manifest = placing("test.place.mode", r#"["tpm"]"#).replace(r#""place":true"#, &format!(r#""place":true,"params":{params}"#));
    for (i, needle) in [r#""mode":"place""#, r#""extension":"tpm""#, r#""name":"Shapes""#, r#""density":10.0"#].into_iter().enumerate() {
        // ART when the parameters hold the needle; the file, which isn't JSON, otherwise.
        let wasm = wat::parse_str(tpl::when_given(&manifest, Given::Params, needle, ART)).unwrap();
        s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "replace": i > 0})).unwrap();
        let r = place(&mut s, "Shapes.tpm", b"not json", json!({"params": {"density": 12}}));
        assert_eq!(r.map(|r| r["format"].clone()).map_err(|e| e.to_string()), Ok(json!("plugin.test.place.mode")), "{needle}");
    }
    let before = state(&mut s);
    let e = place(&mut s, "Shapes.tpm", b"", json!({"params": {"density": "lots"}})).unwrap_err();
    assert!(matches!(e, EngineError::BadParams { .. }), "{e}");
    assert_eq!(state(&mut s), before);
}

/// Built-in formats keep their files: an importer that lists `svg` places an `.svg` only when
/// `importer` names it. An importer whose manifest doesn't set `place` opens its files and places
/// none: File › Place doesn't read them, and naming it is an error, as is naming it for a file of an
/// extension it doesn't list.
#[test]
fn built_in_formats_and_open_only_importers_keep_their_files() {
    let mut s = with_document();
    install(&mut s, &tpl::returning(&placing("test.place.svg", r#"["tps","svg"]"#), ART));
    install(&mut s, &tpl::returning(&tpl::importer("test.place.openonly", r#"["tpx"]"#, "{}"), ART));
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="50" height="40"><rect width="10" height="10"/></svg>"#;
    assert_eq!(place(&mut s, "a.svg", svg, json!({})).unwrap()["format"], "svg");
    assert_eq!(place(&mut s, "b.svg", svg, json!({"importer": "plugin.test.place.svg"})).unwrap()["format"], "plugin.test.place.svg");
    assert_eq!(place(&mut s, "c.tps", svg, json!({})).unwrap()["format"], "svg", "an SVG named as the importer's file");

    let before = state(&mut s);
    let e = place(&mut s, "d.tpx", b"?", json!({})).unwrap_err().to_string();
    assert!(e.contains("not a format VectorCraft reads"), "{e}");
    let e = place(&mut s, "d.tpx", b"?", json!({"importer": "plugin.test.place.openonly"})).unwrap_err().to_string();
    assert!(e.contains("doesn't place them") && e.contains("Test test.place.openonly"), "{e}");
    let e = place(&mut s, "d.tpx", b"?", json!({"importer": "plugin.test.place.svg"})).unwrap_err().to_string();
    assert!(e.contains(".tps, .svg") && e.contains("d.tpx"), "{e}");
    let e = place(&mut s, "d.tps", b"?", json!({"importer": 3})).unwrap_err().to_string();
    assert!(e.contains("`importer` must be"), "{e}");
    assert_eq!(state(&mut s), before);
    // The open-only importer still opens its files.
    let r = s.execute("document.open", &json!({"name": "d.tpx", "dataBase64": vectorcraft_format::base64_encode(b"?")})).unwrap();
    assert_eq!(r["format"], "plugin.test.place.openonly");
}

/// File › Place's dialog lists a placing importer's extensions, under its name and in "All placeable
/// files", and not an open-only importer's; relinking an image keeps the built-in filters. A file only
/// a placing importer reads is dropped onto the canvas to be placed rather than opened.
#[test]
fn the_place_dialog_lists_placing_importers() {
    let mut s = Session::new();
    install(&mut s, &tpl::returning(&placing("test.place.list", r#"["tpl","svg"]"#), ART));
    install(&mut s, &tpl::returning(&tpl::importer("test.place.listopen", r#"["tpy"]"#, "{}"), ART));
    let filters: Vec<_> = fileio::place_dialog_filters().collect();
    assert!(filters.contains(&("Test test.place.list", &["tpl"][..])), "{filters:?}");
    let all = filters.first().unwrap();
    assert_eq!(all.0, "All placeable files");
    assert!(all.1.contains(&"tpl") && !all.1.contains(&"tpy"), "{all:?}");
    assert_eq!(all.1.iter().filter(|e| **e == "svg").count(), 1);
    assert!(!filters.iter().any(|(_, exts)| exts.contains(&"tpy")));
    assert!(!fileio::place_filters().any(|(_, exts)| exts.contains(&"tpl")));
    assert!(fileio::places_with_importer("x.TPL") && !fileio::opens_with_importer("x.tpl"));
    assert!(!fileio::places_with_importer("x.tpy") && fileio::opens_with_importer("x.tpy"));
    assert!(!fileio::places_with_importer("x.svg"));
}

/// Nothing is placed, and the document keeps its content and undo steps, when the importer declines,
/// returns output that doesn't pass the checks, traps or is canceled, or when the file is larger
/// than an importer reads: that file is refused from its size, before it is read.
#[test]
fn failures_place_nothing() {
    let mut s = with_document();
    let far = r#"{"objects":[{"type":"path","path":{"subpaths":[{"closed":false,"anchors":[{"p":[0,0]},{"p":[9000000,0]}]}]}}]}"#;
    let cases = [
        ("test.place.fail.declines", r#"{"error":"Not a drawing."}"#, "Not a drawing."),
        ("test.place.fail.notjson", "not json", "JSON"),
        ("test.place.fail.empty", r#"{"objects":[]}"#, "the importer returned no objects"),
        ("test.place.fail.key", r#"{"objects":[],"layers":[]}"#, "not `layers`"),
        ("test.place.fail.far", far, "4000000"),
        (
            "test.place.fail.point",
            r#"{"objects":[{"type":"path","path":{"subpaths":[{"closed":false,"anchors":[{"p":[5,5]}]}]}}]}"#,
            "no art to place",
        ),
    ];
    for (id, output, _) in cases {
        install(&mut s, &tpl::returning(&placing(id, r#"["tpf"]"#), output));
    }
    install(&mut s, &tpl::module(&placing("test.place.fail.traps", r#"["tpf"]"#), "(unreachable)", "", 1));
    install(&mut s, &tpl::module(&placing("test.place.fail.spins", r#"["tpf"]"#), SPINS, "", 1));
    let before = state(&mut s);
    for (id, _, why) in cases {
        let e = place(&mut s, "x.tpf", b"?", json!({"importer": format!("plugin.{id}")})).unwrap_err().to_string();
        assert!(e.contains(why) && e.contains("x.tpf"), "{id}: {e}");
        assert_eq!(state(&mut s), before, "{id}");
    }
    assert!(place(&mut s, "x.tpf", b"?", json!({"importer": "plugin.test.place.fail.traps"})).is_err());
    assert_eq!(state(&mut s), before, "trapped");
    let token = vectorcraft_doc::cancel::CancelToken::new();
    token.cancel();
    let r = vectorcraft_doc::cancel::scope(&token, None, || place(&mut s, "x.tpf", b"?", json!({"importer": "plugin.test.place.fail.spins"})));
    assert!(matches!(r, Err(EngineError::Cancelled)), "{r:?}");
    assert_eq!(state(&mut s), before, "canceled");

    // A file one byte past the limit, without data in it: the size alone refuses it.
    let dir = std::env::temp_dir().join(format!("vc-plugin-place-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let big = dir.join("big.tpf");
    std::fs::File::create(&big).unwrap().set_len(vectorcraft_plugins::import::MAX_INPUT_BYTES as u64 + 1).unwrap();
    let e = s.execute("file.place", &json!({"path": big.to_string_lossy(), "importer": "plugin.test.place.fail.declines"})).unwrap_err().to_string();
    assert!(e.contains("can't place `big.tpf`") && e.contains("larger than"), "{e}");
    let e = s.execute("file.place", &json!({"path": big.to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("larger than"), "by its extension: {e}");
    assert_eq!(state(&mut s), before, "too large");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file past the limit, placed with `importer` named, is refused from its size before any of it
/// is read: here the file can't be read at all, and the error is still the size.
#[test]
fn a_named_importer_refuses_a_large_file_before_reading_it() {
    let mut s = with_document();
    install(&mut s, &tpl::returning(&placing("test.place.unread", r#"["tpu"]"#), ART));
    let dir = std::env::temp_dir().join(format!("vc-plugin-place-unread-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let big = dir.join("big.tpu");
    std::fs::File::create(&big).unwrap().set_len(vectorcraft_plugins::import::MAX_INPUT_BYTES as u64 + 1).unwrap();
    // Unix: no read permission. Windows: another handle that shares nothing.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&big, std::fs::Permissions::from_mode(0o000)).unwrap();
    }
    #[cfg(windows)]
    let _held = {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().read(true).share_mode(0).open(&big).unwrap()
    };
    // A user that reads the file anyway (root) can't show the order of the checks.
    if std::fs::File::open(&big).is_ok() {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let before = state(&mut s);
    let e = s.execute("file.place", &json!({"path": big.to_string_lossy(), "importer": "plugin.test.place.unread"})).unwrap_err().to_string();
    assert!(e.contains("can't place `big.tpu`") && e.contains("larger than"), "{e}");
    assert_eq!(state(&mut s), before);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&big, std::fs::Permissions::from_mode(0o644));
    }
    #[cfg(windows)]
    drop(_held);
    let _ = std::fs::remove_dir_all(&dir);
}
