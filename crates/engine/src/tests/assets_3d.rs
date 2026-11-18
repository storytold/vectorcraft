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

/// Without `ids`, the selected objects without a 3D effect, a group whose 3D effect is on its fill
/// (it draws nothing to export) and a 3D object inside another selected one are left out; a
/// selection with none left is refused.
#[test]
fn export_3d_object_leaves_flat_objects_in_the_selection_out() {
    let mut s = session();
    let plain = run(&mut s, "shape.rectangle", json!({"x": 10.0, "y": 100.0, "width": 40.0, "height": 30.0}))["id"].as_u64().unwrap();
    let filled = run(&mut s, "shape.rectangle", json!({"x": 10.0, "y": 200.0, "width": 40.0, "height": 30.0}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [filled]}));
    let group = run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [group]}));
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude"}));
    // `effect.apply` puts a group's 3D effect on the group itself; a file can hold one on its fill.
    s.edit("fill", |d, _| {
        let n = d.node_mut(vectorcraft_doc::NodeId(group)).unwrap();
        let fx = n.appearance.effects.remove(0);
        n.appearance.items[0].effects_mut().push(fx);
        Ok(())
    })
    .unwrap();
    run(&mut s, "select.set", json!({"ids": [plain, group]}));
    let before = undo_len(&s);
    assert!(s.execute("assets.add3D", &json!({})).is_err(), "no object with a 3D effect to export");
    assert_eq!(undo_len(&s), before);
    let inner = extruded(&mut s, 100.0, "Inner");
    let other = run(&mut s, "shape.rectangle", json!({"x": 200.0, "y": 10.0, "width": 40.0, "height": 30.0}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [inner, other]}));
    let outer = run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [outer]}));
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude"}));
    run(&mut s, "select.set", json!({"ids": [plain, group, inner, outer]}));
    let r = run(&mut s, "assets.add3D", json!({}));
    assert_eq!(r["added"], 1, "{r}");
    let nodes: Vec<Value> = run(&mut s, "assets.list", json!({}))["assets"].as_array().unwrap().iter().map(|a| a["nodes"].clone()).collect();
    assert_eq!(nodes, [json!([outer])]);
    assert!(s.execute("assets.add3D", &json!({"ids": [plain, outer]})).is_err(), "ids are still refused");
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

/// A blue 100 × 100 square extruded straight on, with a red 10 × 20 graphic "Mark" mapped onto its
/// front at its center and onto its back 30 pt to the right (as the back is seen from behind),
/// collected as a 3D asset in `format`.
fn mapped(format: &str) -> (Session, u64) {
    let mut s = session();
    let mark = run(&mut s, "path.create", json!({"d": "M300 300L310 300L310 320L300 320Z"}))["id"].clone();
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "threeD.addGraphic", json!({"name": "Mark", "ids": [mark]}));
    let id = run(&mut s, "path.create", json!({"d": "M100 100L200 100L200 200L100 200Z"}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setFill", json!({"color": "#0000ff"}));
    run(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "params": {"rotationX": 0.0, "rotationY": 0.0, "rotationZ": 0.0, "depth": 20.0}}));
    run(&mut s, "threeD.mapGraphic", json!({"symbol": "Mark", "ids": [id]}));
    run(&mut s, "threeD.mapGraphic", json!({"symbol": "Mark", "face": "back", "x": 30.0, "ids": [id]}));
    run(&mut s, "layer.setProps", json!({"id": id, "name": "Box"}));
    let asset = run(&mut s, "assets.add3D", json!({"ids": [id], "format": format}))["assets"][0].as_u64().unwrap();
    (s, asset)
}

/// The color of pixel `(x, y)` of PNG `png`.
fn pixel(png: &[u8], x: u32, y: u32) -> [u8; 4] {
    image::load_from_memory(png).unwrap().to_rgba8().get_pixel(x, y).0
}

fn is_red(p: [u8; 4]) -> bool {
    p[0] > 200 && p[1] < 50 && p[2] < 50 && p[3] == 255
}

fn is_blue(p: [u8; 4]) -> bool {
    p[0] < 50 && p[1] < 50 && p[2] > 200 && p[3] == 255
}

