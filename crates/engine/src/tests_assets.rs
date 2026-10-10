//! Asset Export: collecting art as assets (single and multiple), assets following their art
//! (moves, deletions), names, undo, the native round trip, the shared export settings and
//! exporting assets alone, cropped to their art.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    // Unstroked art: visual bounds are the geometry.
    s.paint.stroke = Paint::None;
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> u64 {
    run(s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h}))["id"].as_u64().unwrap()
}

fn assets(s: &mut Session) -> Vec<Value> {
    run(s, "assets.list", json!({}))["assets"].as_array().unwrap().clone()
}

fn names(s: &mut Session) -> Vec<String> {
    assets(s).iter().map(|a| a["name"].as_str().unwrap().to_string()).collect()
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
fn multiple_gives_an_asset_per_object_and_single_one_of_them_all() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 10.0, 10.0, 20.0, 20.0), rect(&mut s, 50.0, 10.0, 30.0, 20.0));
    run(&mut s, "select.set", json!({"ids": [a, b]}));
    let r = run(&mut s, "assets.add", json!({}));
    assert_eq!(r["added"], 2);
    let list = assets(&mut s);
    assert_eq!(list.len(), 2);
    assert_eq!((list[0]["nodes"].clone(), list[1]["nodes"].clone()), (json!([a]), json!([b])));
    assert_eq!(r["assets"], json!([list[0]["id"], list[1]["id"]]));
    let single = run(&mut s, "assets.add", json!({"multiple": false}));
    assert_eq!(single["added"], 1);
    let list = assets(&mut s);
    assert_eq!(list.len(), 3);
    assert_eq!(list[2]["nodes"], json!([a, b]), "one asset of both, back to front");
    assert_eq!(list[2]["bounds"], json!([10.0, 10.0, 70.0, 20.0]), "the crop is the art's bounds");
    // Collecting the same art again keeps its assets (and isn't an undo step).
    let steps = undo_len(&s);
    let again = run(&mut s, "assets.add", json!({"ids": [b, a]}));
    assert_eq!((again["added"].clone(), again["assets"].clone()), (json!(0), json!([list[0]["id"], list[1]["id"]])));
    assert_eq!(undo_len(&s), steps);
    // Layers aren't art; nothing to collect is an error.
    let layer = s.doc().unwrap().doc.layers[0].id.0;
    assert!(s.execute("assets.add", &json!({"ids": [layer]})).is_err());
    run(&mut s, "select.set", json!({"ids": []}));
    assert!(s.execute("assets.add", &json!({})).is_err());
    assert!(s.execute("assets.add", &json!({"ids": [a, b], "name": "Two"})).is_err(), "a name names one asset");
}

#[test]
fn deleting_the_object_prunes_it_and_undo_brings_it_back() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 10.0, 10.0, 20.0, 20.0), rect(&mut s, 50.0, 10.0, 30.0, 20.0));
    run(&mut s, "assets.add", json!({"ids": [a]}));
    run(&mut s, "assets.add", json!({"ids": [a, b], "multiple": false}));
    run(&mut s, "select.set", json!({"ids": [a]}));
    run(&mut s, "edit.clear", json!({}));
    let list = assets(&mut s);
    assert_eq!(list.len(), 1, "the asset of the deleted object goes");
    assert_eq!(list[0]["nodes"], json!([b]), "the other keeps the art that is left");
    run(&mut s, "edit.undo", json!({}));
    let list = assets(&mut s);
    assert_eq!((list.len(), list[1]["nodes"].clone()), (2, json!([a, b])));
}

#[test]
fn moving_the_art_moves_the_crop() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    run(&mut s, "assets.add", json!({"ids": [a]}));
    let before = files(&run(&mut s, "assets.export", json!({})));
    run(&mut s, "select.set", json!({"ids": [a]}));
    run(&mut s, "object.move", json!({"dx": 40, "dy": 25}));
    assert_eq!(assets(&mut s)[0]["bounds"], json!([50.0, 35.0, 20.0, 20.0]));
    let after = files(&run(&mut s, "assets.export", json!({})));
    assert_eq!(before, after, "the same art, cropped the same way");
    assert_eq!(after[0].0, "Asset-1.png");
}

#[test]
fn an_asset_exports_its_art_alone() {
    let mut s = session();
    s.paint.fill = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let red = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    // Blue art over it isn't part of the asset.
    s.paint.fill = Paint::solid(Color::rgb(0.0, 0.0, 1.0));
    rect(&mut s, 0.0, 0.0, 25.0, 25.0);
    run(&mut s, "assets.add", json!({"ids": [red]}));
    let out = files(&run(&mut s, "assets.export", json!({"formats": [{"format": "png", "scale": "2x"}]})));
    assert_eq!(out[0].0, "Asset-1@2x.png");
    let img = image::load_from_memory(&out[0].1).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (40, 40));
    assert!(img.pixels().all(|p| p.0 == [255, 0, 0, 255]), "only the red square");
    // Vector formats crop to it too.
    let svg = files(&run(&mut s, "assets.export", json!({"formats": [{"format": "svg"}]})));
    let text = String::from_utf8(svg[0].1.clone()).unwrap();
    assert_eq!(svg[0].0, "Asset-1.svg");
    assert!(text.contains("viewBox=\"10 10 20 20\"") || text.contains("viewBox=\"0 0 20 20\""), "{text}");
    assert!(!text.to_lowercase().contains("#0000ff"), "no blue: {text}");
}

