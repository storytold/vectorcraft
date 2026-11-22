use serde_json::Value;
use vectorcraft_render::effects::{Mesh, SolidMesh};

use super::*;

/// A 10 × 10 square facing the viewer: two triangles, each with corners of its own.
fn square(color: [f32; 4]) -> SolidMesh {
    let [a, b, c, d] = [[-5.0, -5.0, 0.0], [5.0, -5.0, 0.0], [5.0, 5.0, 0.0], [-5.0, 5.0, 0.0]];
    let mesh = Mesh { positions: vec![a, b, c, a, c, d], normals: vec![[0.0, 0.0, 1.0]; 6], triangles: vec![[0, 1, 2], [3, 4, 5]] };
    SolidMesh::plain(mesh, color)
}

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 0.5];

fn text(files: &[(String, Vec<u8>)], name: &str) -> String {
    let (_, bytes) = files.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no {name}"));
    String::from_utf8(bytes.clone()).unwrap()
}

/// [`mesh_files`] without textures.
fn plain_files(stem: &str, format: &str, parts: &[SolidMesh]) -> Result<Vec<(String, Vec<u8>)>, String> {
    mesh_files(stem, format, parts, &[])
}

#[test]
fn obj_numbers_corners_across_solids_and_names_its_materials() {
    let files = plain_files("Box", "obj", &[square(RED), square(BLUE), square(RED)]).unwrap();
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
    let files = plain_files("Box", "gltf", &[square(RED), square(BLUE)]).unwrap();
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
    // Without graphics there are no textures.
    assert!(g.get("images").is_none() && g.get("textures").is_none() && prims[0]["attributes"].get("TEXCOORD_0").is_none());
}

#[test]
fn usda_is_y_up_in_points_with_a_material_for_each_color() {
    let files = plain_files("My Box", "usda", &[square(RED), square([0.5, 0.5, 0.5, 1.0])]).unwrap();
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
    assert!(!u.contains("primvars:st") && !u.contains("UsdUVTexture"));
}

#[test]
fn usdz_stores_the_layer_uncompressed_on_a_64_byte_boundary() {
    let files = plain_files("Box", "usdz", &[square(RED)]).unwrap();
    let (name, zip) = &files[0];
    assert_eq!(name, "Box.usdz");
    // The local header: 30 bytes, the name, then the extra field.
    let u16_at = |at: usize| u16::from_le_bytes([zip[at], zip[at + 1]]) as usize;
    assert_eq!(&zip[..4], &[0x50, 0x4b, 0x03, 0x04]);
    assert_eq!(u16_at(8), 0, "stored, not deflated");
    let start = 30 + u16_at(26) + u16_at(28);
    assert_eq!(start % 64, 0);
    let layer = plain_files("Box", "usda", &[square(RED)]).unwrap().remove(0).1;
    assert_eq!(&zip[start..start + layer.len()], layer.as_slice());
    let archive = super::super::zip::Archive::open(zip, 8).unwrap();
    assert_eq!(archive.read("Box.usda", usize::MAX).unwrap(), Some(layer));
}

#[test]
fn an_obj_names_its_mtl_file_without_spaces_or_comment_marks() {
    let files = plain_files("Box #1", "obj", &[square(RED)]).unwrap();
    assert_eq!(files.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["Box__1.obj", "Box__1.mtl"]);
    assert!(text(&files, "Box__1.obj").starts_with("mtllib Box__1.mtl\n"));
    assert_eq!(mesh_file_names("Box #1", "obj", &[]).unwrap(), ["Box__1.obj", "Box__1.mtl"]);
    assert_eq!(mesh_file_names("Box #1", "usda", &[]).unwrap(), ["Box #1.usda"]);
}

#[test]
fn obj_and_usda_keep_coordinates_a_32_bit_float_would_merge() {
    let mut part = square(RED);
    part.mesh.positions[0] = [20_000_000.0, 0.0, 0.0];
    part.mesh.positions[1] = [20_000_001.0, 0.0, 0.0];
    let obj = text(&plain_files("Box", "obj", std::slice::from_ref(&part)).unwrap(), "Box.obj");
    assert!(obj.contains("v 20000000 0 0\nv 20000001 0 0\n"), "{obj}");
    let usda = text(&plain_files("Box", "usda", &[part]).unwrap(), "Box.usda");
    assert!(usda.contains("(20000001, 0, 0)"));
}

#[test]
fn an_unknown_format_is_refused() {
    assert!(plain_files("Box", "fbx", &[square(RED)]).is_err());
    assert!(mesh_file_names("Box", "", &[]).is_err());
    assert_eq!(mesh_file_names("Box", "obj", &[]).unwrap(), ["Box.obj", "Box.mtl"]);
}