#[test]
fn export_3d_paints_mapped_graphics_on_their_caps_in_every_format() {
    for (format, names) in [
        ("usda", vec!["Box.usda", "Box-texture-1.png", "Box-texture-2.png"]),
        ("usdz", vec!["Box.usdz"]),
        ("gltf", vec!["Box.gltf"]),
        ("obj", vec!["Box.obj", "Box.mtl", "Box-texture-1.png", "Box-texture-2.png"]),
    ] {
        let (mut s, _) = mapped(format);
        let out = files(&run(&mut s, "assets.export3D", json!({"textureSize": 128})));
        assert_eq!(out.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), names, "{format}");
    }
    // The front's picture: the cap's 100 pt across 128 pixels, blue with the red graphic at the
    // middle; the back's: the graphic 30 pt to the right as seen from behind, which is to the left
    // in the art's plane.
    let (mut s, _) = mapped("usda");
    let out = files(&run(&mut s, "assets.export3D", json!({"textureSize": 128})));
    let (front, back) = (&out[1].1, &out[2].1);
    assert_eq!(image::load_from_memory(front).unwrap().to_rgba8().dimensions(), (128, 128));
    assert!(is_red(pixel(front, 64, 64)) && is_blue(pixel(front, 2, 2)) && is_blue(pixel(front, 64, 10)));
    let at = |pt: f64| (pt * 1.28) as u32;
    assert!(is_red(pixel(back, at(20.0), 64)) && is_blue(pixel(back, 64, 64)), "{:?}", pixel(back, 64, 64));
    // The USDA's front cap shows the front picture through st, up from the picture's bottom.
    let usda = String::from_utf8(out[0].1.clone()).unwrap();
    assert_eq!(usda.matches("primvars:st").count(), 2);
    assert_eq!(usda.matches("def Mesh").count(), 3, "the walls, the front cap and the back cap");
    // glTF holds both pictures, and a cap's coordinates stay inside its picture.
    let (mut s, _) = mapped("gltf");
    let out = files(&run(&mut s, "assets.export3D", json!({})));
    let g: Value = serde_json::from_slice(&out[0].1).unwrap();
    assert_eq!(g["images"].as_array().map(Vec::len), Some(2));
    let png = vectorcraft_format::base64_decode(g["images"][0]["uri"].as_str().unwrap().split(',').nth(1).unwrap()).unwrap();
    assert_eq!(image::load_from_memory(&png).unwrap().to_rgba8().dimensions(), (1024, 1024), "1024 pixels by default");
}

#[test]
fn a_texture_takes_the_opacity_of_the_fill_and_of_the_object_as_the_canvas_draws_them() {
    let half = |a: u8| (120..=135).contains(&a);
    // A half-opaque fill: the face and the graphic over it are each drawn at half opacity, so the
    // graphic covers three quarters.
    let (mut s, asset) = mapped("usda");
    let doc = &s.active().unwrap().doc;
    let id = doc.asset(asset).unwrap().nodes[0];
    let fill = doc.node(id).unwrap().appearance.items.iter().position(|i| matches!(i, vectorcraft_doc::AppearanceItem::Fill(_))).unwrap();
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "transparency.set", json!({"item": fill, "opacity": 50}));
    let out = files(&run(&mut s, "assets.export3D", json!({"textureSize": 128})));
    let (on, around) = (pixel(&out[1].1, 64, 64), pixel(&out[1].1, 2, 2));
    assert!(on[0] > on[2] && (185..=197).contains(&on[3]), "{on:?}");
    assert!(around[2] > 200 && half(around[3]), "{around:?}");
    let usda = String::from_utf8(out[0].1.clone()).unwrap();
    assert!(usda.contains("float inputs:opacity.connect"), "the texture's alpha is the opacity");
    // An OBJ's opacity map has the alpha as gray levels.
    run(&mut s, "assets.set3DFormat", json!({"assets": [asset], "format": "obj"}));
    let out = files(&run(&mut s, "assets.export3D", json!({"textureSize": 128})));
    assert_eq!(out[3].0, "Box-texture-1-opacity.png");
    let gray = image::load_from_memory(&out[3].1).unwrap().to_luma8();
    let (g_on, g_around) = (gray.get_pixel(64, 64).0[0], gray.get_pixel(2, 2).0[0]);
    assert!((185..=197).contains(&g_on) && half(g_around), "{g_on} {g_around}");
    // A half-opaque object: the face and the graphic are drawn opaque, and the whole picture at
    // half opacity.
    let (mut s, asset) = mapped("usda");
    let id = s.active().unwrap().doc.asset(asset).unwrap().nodes[0];
    run(&mut s, "transparency.set", json!({"ids": [id.0], "opacity": 50}));
    let out = files(&run(&mut s, "assets.export3D", json!({"textureSize": 128})));
    let (on, around) = (pixel(&out[1].1, 64, 64), pixel(&out[1].1, 2, 2));
    assert!(on[0] > 200 && on[2] < 50 && half(on[3]), "{on:?}");
    assert!(around[2] > 200 && around[0] < 50 && half(around[3]), "{around:?}");
    // A gradient whose stops are clear: the face is clear, and the graphic keeps its opacity.
    let (mut s, asset) = mapped("usda");
    let id = s.active().unwrap().doc.asset(asset).unwrap().nodes[0];
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let clear = json!({"stops": [{"offset": 0, "color": "#0000ff", "opacity": 0}, {"offset": 1, "color": "#0000ff", "opacity": 0}]});
    run(&mut s, "paint.setFill", json!({"gradient": clear}));
    let out = files(&run(&mut s, "assets.export3D", json!({"textureSize": 128})));
    let (on, around) = (pixel(&out[1].1, 64, 64), pixel(&out[1].1, 2, 2));
    assert!(is_red(on), "{on:?}");
    assert_eq!(around[3], 0, "{around:?}");
}

