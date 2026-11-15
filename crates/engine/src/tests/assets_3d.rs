//! 3D assets: Export 3D Object collects objects with a 3D effect as 3D assets, their Format picks
//! the mesh file (USDA by default, USDZ, glTF or OBJ), and `assets.export3D` writes them, apart from
//! the picture assets `assets.export` writes.

use serde_json::{Value, json};
use vectorcraft_color::Paint;

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    // Unstroked art: each object is one solid.
    s.paint.stroke = Paint::None;
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A rectangle with Extrude & Bevel, named `name`.
fn extruded(s: &mut Session, x: f64, name: &str) -> u64 {
    let id = run(s, "shape.rectangle", json!({"x": x, "y": 10.0, "width": 40.0, "height": 30.0}))["id"].as_u64().unwrap();
    run(s, "select.set", json!({"ids": [id]}));
    run(s, "effect.apply", json!({"effect": "threeD.extrude", "params": {"depth": 20.0}}));
    run(s, "layer.setProps", json!({"id": id, "name": name}));
    id
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// `(name, bytes)` of each file an export returned.
fn files(r: &Value) -> Vec<(String, Vec<u8>)> {
    r["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["name"].as_str().unwrap().to_string(), vectorcraft_format::base64_decode(f["dataBase64"].as_str().unwrap()).unwrap()))
        .collect()
}

#[test]
fn export_3d_object_collects_each_3d_object_as_a_usda_asset() {
    let mut s = session();
    let a = extruded(&mut s, 10.0, "Box");
    let b = extruded(&mut s, 100.0, "Crate");
    run(&mut s, "select.set", json!({"ids": [a, b]}));
    let before = undo_len(&s);
    let r = run(&mut s, "assets.add3D", json!({}));
    assert_eq!(r["added"], 2);
    assert_eq!(undo_len(&s), before + 1, "one undo step for every object");
    let list = run(&mut s, "assets.list", json!({}))["assets"].as_array().unwrap().clone();
    assert_eq!(
        list.iter().map(|a| (a["name"].clone(), a["format3D"].clone())).collect::<Vec<_>>(),
        [(json!("Box"), json!("usda")), (json!("Crate"), json!("usda"))]
    );
    // Collected again: the same assets, and no undo step.
    let again = run(&mut s, "assets.add3D", json!({"ids": [a]}));
    assert_eq!((again["added"].clone(), again["assets"][0].clone()), (json!(0), r["assets"][0].clone()));
    assert_eq!(undo_len(&s), before + 1);
    // Collect for Export of the same object makes a picture asset beside it.
    assert_eq!(run(&mut s, "assets.add", json!({"ids": [a]}))["added"], 1);
    // Undo takes the 3D assets back out in one step.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().doc.assets.is_empty());
}

#[test]
fn export_3d_object_refuses_objects_without_a_3d_effect_and_bad_formats() {
    let mut s = session();
    let plain = run(&mut s, "shape.rectangle", json!({"x": 10.0, "y": 10.0, "width": 40.0, "height": 30.0}))["id"].as_u64().unwrap();
    let box3d = extruded(&mut s, 100.0, "Box");
    let before = undo_len(&s);
    assert!(s.execute("assets.add3D", &json!({"ids": [box3d, plain]})).is_err(), "one object without a 3D effect refuses them all");
    assert!(s.execute("assets.add3D", &json!({"ids": [box3d], "format": "fbx"})).is_err());
    assert!(s.execute("assets.add3D", &json!({"ids": [box3d], "format": 3})).is_err());
    assert!(s.execute("assets.add3D", &json!({"ids": [999_999]})).is_err());
    run(&mut s, "select.set", json!({"ids": []}));
    assert!(s.execute("assets.add3D", &json!({})).is_err(), "nothing selected");
    assert!(s.doc().unwrap().doc.assets.is_empty());
    assert_eq!(undo_len(&s), before);
    run(&mut s, "select.set", json!({"ids": [box3d]}));
    let r = run(&mut s, "assets.add3D", json!({"format": "gltf"}));
    assert_eq!(run(&mut s, "assets.list", json!({}))["assets"][0]["format3D"], "gltf");
    assert_eq!(r["added"], 1);
}

#[test]
fn the_format_is_set_per_asset_and_only_a_change_is_an_undo_step() {
    let mut s = session();
    let a = extruded(&mut s, 10.0, "Box");
    let asset = run(&mut s, "assets.add3D", json!({"ids": [a]}))["assets"][0].as_u64().unwrap();
    let picture = run(&mut s, "assets.add", json!({"ids": [a]}))["assets"][0].as_u64().unwrap();
    let before = undo_len(&s);
    assert_eq!(run(&mut s, "assets.set3DFormat", json!({"assets": [asset], "format": "usda"}))["changed"], 0);
    assert_eq!(undo_len(&s), before, "no change, no undo step");
    assert!(s.execute("assets.set3DFormat", &json!({"assets": [asset], "format": "stl"})).is_err());
    assert!(s.execute("assets.set3DFormat", &json!({"assets": [asset]})).is_err(), "the format is required");
    assert!(s.execute("assets.set3DFormat", &json!({"assets": [picture], "format": "obj"})).is_err(), "a picture asset has no 3D format");
    assert!(s.execute("assets.set3DFormat", &json!({"assets": [12345], "format": "obj"})).is_err());
    assert_eq!(run(&mut s, "assets.set3DFormat", json!({"assets": [asset], "format": "obj"}))["changed"], 1);
    assert_eq!(undo_len(&s), before + 1);
    assert_eq!(s.doc().unwrap().doc.asset(asset).unwrap().format_3d.as_deref(), Some("obj"));
}

#[test]
fn export_3d_writes_each_format() {
    let mut s = session();
    let a = extruded(&mut s, 10.0, "Box");
    let asset = run(&mut s, "assets.add3D", json!({"ids": [a]}))["assets"][0].as_u64().unwrap();
    let cases = [("usda", vec!["Box.usda"]), ("usdz", vec!["Box.usdz"]), ("gltf", vec!["Box.gltf"]), ("obj", vec!["Box.obj", "Box.mtl"])];
    for (format, names) in cases {
        run(&mut s, "assets.set3DFormat", json!({"assets": [asset], "format": format}));
        let out = files(&run(&mut s, "assets.export3D", json!({})));
        assert_eq!(out.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), names, "{format}");
        assert!(out.iter().all(|(_, b)| !b.is_empty()), "{format}");
    }
    // The OBJ holds the box: a rectangle's front and back and four walls, two triangles each.
    let out = files(&run(&mut s, "assets.export3D", json!({})));
    let obj = String::from_utf8(out[0].1.clone()).unwrap();
    assert_eq!(obj.lines().filter(|l| l.starts_with("f ")).count(), 12);
}

#[test]
fn export_3d_numbers_files_that_would_share_a_name_and_cleans_path_characters() {
    let mut s = session();
    let a = extruded(&mut s, 10.0, "Box");
    let b = extruded(&mut s, 100.0, "box");
    let c = extruded(&mut s, 200.0, "../a:b");
    run(&mut s, "assets.add3D", json!({"ids": [a, b, c]}));
    let out = files(&run(&mut s, "assets.export3D", json!({})));
    assert_eq!(out.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["Box.usda", "box-2.usda", "..-a-b.usda"]);
}

#[test]
fn picture_and_3d_assets_export_apart() {
    let mut s = session();
    let a = extruded(&mut s, 10.0, "Box");
    let three_d = run(&mut s, "assets.add3D", json!({"ids": [a]}))["assets"][0].as_u64().unwrap();
    // assets.export leaves 3D assets to assets.export3D.
    let e = s.execute("assets.export", &json!({"formats": [{"format": "png", "scale": "1x"}]})).unwrap_err().to_string();
    assert!(e.contains("no assets yet"), "{e}");
    let e = s.execute("assets.export", &json!({"assets": [three_d]})).unwrap_err().to_string();
    assert!(e.contains("assets.export3D"), "{e}");
    let e = s.execute("document.exportForScreens", &json!({"assets": [three_d]})).unwrap_err().to_string();
    assert!(e.contains("assets.export3D"), "Export for Screens refuses it too: {e}");
    let picture = run(&mut s, "assets.add", json!({"ids": [a]}))["assets"][0].as_u64().unwrap();
    let pngs = files(&run(&mut s, "assets.export", json!({"formats": [{"format": "png", "scale": "1x"}]})));
    assert_eq!(pngs.len(), 1);
    assert!(s.execute("assets.export3D", &json!({"assets": [picture]})).is_err());
}

#[test]
fn export_3d_needs_3d_assets() {
    let mut s = session();
    assert!(s.execute("assets.export3D", &json!({})).is_err());
    assert!(s.execute("assets.export3D", &json!({"assets": [77]})).is_err());
    assert!(s.execute("assets.export3D", &json!({"assets": "all"})).is_err());
}

#[test]
fn bad_3d_params_are_refused() {
    let mut s = session();
    let a = extruded(&mut s, 10.0, "Box");
    assert!(s.execute("assets.add3D", &json!({"ids": [a], "format": null})).is_err(), "a given format must be a format");
    run(&mut s, "assets.add3D", json!({"ids": [a]}));
    assert!(s.execute("assets.export3D", &json!({"folder": 123})).is_err());
    assert!(s.execute("assets.export3D", &json!({"zip": "true"})).is_err());
}

#[test]
fn an_asset_whose_object_lost_its_effect_is_refused_at_export() {
    let mut s = session();
    let a = extruded(&mut s, 10.0, "Box");
    run(&mut s, "assets.add3D", json!({"ids": [a]}));
    run(&mut s, "select.set", json!({"ids": [a]}));
    run(&mut s, "effect.remove", json!({"index": 0}));
    let e = s.execute("assets.export3D", &json!({})).unwrap_err().to_string();
    assert!(e.contains("Box"), "{e}");
}

#[test]
fn export_3d_writes_into_a_folder_or_one_zip() {
    let dir = std::env::temp_dir().join(format!("vc-assets-3d-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let folder = dir.to_string_lossy().replace('\\', "/");
    let mut s = session();
    let a = extruded(&mut s, 10.0, "Box");
    let asset = run(&mut s, "assets.add3D", json!({"ids": [a], "format": "obj"}))["assets"][0].as_u64().unwrap();
    let r = run(&mut s, "assets.export3D", json!({"folder": folder}));
    assert_eq!(r["files"], json!([format!("{folder}/Box.obj"), format!("{folder}/Box.mtl")]));
    assert!(dir.join("Box.obj").is_file() && dir.join("Box.mtl").is_file());
    let zip = run(&mut s, "assets.export3D", json!({"assets": [asset], "zip": true}));
    assert_eq!(zip["files"], json!(["Box.obj", "Box.mtl"]));
    assert!(zip["name"].as_str().unwrap().ends_with("-3D.zip"), "{}", zip["name"]);
    // A picture ZIP and a 3D ZIP in one folder are two files.
    run(&mut s, "assets.add", json!({"ids": [a]}));
    let pictures = run(&mut s, "assets.export", json!({"formats": [{"format": "png", "scale": "1x"}], "folder": folder, "zip": true}));
    let meshes = run(&mut s, "assets.export3D", json!({"folder": folder, "zip": true}));
    let (p, m) = (pictures["path"].as_str().unwrap(), meshes["path"].as_str().unwrap());
    assert_ne!(p, m);
    assert!(std::path::Path::new(p).is_file() && std::path::Path::new(m).is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn three_d_assets_round_trip_through_the_native_format() {
    let mut s = session();
    let a = extruded(&mut s, 10.0, "Box");
    run(&mut s, "assets.add3D", json!({"ids": [a], "format": "usdz"}));
    let saved = s.doc().unwrap().doc.assets.clone();
    let b64 = run(&mut s, "document.serialize", json!({"format": "vectorcraft"}))["dataBase64"].as_str().unwrap().to_string();
    run(&mut s, "document.open", json!({"name": "assets3d.vectorcraft", "dataBase64": b64}));
    assert_eq!(s.doc().unwrap().doc.assets, saved);
    assert_eq!(saved[0].format_3d.as_deref(), Some("usdz"));
}