/// #983: the fractional shared edge is covered by the asset, even after scaling and cropping.
#[test]
fn asset_export_has_no_ghost_border_at_a_shared_edge() {
    let mut s = session();
    s.paint.fill = Paint::solid(Color::BLACK);
    let a = rect(&mut s, 10.3, 20.7, 16.375, 32.0);
    let b = rect(&mut s, 26.675, 20.7, 15.625, 32.0);
    run(&mut s, "assets.add", json!({"ids": [a, b], "multiple": false}));
    for scale in ["1x", "1.5x", "2x"] {
        for background in ["transparent", "white"] {
            let out = files(&run(&mut s, "assets.export", json!({"formats": [{"format": "png", "scale": scale, "background": background}]})));
            let img = image::load_from_memory(&out[0].1).unwrap().to_rgba8();
            assert!(img.pixels().all(|p| p.0 == [0, 0, 0, 255]), "{scale}, {background}: a solid black asset");
        }
    }
}

#[test]
fn names_follow_the_object_or_count_up_and_can_be_changed() {
    let mut s = session();
    let (a, b, c) = (rect(&mut s, 10.0, 10.0, 20.0, 20.0), rect(&mut s, 50.0, 10.0, 20.0, 20.0), rect(&mut s, 90.0, 10.0, 20.0, 20.0));
    run(&mut s, "layer.setProps", json!({"id": b, "name": "Icon"}));
    run(&mut s, "assets.add", json!({"ids": [a, b]}));
    assert_eq!(names(&mut s), ["Asset 1", "Icon"]);
    let first = assets(&mut s)[0]["id"].as_u64().unwrap();
    run(&mut s, "assets.rename", json!({"asset": first, "name": "  Asset 7 "}));
    run(&mut s, "assets.add", json!({"ids": [c]}));
    assert_eq!(names(&mut s), ["Asset 7", "Icon", "Asset 8"], "trimmed; numbering goes on from the highest");
    run(&mut s, "assets.add", json!({"ids": [a, c], "multiple": false, "name": "Pair"}));
    assert_eq!(names(&mut s)[3], "Pair");
    assert!(s.execute("assets.rename", &json!({"asset": first, "name": "  "})).is_err());
    assert!(s.execute("assets.rename", &json!({"asset": 999_999, "name": "X"})).is_err());
    // Assets that share a name (in any case) get -2, -3… instead of overwriting.
    let ids: Vec<u64> = assets(&mut s).iter().map(|a| a["id"].as_u64().unwrap()).collect();
    run(&mut s, "assets.rename", json!({"asset": ids[0], "name": "icon"}));
    let out = files(&run(&mut s, "assets.export", json!({"assets": [ids[0], ids[1]]})));
    assert_eq!(out.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), ["icon.png", "Icon-2.png"]);
}

#[test]
fn add_remove_and_rename_are_undo_steps() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    let steps = undo_len(&s);
    let id = run(&mut s, "assets.add", json!({"ids": [a]}))["assets"][0].as_u64().unwrap();
    run(&mut s, "assets.rename", json!({"asset": id, "name": "Logo"}));
    run(&mut s, "assets.remove", json!({"assets": [id]}));
    assert_eq!(undo_len(&s), steps + 3);
    assert!(assets(&mut s).is_empty());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(names(&mut s), ["Logo"]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(names(&mut s), ["Asset 1"]);
    run(&mut s, "edit.undo", json!({}));
    assert!(assets(&mut s).is_empty());
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(names(&mut s), ["Asset 1"]);
    assert!(s.execute("assets.remove", &json!({"assets": [12345]})).is_err());
    assert!(s.execute("assets.remove", &json!({})).is_err());
}

#[test]
fn assets_round_trip_through_the_native_format() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 10.0, 10.0, 20.0, 20.0), rect(&mut s, 50.0, 10.0, 30.0, 20.0));
    run(&mut s, "assets.add", json!({"ids": [a, b], "multiple": false, "name": "Both"}));
    let saved = s.doc().unwrap().doc.assets.clone();
    let b64 = run(&mut s, "document.serialize", json!({"format": "vectorcraft"}))["dataBase64"].as_str().unwrap().to_string();
    run(&mut s, "document.open", json!({"name": "assets.vectorcraft", "dataBase64": b64}));
    assert_eq!(s.doc().unwrap().doc.assets, saved);
    // A new asset never takes an id already used.
    let c = rect(&mut s, 90.0, 10.0, 20.0, 20.0);
    let id = run(&mut s, "assets.add", json!({"ids": [c]}))["assets"][0].as_u64().unwrap();
    assert!(saved.iter().all(|x| x.id != id));
    // Documents without assets don't write the key, and files without it load.
    let mut d = vectorcraft_doc::Document::new(10.0, 10.0);
    let text = serde_json::to_string(&d).unwrap();
    assert!(!text.contains("\"assets\""));
    d = serde_json::from_str(&text).unwrap();
    assert!(d.assets.is_empty());
}