#[test]
fn export_3d_keeps_flat_colors_without_graphics_and_refuses_bad_texture_sizes() {
    let (mut s, asset) = mapped("obj");
    for bad in [json!(64), json!(5000), json!(512.5), json!("512"), json!(-1)] {
        assert!(s.execute("assets.export3D", &json!({"textureSize": bad})).is_err(), "{bad}");
    }
    // Without its graphics the box is one plain mesh, as before.
    let id = s.doc().unwrap().doc.asset(asset).unwrap().nodes[0].0;
    run(&mut s, "select.set", json!({"ids": [id]}));
    run(&mut s, "effect.setParams", json!({"index": 0, "params": {"graphics": []}}));
    let out = files(&run(&mut s, "assets.export3D", json!({"textureSize": 128})));
    assert_eq!(out.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["Box.obj", "Box.mtl"]);
    let obj = String::from_utf8(out[0].1.clone()).unwrap();
    assert!(!obj.contains("vt ") && obj.lines().filter(|l| l.starts_with("g ")).count() == 1);
}

#[test]
fn texture_pixels_are_counted_across_one_export() {
    let (s, asset) = mapped("gltf");
    let d = &s.doc().unwrap().doc;
    let node = d.node(d.asset(asset).unwrap().nodes[0]).unwrap();
    let parts = vectorcraft_render::effects::three_d_meshes_in(Some(d), node).unwrap();
    let mut pixels = 0;
    let made = crate::cmd::fileio::textures(d, &parts, 128, &mut pixels).unwrap();
    assert_eq!((made.iter().flatten().count(), pixels), (2, 2 * 128 * 128));
    let mut pixels = crate::cmd::fileio::MAX_TEXTURE_PIXELS - 128 * 128;
    let e = crate::cmd::fileio::textures(d, &parts, 128, &mut pixels).unwrap_err();
    assert!(e.contains("textureSize"), "{e}");
}

#[test]
fn only_graphics_that_draw_count_as_textures() {
    let parts_of = |s: &Session, asset: u64| {
        let d = &s.doc().unwrap().doc;
        let node = d.node(d.asset(asset).unwrap().nodes[0]).unwrap();
        vectorcraft_render::effects::three_d_meshes_in(Some(d), node).unwrap()
    };
    let (mut s, asset) = mapped("obj");
    assert!(crate::cmd::fileio::draws_textures(&s.doc().unwrap().doc, &parts_of(&s, asset)));
    // With its symbol gone the graphic draws nothing, so the export writes no texture.
    run(&mut s, "symbol.delete", json!({"name": "Mark"}));
    assert!(!crate::cmd::fileio::draws_textures(&s.doc().unwrap().doc, &parts_of(&s, asset)));
    let out = files(&run(&mut s, "assets.export3D", json!({"textureSize": 128})));
    assert_eq!(out.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["Box.obj", "Box.mtl"]);
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