#[test]
fn prim_names_are_usd_identifiers() {
    assert_eq!(prim_name("Logo 2"), "Logo_2");
    assert_eq!(prim_name("3D"), "_3D");
    assert_eq!(prim_name(""), "_");
    assert_eq!(prim_name("é/x"), "__x");
}

/// A red square, then a blue one whose corners show picture `png` (u to the right, v down).
fn textured(png: &[u8]) -> (Vec<SolidMesh>, Vec<Option<Texture>>) {
    let uv = vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    let texture = Texture { png: Arc::new(png.to_vec()), opacity: None, uv };
    (vec![square(RED), square(BLUE)], vec![None, Some(texture)])
}

#[test]
fn each_format_carries_a_textured_part_with_its_coordinates_and_picture() {
    let png = b"\x89PNG picture".to_vec();
    let (parts, pictures) = textured(&png);
    // OBJ: vt runs up from the picture's bottom, the textured faces name it, and map_Kd the PNG
    // written beside it, with its name in one word.
    let files = mesh_files("My Box", "obj", &parts, &pictures).unwrap();
    assert_eq!(files.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["My_Box.obj", "My_Box.mtl", "My_Box-texture-1.png"]);
    assert_eq!(files[2].1, png);
    let obj = text(&files, "My_Box.obj");
    assert_eq!(obj.lines().filter(|l| l.starts_with("vt ")).collect::<Vec<_>>(), ["vt 0 0", "vt 1 0", "vt 1 1", "vt 0 0", "vt 1 1", "vt 0 1"]);
    assert_eq!(obj.lines().rfind(|l| l.starts_with("f ")), Some("f 10/4/10 11/5/11 12/6/12"));
    assert!(obj.contains("f 1//1 2//2 3//3"), "the plain part keeps no texture coordinates");
    let mtl = text(&files, "My_Box.mtl");
    assert!(mtl.contains("newmtl Material_2\nKd 1 1 1\nd 1\nillum 1\nmap_Kd -clamp on My_Box-texture-1.png\n"), "{mtl}");
    assert_eq!(mtl.matches("map_Kd").count(), 1);
    // glTF: TEXCOORD_0 down from the top, and the picture embedded as a data URI.
    let g: Value = serde_json::from_str(&text(&mesh_files("Box", "gltf", &parts, &pictures).unwrap(), "Box.gltf")).unwrap();
    let prims = g["meshes"][0]["primitives"].as_array().unwrap();
    assert!(prims[0]["attributes"].get("TEXCOORD_0").is_none());
    let uv = &g["accessors"][prims[1]["attributes"]["TEXCOORD_0"].as_u64().unwrap() as usize];
    assert_eq!((uv["type"].as_str(), uv["count"].as_u64(), uv["componentType"].as_u64()), (Some("VEC2"), Some(6), Some(5126)));
    let view = &g["bufferViews"][uv["bufferView"].as_u64().unwrap() as usize];
    let data = vectorcraft_format::base64_decode(g["buffers"][0]["uri"].as_str().unwrap().split(',').nth(1).unwrap()).unwrap();
    let at = view["byteOffset"].as_u64().unwrap() as usize;
    let first: Vec<f32> = (0..2).map(|k| f32::from_le_bytes(data[at + 4 * k..at + 4 * k + 4].try_into().unwrap())).collect();
    assert_eq!(first, [0.0, 1.0]);
    let material = &g["materials"][prims[1]["material"].as_u64().unwrap() as usize];
    assert_eq!(material["pbrMetallicRoughness"]["baseColorTexture"]["index"], 0);
    assert_eq!(material["pbrMetallicRoughness"]["baseColorFactor"], serde_json::json!([1.0, 1.0, 1.0, 1.0]));
    assert_eq!(g["textures"][0]["source"], 0);
    assert_eq!(g["samplers"][0]["wrapS"], 33071);
    let image = g["images"][0]["uri"].as_str().unwrap().strip_prefix("data:image/png;base64,").unwrap();
    assert_eq!(vectorcraft_format::base64_decode(image).unwrap(), png);
    // USDA: st up from the bottom, read by a primvar reader into a UV texture of the PNG beside it.
    let files = mesh_files("Box", "usda", &parts, &pictures).unwrap();
    assert_eq!(files.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["Box.usda", "Box-texture-1.png"]);
    let u = text(&files, "Box.usda");
    assert_eq!(u.matches("primvars:st").count(), 1);
    assert!(u.contains("texCoord2f[] primvars:st = [(0, 0), (1, 0), (1, 1), (0, 0), (1, 1), (0, 1)]"), "{u}");
    assert!(u.contains("color3f inputs:diffuseColor.connect = </Box/Materials/Material_2/Texture.outputs:rgb>"), "{u}");
    assert!(u.contains("asset inputs:file = @Box-texture-1.png@") && u.contains("uniform token info:id = \"UsdUVTexture\""));
    assert!(u.contains("float2 inputs:st.connect = </Box/Materials/Material_2/Coordinates.outputs:result>"));
    assert!(u.contains("string inputs:varname = \"st\""));
    assert_eq!(u.matches('{').count(), u.matches('}').count());
    // USDZ: the layer first, then the picture, each on a 64-byte boundary.
    let zip = mesh_files("Box", "usdz", &parts, &pictures).unwrap().remove(0).1;
    let archive = super::super::zip::Archive::open(&zip, 8).unwrap();
    assert_eq!(archive.read("Box.usda", usize::MAX).unwrap().map(String::from_utf8), Some(Ok(u)));
    assert_eq!(archive.read("Box-texture-1.png", usize::MAX).unwrap(), Some(png.clone()));
    let u16_at = |at: usize| u16::from_le_bytes([zip[at], zip[at + 1]]) as usize;
    let u32_at = |at: usize| u32::from_le_bytes([zip[at], zip[at + 1], zip[at + 2], zip[at + 3]]) as usize;
    let first = 30 + u16_at(26) + u16_at(28);
    let second = first + u32_at(18);
    assert_eq!(&zip[second..second + 4], &[0x50, 0x4b, 0x03, 0x04], "the picture comes second");
    let data = second + 30 + u16_at(second + 26) + u16_at(second + 28);
    assert_eq!((first % 64, data % 64), (0, 0));
    assert_eq!(&zip[data..data + png.len()], png.as_slice());
}