#[test]
fn the_shared_settings_drive_the_export_and_arent_an_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    run(&mut s, "assets.add", json!({"ids": [a]}));
    let steps = undo_len(&s);
    let r = run(&mut s, "assets.settings.set", json!({"formats": [{"format": "png", "scale": "1x"}, {"format": "svg"}], "prefix": "ic_"}));
    assert_eq!(r["settings"]["prefix"], "ic_");
    assert_eq!(undo_len(&s), steps);
    assert!(s.doc().unwrap().is_dirty());
    // Export for Screens reads them too (they are its remembered settings).
    assert_eq!(run(&mut s, "document.exportSettings", json!({}))["settings"]["formats"][1]["format"], "svg");
    let out = files(&run(&mut s, "assets.export", json!({})));
    assert_eq!(out.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), ["ic_Asset-1.png", "ic_Asset-1.svg"]);
    // A preset replaces the rows (and turns on sub-folders); rows replace a preset.
    run(&mut s, "assets.settings.set", json!({"preset": "mobile"}));
    let settings = run(&mut s, "document.exportSettings", json!({}))["settings"].clone();
    assert!(settings.get("formats").is_none() && settings["preset"] == "mobile");
    let out = files(&run(&mut s, "assets.export", json!({})));
    assert_eq!(out.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), ["1x/ic_Asset-1.png", "2x/ic_Asset-1@2x.png", "3x/ic_Asset-1@3x.png"]);
    run(&mut s, "assets.settings.set", json!({"formats": [{"format": "jpg", "quality": 80}], "preset": ""}));
    let settings = run(&mut s, "document.exportSettings", json!({}))["settings"].clone();
    assert!(settings.get("preset").is_none() && settings["formats"][0]["format"] == "jpg");
    // Params given to one export win without changing the settings.
    let out = files(&run(&mut s, "assets.export", json!({"formats": [{"format": "webp"}], "prefix": null})));
    assert_eq!(out[0].0, "Asset-1.webp");
    assert_eq!(run(&mut s, "document.exportSettings", json!({}))["settings"]["formats"][0]["format"], "jpg");
    // Bad settings are refused and change nothing.
    for bad in [json!({"formats": [{"format": "dxf"}]}), json!({"preset": "tiny"}), json!({"folder": "/tmp"}), json!([])] {
        assert!(s.execute("assets.settings.set", &bad).is_err(), "{bad}");
    }
    assert_eq!(run(&mut s, "document.exportSettings", json!({}))["settings"]["formats"][0]["format"], "jpg");
}

#[test]
fn export_for_screens_writes_chosen_assets_and_keeps_its_rules() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 10.0, 10.0, 20.0, 20.0), rect(&mut s, 50.0, 10.0, 30.0, 20.0));
    let ids = run(&mut s, "assets.add", json!({"ids": [a, b]}))["assets"].clone();
    let r = run(&mut s, "document.exportForScreens", json!({"assets": [ids[1]], "formats": [{"format": "png", "scale": "60w"}]}));
    let out = files(&r);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].0, "Asset-2@60w.png");
    assert_eq!(image::load_from_memory(&out[0].1).unwrap().to_rgba8().dimensions(), (60, 40), "the width of the asset, not the artboard");
    // The choice of art isn't remembered; the formats are.
    let settings = run(&mut s, "document.exportSettings", json!({}))["settings"].clone();
    assert!(settings.get("assets").is_none() && settings["formats"][0]["scale"] == "60w");
    // A zip of every asset.
    let zip = run(&mut s, "document.exportForScreens", json!({"assets": ids, "zip": true}));
    assert_eq!(zip["files"], json!(["Asset-1.png", "Asset-2.png"]));
    for bad in [json!({"assets": [ids[0]], "fullDocument": true}), json!({"assets": [424242]}), json!({"assets": []}), json!({"assets": "all"})] {
        assert!(s.execute("document.exportForScreens", &bad).is_err(), "{bad}");
    }
    // No assets: nothing for assets.export to do.
    run(&mut s, "assets.remove", json!({"assets": ids}));
    assert!(s.execute("assets.export", &json!({})).is_err());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn assets_export_into_a_folder() {
    let dir = std::env::temp_dir().join(format!("vc-assets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let folder = dir.to_string_lossy().replace('\\', "/");
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    run(&mut s, "assets.add", json!({"ids": [a], "name": "Logo"}));
    let r =
        run(&mut s, "assets.export", json!({"folder": folder, "subfolders": true, "formats": [{"format": "png", "scale": "2x"}, {"format": "pdf"}]}));
    assert_eq!(r["files"], json!([format!("{folder}/2x/Logo@2x.png"), format!("{folder}/PDF/Logo.pdf")]));
    assert!(dir.join("2x/Logo@2x.png").is_file() && dir.join("PDF/Logo.pdf").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}
