use serde_json::Value;
use vectorcraft_render::effects::{Mesh, SolidMesh};

use super::*;

/// A 10 × 10 square facing the viewer: two triangles, each with corners of its own.
fn square(color: [f32; 4]) -> SolidMesh {
    let [a, b, c, d] = [[-5.0, -5.0, 0.0], [5.0, -5.0, 0.0], [5.0, 5.0, 0.0], [-5.0, 5.0, 0.0]];
    let mesh = Mesh { positions: vec![a, b, c, a, c, d], normals: vec![[0.0, 0.0, 1.0]; 6], triangles: vec![[0, 1, 2], [3, 4, 5]] };
    SolidMesh { mesh, color }
}

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 0.5];

fn text(files: &[(String, Vec<u8>)], name: &str) -> String {
    let (_, bytes) = files.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no {name}"));
    String::from_utf8(bytes.clone()).unwrap()
}

#[test]
fn obj_numbers_corners_across_solids_and_names_its_materials() {
    let files = mesh_files("Box", "obj", &[square(RED), square(BLUE), square(RED)]).unwrap();
    assert_eq!(files.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["Box.obj", "Box.mtl"]);
    let obj = text(&files, "Box.obj");
    assert!(obj.starts_with("mtllib Box.mtl\no Box\n"));
    assert_eq!(obj.lines().filter(|l| l.starts_with("v ")).count(), 18);
    assert_eq!(obj.lines().filter(|l| l.starts_with("vn ")).count(), 18);
    // The third solid's last triangle uses corners 13 to 18; red solids share Material_1.
    assert_eq!(obj.lines().rfind(|l| l.starts_with("f ")), Some("f 16//16 17//17 18//18"));
    assert_eq!(obj.lines().filter(|l| l.starts_with("usemtl ")).collect::<Vec<_>>(), ["usemtl Material_1", "usemtl Material_2", "usemtl Material_1"]);
    let mtl = text(&files, "Box.mtl");
    assert_eq!(mtl.matches("newmtl").count(), 2);
    assert!(mtl.contains("newmtl Material_2\nKd 0 0 1\nd 0.5\n"), "{mtl}");
}

#[test]
fn gltf_holds_its_buffer_and_every_accessor_fits_it() {
    let files = mesh_files("Box", "gltf", &[square(RED), square(BLUE)]).unwrap();
    let g: Value = serde_json::from_str(&text(&files, "Box.gltf")).unwrap();
    assert_eq!(g["asset"]["version"], "2.0");
    let uri = g["buffers"][0]["uri"].as_str().unwrap();
    let data = vectorcraft_format::base64_decode(uri.strip_prefix("data:application/octet-stream;base64,").unwrap()).unwrap();
    assert_eq!(g["buffers"][0]["byteLength"].as_u64(), Some(data.len() as u64));
    for v in g["bufferViews"].as_array().unwrap() {
        assert!(v["byteOffset"].as_u64().unwrap() + v["byteLength"].as_u64().unwrap() <= data.len() as u64);
        assert_eq!(v["byteOffset"].as_u64().unwrap() % 4, 0);
    }
    let prims = g["meshes"][0]["primitives"].as_array().unwrap();
    assert_eq!(prims.len(), 2);
    assert_eq!((prims[0]["material"].as_u64(), prims[1]["material"].as_u64()), (Some(0), Some(1)));
    // Positions are meters: the 10 pt square is 10/72 inch across.
    let pos = &g["accessors"][prims[0]["attributes"]["POSITION"].as_u64().unwrap() as usize];
    let width = pos["max"][0].as_f64().unwrap() - pos["min"][0].as_f64().unwrap();
    assert!((width - 10.0 * 0.0254 / 72.0).abs() < 1e-7, "{width}");
    let index = &g["accessors"][prims[0]["indices"].as_u64().unwrap() as usize];
    assert_eq!((index["componentType"].as_u64(), index["count"].as_u64()), (Some(5125), Some(6)));
    // Translucent paint blends; colors are linear.
    assert_eq!(g["materials"][1]["alphaMode"], "BLEND");
    assert!(g["materials"][0].get("alphaMode").is_none());
    assert_eq!(g["materials"][0]["doubleSided"], true);
}