#[test]
fn a_textured_face_that_is_not_opaque_takes_its_opacity_from_the_picture() {
    let (parts, mut pictures) = textured(b"picture");
    if let Some(t) = pictures[1].as_mut() {
        t.opacity = Some(Arc::new(b"gray".to_vec()));
    }
    // OBJ: the opacity map, written after its texture, is the MTL's map_d.
    let files = mesh_files("My Box", "obj", &parts, &pictures).unwrap();
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["My_Box.obj", "My_Box.mtl", "My_Box-texture-1.png", "My_Box-texture-1-opacity.png"]);
    assert_eq!(files[3].1, b"gray");
    assert_eq!(mesh_file_names("My Box", "obj", &image_maps(&pictures)).unwrap(), names);
    let mtl = text(&files, "My_Box.mtl");
    assert!(mtl.contains("d 1\nillum 1\nmap_Kd -clamp on My_Box-texture-1.png\nmap_d -clamp on My_Box-texture-1-opacity.png\n"), "{mtl}");
    let g: Value = serde_json::from_str(&text(&mesh_files("Box", "gltf", &parts, &pictures).unwrap(), "Box.gltf")).unwrap();
    let material = &g["materials"][1];
    assert_eq!((material["pbrMetallicRoughness"]["baseColorFactor"][3].as_f64(), material["alphaMode"].as_str()), (Some(1.0), Some("BLEND")));
    let usda = mesh_files("Box", "usda", &parts, &pictures).unwrap();
    assert_eq!(usda.len(), 2, "USD reads the opacity from the texture's alpha");
    let u = text(&usda, "Box.usda");
    assert!(u.contains("float inputs:opacity.connect = </Box/Materials/Material_2/Texture.outputs:a>") && u.contains("float outputs:a"), "{u}");
    // An opaque textured face reads no opacity from its picture.
    let (_, plain) = textured(b"picture");
    let opaque = vec![square(RED), square([0.0, 0.0, 1.0, 1.0])];
    let u = text(&mesh_files("Box", "usda", &opaque, &plain).unwrap(), "Box.usda");
    assert!(!u.contains("outputs:a") && u.contains("float inputs:opacity = 1"), "{u}");
    let mtl = text(&mesh_files("Box", "obj", &opaque, &plain).unwrap(), "Box.mtl");
    assert!(!mtl.contains("map_d") && mtl.contains("Kd 1 1 1\nd 1\n"), "{mtl}");
}

#[test]
fn parts_sharing_a_picture_write_it_once() {
    let (mut parts, mut pictures) = textured(b"one");
    parts.push(square(BLUE));
    pictures.push(pictures[1].clone());
    assert_eq!(image_maps(&pictures), [false]);
    let files = mesh_files("Box", "obj", &parts, &pictures).unwrap();
    assert_eq!(files.len(), 3);
    assert_eq!(text(&files, "Box.mtl").matches("newmtl").count(), 2, "the blue parts share the textured material");
    assert_eq!(mesh_file_names("Box", "usda", &[false, true]).unwrap(), ["Box.usda", "Box-texture-1.png", "Box-texture-2.png"]);
    assert_eq!(mesh_file_names("A @b", "gltf", &[false, false]).unwrap(), ["A @b.gltf"]);
    assert_eq!(texture_name("A @b#c", 0), "A__b_c-texture-1.png");
}