#[test]
fn usda_is_y_up_in_points_with_a_material_for_each_color() {
    let files = mesh_files("My Box", "usda", &[square(RED), square([0.5, 0.5, 0.5, 1.0])]).unwrap();
    let u = text(&files, "My Box.usda");
    assert!(u.starts_with("#usda 1.0\n"));
    assert!(u.contains("defaultPrim = \"My_Box\"") && u.contains("upAxis = \"Y\"") && u.contains("def Xform \"My_Box\""), "{u}");
    assert!(u.contains(&format!("metersPerUnit = {}", 0.0254 / 72.0)));
    assert_eq!(u.matches("def Mesh").count(), 2);
    assert_eq!(u.matches("def Material").count(), 2);
    assert!(u.contains("int[] faceVertexCounts = [3, 3]"));
    assert!(u.contains("rel material:binding = </My_Box/Materials/Material_2>"));
    // sRGB 0.5 is about 0.214 linear.
    assert!(u.contains("color3f inputs:diffuseColor = (0.21404"), "{u}");
    assert_eq!(u.matches('{').count(), u.matches('}').count());
}

#[test]
fn usdz_stores_the_layer_uncompressed_on_a_64_byte_boundary() {
    let files = mesh_files("Box", "usdz", &[square(RED)]).unwrap();
    let (name, zip) = &files[0];
    assert_eq!(name, "Box.usdz");
    // The local header: 30 bytes, the name, then the extra field.
    let u16_at = |at: usize| u16::from_le_bytes([zip[at], zip[at + 1]]) as usize;
    assert_eq!(&zip[..4], &[0x50, 0x4b, 0x03, 0x04]);
    assert_eq!(u16_at(8), 0, "stored, not deflated");
    let start = 30 + u16_at(26) + u16_at(28);
    assert_eq!(start % 64, 0);
    let layer = mesh_files("Box", "usda", &[square(RED)]).unwrap().remove(0).1;
    assert_eq!(&zip[start..start + layer.len()], layer.as_slice());
    let archive = super::super::zip::Archive::open(zip, 8).unwrap();
    assert_eq!(archive.read("Box.usda", usize::MAX).unwrap(), Some(layer));
}

#[test]
fn an_obj_names_its_mtl_file_without_spaces_or_comment_marks() {
    let files = mesh_files("Box #1", "obj", &[square(RED)]).unwrap();
    assert_eq!(files.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["Box__1.obj", "Box__1.mtl"]);
    assert!(text(&files, "Box__1.obj").starts_with("mtllib Box__1.mtl\n"));
    assert_eq!(mesh_file_names("Box #1", "obj").unwrap(), ["Box__1.obj", "Box__1.mtl"]);
    assert_eq!(mesh_file_names("Box #1", "usda").unwrap(), ["Box #1.usda"]);
}

#[test]
fn obj_and_usda_keep_coordinates_a_32_bit_float_would_merge() {
    let mut part = square(RED);
    part.mesh.positions[0] = [20_000_000.0, 0.0, 0.0];
    part.mesh.positions[1] = [20_000_001.0, 0.0, 0.0];
    let obj = text(&mesh_files("Box", "obj", std::slice::from_ref(&part)).unwrap(), "Box.obj");
    assert!(obj.contains("v 20000000 0 0\nv 20000001 0 0\n"), "{obj}");
    let usda = text(&mesh_files("Box", "usda", &[part]).unwrap(), "Box.usda");
    assert!(usda.contains("(20000001, 0, 0)"));
}

#[test]
fn an_unknown_format_is_refused() {
    assert!(mesh_files("Box", "fbx", &[square(RED)]).is_err());
    assert!(mesh_file_names("Box", "").is_err());
    assert_eq!(mesh_file_names("Box", "obj").unwrap(), ["Box.obj", "Box.mtl"]);
}

#[test]
fn prim_names_are_usd_identifiers() {
    assert_eq!(prim_name("Logo 2"), "Logo_2");
    assert_eq!(prim_name("3D"), "_3D");
    assert_eq!(prim_name(""), "_");
    assert_eq!(prim_name("é/x"), "__x");
}
