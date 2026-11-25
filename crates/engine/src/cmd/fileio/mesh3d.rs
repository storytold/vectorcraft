//! 3D objects as mesh files, for a 3D asset of Window › Asset Export (`assets.export3D`): OBJ with
//! its MTL materials, glTF 2.0 (one `.gltf` file, its buffer embedded as a data URI), USDA and
//! USDZ (the USDA layer in an uncompressed ZIP, its data on a 64-byte boundary).
//!
//! Each solid of the object ([`vectorcraft_render::effects::three_d_meshes`]) is its own mesh, with a
//! material in the color it is painted (solids of one color share it). Positions are y up and
//! about the point the object turns about: in points in OBJ and USD (whose layer says how many
//! meters a unit is), in meters in glTF, as its specification asks. glTF and USD colors are linear,
//! as their specifications ask; MTL colors are the display colors. Materials are two-sided, since
//! an open solid or a turned plane is seen from both sides. Every corner carries its normal and
//! texture coordinates (`vt`, `TEXCOORD_0`, `primvars:st`); a mesh whose lists do not match, or
//! that holds a value that is not a number, is refused before anything is written.
//!
//! The graphics mapped onto a face of a solid ([`textures`]) are a PNG picture of that face: its
//! color with the graphics drawn over it, flat and unshaded, which the face's own mesh shows through
//! texture coordinates and a material of its own. glTF embeds each PNG as a data URI, USDZ stores it
//! after the layer, and OBJ and USDA name PNG files written beside them (`<name>-texture-1.png`…).
//!
//! Formats: glTF 2.0 <https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html>, USD
//! <https://openusd.org/release/index.html> (USDZ: <https://openusd.org/release/spec_usdz.html>;
//! textures: <https://openusd.org/release/spec_usdpreviewsurface.html>), OBJ and MTL
//! <https://www.loc.gov/preservation/digital/formats/fdd/fdd000508.shtml>.

use std::fmt::Write as _;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId};
use vectorcraft_geom::{Affine, Rect};
use vectorcraft_render::effects::{MeshGraphics, SolidMesh};

/// The formats a 3D asset exports to, as `assets.set3DFormat` names them: USDA first, the default.
pub const FORMATS_3D: [&str; 4] = ["usda", "usdz", "gltf", "obj"];
/// The format a new 3D asset exports to.
pub const DEFAULT_FORMAT_3D: &str = "usda";
/// The meters in a point (1/72 inch).
const METERS_PER_POINT: f64 = 0.0254 / 72.0;
/// The pixels along the longer side of a face's texture, unless `assets.export3D` is given another
/// size, and the sizes it takes.
pub(crate) const TEXTURE_SIZE: u32 = 1024;
pub(crate) const TEXTURE_SIZES: (u32, u32) = (128, 4096);
/// The most texture pixels one export draws, all its assets together.
pub(crate) const MAX_TEXTURE_PIXELS: u64 = 4 * 4096 * 4096;

/// The label of `format` for menus: `USDA`, `USDZ`, `glTF`, `OBJ`.
pub fn format_3d_label(format: &str) -> &'static str {
    match format {
        "usdz" => "USDZ",
        "gltf" => "glTF",
        "obj" => "OBJ",
        _ => "USDA",
    }
}

/// `stem` as an OBJ file name: whitespace and `#` become `_`, so the `mtllib` line names the MTL
/// file as one word.
fn obj_stem(stem: &str) -> String {
    stem.chars().map(|c| if c.is_whitespace() || c == '#' { '_' } else { c }).collect()
}

/// The name of texture `k` (from 0) of the object named `stem`: whitespace, `#` and `@` become `_`,
/// so an MTL `map_Kd` line and a USD asset path name it as one word.
fn texture_name(stem: &str, k: usize) -> String {
    format!("{}-texture-{}.png", texture_stem(stem), k + 1)
}

/// The name of the opacity map of texture `k` (from 0) of the object named `stem`, as
/// [`texture_name`] names the texture.
fn opacity_name(stem: &str, k: usize) -> String {
    format!("{}-texture-{}-opacity.png", texture_stem(stem), k + 1)
}

/// `stem` with whitespace, `#` and `@` turned into `_`, for the names of its textures.
fn texture_stem(stem: &str) -> String {
    stem.chars().map(|c| if c.is_whitespace() || c == '#' || c == '@' { '_' } else { c }).collect()
}

/// The names of the files [`mesh_files`] writes for `stem` in `format` with the textures `images`
/// ([`image_maps`]: for each, whether it has an opacity map): OBJ and USDA name a PNG file for each
/// texture after them, and OBJ also one for each opacity map, after its texture; glTF and USDZ hold
/// theirs.
pub(crate) fn mesh_file_names(stem: &str, format: &str, images: &[bool]) -> Result<Vec<String>, String> {
    let beside = |mut names: Vec<String>, stem: &str, maps: bool| {
        for (k, map) in images.iter().enumerate() {
            names.push(texture_name(stem, k));
            if maps && *map {
                names.push(opacity_name(stem, k));
            }
        }
        names
    };
    match format {
        "obj" => {
            let stem = obj_stem(stem);
            Ok(beside(vec![format!("{stem}.obj"), format!("{stem}.mtl")], &stem, true))
        }
        "usda" => Ok(beside(vec![format!("{stem}.usda")], stem, false)),
        "gltf" | "usdz" => Ok(vec![format!("{stem}.{format}")]),
        _ => {
            let key = "format";
            Err(format!("`{key}` must be one of {}", FORMATS_3D.join(", ")))
        }
    }
}

/// A picture painted on one part of a 3D object ([`SolidMesh`]): a PNG, and for each corner of the
/// part's mesh where it falls on the picture (0 to 1 across from the left, 0 to 1 down from the
/// top). `opacity` is a grayscale PNG of the picture's alpha when some of it is not opaque.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Texture {
    pub png: Arc<Vec<u8>>,
    pub opacity: Option<Arc<Vec<u8>>>,
    pub uv: Vec<[f64; 2]>,
}

/// A distinct picture of a 3D object: the PNG and its opacity map.
type Picture = (Arc<Vec<u8>>, Option<Arc<Vec<u8>>>);

/// The distinct pictures of `textures` in order of first use, and the index among them of each
/// part's (`None` for a part without one).
fn images(parts: usize, textures: &[Option<Texture>]) -> (Vec<Picture>, Vec<Option<usize>>) {
    let mut out: Vec<Picture> = vec![];
    let index = (0..parts)
        .map(|k| {
            let t = textures.get(k)?.as_ref()?;
            Some(out.iter().position(|(p, _)| Arc::ptr_eq(p, &t.png) || p == &t.png).unwrap_or_else(|| {
                out.push((t.png.clone(), t.opacity.clone()));
                out.len() - 1
            }))
        })
        .collect();
    (out, index)
}

/// For each distinct picture of `textures`, whether it has an opacity map ([`mesh_file_names`]).
pub(crate) fn image_maps(textures: &[Option<Texture>]) -> Vec<bool> {
    images(textures.len(), textures).0.iter().map(|(_, map)| map.is_some()).collect()
}

/// The files of a 3D object named `stem` in `format` ([`FORMATS_3D`]): `(file name, bytes)`, the
/// OBJ with its MTL after it and the PNG files of `textures` last (an OBJ's opacity maps each after
/// its texture). `textures` holds the picture of each part that has one, in the order of `parts`.
pub(crate) fn mesh_files(stem: &str, format: &str, parts: &[SolidMesh], textures: &[Option<Texture>]) -> Result<Vec<(String, Vec<u8>)>, String> {
    check_parts(parts)?;
    let (images, image) = images(parts.len(), textures);
    let maps: Vec<bool> = images.iter().map(|(_, map)| map.is_some()).collect();
    let names = mesh_file_names(stem, format, &maps)?;
    let pictures: Vec<Arc<Vec<u8>>> = images.iter().map(|(p, _)| p.clone()).collect();
    let scene = Scene { parts, textures, image: &image, pictures: &pictures, maps: &maps };
    let pngs = || pictures.iter().map(|p| p.as_ref().clone());
    let bytes: Vec<Vec<u8>> = match format {
        "obj" => {
            let stem = obj_stem(stem);
            let (obj, mtl) = obj(&stem, &scene);
            let with_maps = images.iter().flat_map(|(p, map)| std::iter::once(p.as_ref().clone()).chain(map.as_ref().map(|m| m.as_ref().clone())));
            [obj.into_bytes(), mtl.into_bytes()].into_iter().chain(with_maps).collect()
        }
        "gltf" => vec![gltf(stem, &scene).into_bytes()],
        "usda" => std::iter::once(usda(stem, &scene).into_bytes()).chain(pngs()).collect(),
        _ => {
            let mut members = vec![(format!("{stem}.usda"), usda(stem, &scene).into_bytes())];
            members.extend(pngs().enumerate().map(|(k, png)| (texture_name(stem, k), png)));
            vec![super::zip::store_aligned(&members, 64)?]
        }
    };
    Ok(names.into_iter().zip(bytes).collect())
}

/// The parts of an object with their pictures: `image` is each part's index among `pictures`, and
/// `maps` says for each picture whether it has an opacity map.
struct Scene<'a> {
    parts: &'a [SolidMesh],
    textures: &'a [Option<Texture>],
    image: &'a [Option<usize>],
    pictures: &'a [Arc<Vec<u8>>],
    maps: &'a [bool],
}

impl Scene<'_> {
    /// Part `k`'s texture coordinates and picture, when it has a picture.
    fn texture(&self, k: usize) -> Option<(&[[f64; 2]], usize)> {
        let t = self.textures.get(k)?.as_ref()?;
        Some((t.uv.as_slice(), (*self.image.get(k)?)?))
    }

    /// Part `k`'s texture coordinates, one for each corner, 0 to 1 across from the left and down
    /// from the top: its picture's when it has one, else its mesh's own.
    fn coords(&self, k: usize) -> Option<&[[f64; 2]]> {
        let count = self.parts.get(k)?.mesh.positions.len();
        let own = self.parts.get(k).map(|p| p.mesh.uvs.as_slice());
        self.texture(k).map(|(uv, _)| uv).into_iter().chain(own).find(|uv| uv.len() == count && count > 0)
    }
}

/// An error when a part's mesh cannot be written as it is: its lists do not match, or a
/// position, normal or texture coordinate is not a number
/// ([`vectorcraft_render::effects::Mesh::check`]).
fn check_parts(parts: &[SolidMesh]) -> Result<(), String> {
    parts.iter().try_for_each(|p| p.mesh.check())
}

/// A material: a color, with a picture that replaces its red, green and blue.
#[derive(Clone, Copy, PartialEq)]
struct Material {
    color: [f32; 4],
    image: Option<usize>,
}

/// The distinct materials of `scene`'s parts in order, and each part's index among them.
fn materials(scene: &Scene) -> (Vec<Material>, Vec<usize>) {
    let mut out: Vec<Material> = vec![];
    let index = scene
        .parts
        .iter()
        .enumerate()
        .map(|(k, p)| {
            let m = Material { color: p.color, image: scene.texture(k).map(|(_, i)| i) };
            out.iter().position(|c| *c == m).unwrap_or_else(|| {
                out.push(m);
                out.len() - 1
            })
        })
        .collect();
    (out, index)
}

/// An sRGB channel (0..1) as linear light.
fn linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// A number as text: the shortest decimal that reads back as the same value.
fn num(v: f64) -> String {
    format!("{v}")
}

fn obj(stem: &str, scene: &Scene) -> (String, String) {
    let parts = scene.parts;
    let (materials, material) = materials(scene);
    let mut o = String::new();
    let _ = writeln!(o, "mtllib {stem}.mtl");
    let _ = writeln!(o, "o {stem}");
    for p in parts {
        for v in &p.mesh.positions {
            let _ = writeln!(o, "v {} {} {}", num(v[0]), num(v[1]), num(v[2]));
        }
    }
    // OBJ's texture coordinates run up from the picture's bottom.
    for k in 0..parts.len() {
        for uv in scene.coords(k).unwrap_or_default() {
            let _ = writeln!(o, "vt {} {}", num(uv[0]), num(1.0 - uv[1]));
        }
    }
    for p in parts {
        for n in &p.mesh.normals {
            let _ = writeln!(o, "vn {} {} {}", num(n[0]), num(n[1]), num(n[2]));
        }
    }
    let (mut base, mut uv_base) = (1usize, 1usize);
    for (k, p) in parts.iter().enumerate() {
        let _ = writeln!(o, "g Solid_{}", k + 1);
        let _ = writeln!(o, "usemtl Material_{}", material.get(k).copied().unwrap_or(0) + 1);
        let coords = scene.coords(k).is_some();
        for t in &p.mesh.triangles {
            if coords {
                let [a, b, c] = t.map(|i| i as usize);
                let corner = |i: usize| format!("{}/{}/{}", i + base, i + uv_base, i + base);
                let _ = writeln!(o, "f {} {} {}", corner(a), corner(b), corner(c));
            } else {
                let [a, b, c] = t.map(|i| i as usize + base);
                let _ = writeln!(o, "f {a}//{a} {b}//{b} {c}//{c}");
            }
        }
        base += p.mesh.positions.len();
        uv_base += scene.coords(k).map_or(0, <[_]>::len);
    }
    let mut m = String::new();
    for (k, mat) in materials.iter().enumerate() {
        let c = mat.color;
        let _ = writeln!(m, "newmtl Material_{}", k + 1);
        match mat.image {
            // The picture holds the color; Kd multiplies it.
            Some(_) => {
                let _ = writeln!(m, "Kd 1 1 1");
            }
            None => {
                let _ = writeln!(m, "Kd {} {} {}", num(f64::from(c[0])), num(f64::from(c[1])), num(f64::from(c[2])));
            }
        }
        // A picture's opacity map holds the opacity, as gray levels.
        let _ = writeln!(m, "d {}", if mat.image.is_some() { "1".to_string() } else { num(f64::from(c[3])) });
        let _ = writeln!(m, "illum 1");
        if let Some(i) = mat.image {
            let _ = writeln!(m, "map_Kd -clamp on {}", texture_name(stem, i));
            if scene.maps.get(i).copied().unwrap_or(false) {
                let _ = writeln!(m, "map_d -clamp on {}", opacity_name(stem, i));
            }
        }
    }
    (o, m)
}

fn gltf(stem: &str, scene: &Scene) -> String {
    let parts = scene.parts;
    let (materials, material) = materials(scene);
    let mut buffer: Vec<u8> = vec![];
    let (mut views, mut accessors, mut primitives) = (vec![], vec![], vec![]);
    let mut view = |buffer: &mut Vec<u8>, bytes: Vec<u8>, target: u32| -> usize {
        views.push(json!({ "buffer": 0, "byteOffset": buffer.len(), "byteLength": bytes.len(), "target": target }));
        buffer.extend(bytes);
        views.len() - 1
    };
    for (k, p) in parts.iter().enumerate() {
        let floats = |v: &[[f64; 3]], scale: f64| v.iter().flatten().flat_map(|x| ((x * scale) as f32).to_le_bytes()).collect::<Vec<u8>>();
        let (lo, hi) = p.mesh.bounds().unwrap_or_default();
        let positions = view(&mut buffer, floats(&p.mesh.positions, METERS_PER_POINT), 34962);
        let normals = view(&mut buffer, floats(&p.mesh.normals, 1.0), 34962);
        let indices = view(&mut buffer, p.mesh.triangles.iter().flatten().flat_map(|i| i.to_le_bytes()).collect(), 34963);
        let count = p.mesh.positions.len();
        let at = accessors.len();
        accessors.push(json!({
            "bufferView": positions, "componentType": 5126, "count": count, "type": "VEC3",
            "min": lo.map(|v| (v * METERS_PER_POINT) as f32), "max": hi.map(|v| (v * METERS_PER_POINT) as f32),
        }));
        accessors.push(json!({ "bufferView": normals, "componentType": 5126, "count": count, "type": "VEC3" }));
        accessors.push(json!({ "bufferView": indices, "componentType": 5125, "count": p.mesh.triangles.len() * 3, "type": "SCALAR" }));
        let mut primitive = json!({
            "attributes": { "POSITION": at, "NORMAL": at + 1 },
            "indices": at + 2,
            "material": material.get(k).copied().unwrap_or(0),
        });
        // glTF's texture coordinates run down from the picture's top, as the picture's own do.
        if let Some(uv) = scene.coords(k) {
            let bytes = uv.iter().flatten().flat_map(|x| (*x as f32).to_le_bytes()).collect();
            let coords = view(&mut buffer, bytes, 34962);
            primitive["attributes"]["TEXCOORD_0"] = json!(accessors.len());
            accessors.push(json!({ "bufferView": coords, "componentType": 5126, "count": count, "type": "VEC2" }));
        }
        primitives.push(primitive);
    }
    let materials: Vec<Value> = materials
        .iter()
        .enumerate()
        .map(|(k, mat)| {
            let c = mat.color;
            let mut m = json!({
                "name": format!("Material_{}", k + 1),
                "pbrMetallicRoughness": {
                    "baseColorFactor": [linear(c[0]), linear(c[1]), linear(c[2]), c[3]],
                    "metallicFactor": 0.0,
                    "roughnessFactor": 1.0,
                },
                "doubleSided": true,
            });
            if let Some(i) = mat.image {
                // The picture holds the color and the opacity; the factor multiplies them.
                m["pbrMetallicRoughness"]["baseColorFactor"] = json!([1.0, 1.0, 1.0, 1.0]);
                m["pbrMetallicRoughness"]["baseColorTexture"] = json!({ "index": i });
            }
            if c[3] < 1.0 {
                m["alphaMode"] = json!("BLEND");
            }
            m
        })
        .collect();
    let mut doc = json!({
        "asset": { "version": "2.0", "generator": "VectorCraft" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "name": stem, "mesh": 0 }],
        "meshes": [{ "name": stem, "primitives": primitives }],
        "materials": materials,
        "buffers": [{
            "byteLength": buffer.len(),
            "uri": format!("data:application/octet-stream;base64,{}", vectorcraft_format::base64_encode(&buffer)),
        }],
        "bufferViews": views,
        "accessors": accessors,
    });
    if !scene.pictures.is_empty() {
        // Linear filtering, mipmaps, and coordinates held at the picture's edge.
        doc["samplers"] = json!([{ "magFilter": 9729, "minFilter": 9987, "wrapS": 33071, "wrapT": 33071 }]);
        doc["textures"] = Value::Array((0..scene.pictures.len()).map(|i| json!({ "sampler": 0, "source": i })).collect());
        doc["images"] = Value::Array(
            scene.pictures.iter().map(|p| json!({ "uri": format!("data:image/png;base64,{}", vectorcraft_format::base64_encode(p)) })).collect(),
        );
    }
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

/// `name` as a USD prim name: letters, digits and `_`, not starting with a digit.
fn prim_name(name: &str) -> String {
    let mut out: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

fn usda(stem: &str, scene: &Scene) -> String {
    let parts = scene.parts;
    let (materials, material) = materials(scene);
    let root = prim_name(stem);
    let triple = |v: [f64; 3]| format!("({}, {}, {})", num(v[0]), num(v[1]), num(v[2]));
    let list = |items: Vec<String>| items.join(", ");
    let mut u = String::new();
    let _ = writeln!(u, "#usda 1.0");
    let _ = writeln!(u, "(\n    defaultPrim = \"{root}\"\n    metersPerUnit = {}\n    upAxis = \"Y\"\n)\n", METERS_PER_POINT);
    let _ = writeln!(u, "def Xform \"{root}\"\n{{");
    for (k, p) in parts.iter().enumerate() {
        let (lo, hi) = p.mesh.bounds().unwrap_or_default();
        let m = material.get(k).copied().unwrap_or(0) + 1;
        let _ = writeln!(u, "    def Mesh \"Solid_{}\" (\n        prepend apiSchemas = [\"MaterialBindingAPI\"]\n    )\n    {{", k + 1);
        let _ = writeln!(u, "        uniform bool doubleSided = 1");
        let _ = writeln!(u, "        float3[] extent = [{}, {}]", triple(lo), triple(hi));
        let _ = writeln!(u, "        int[] faceVertexCounts = [{}]", list(vec!["3".to_string(); p.mesh.triangles.len()]));
        let _ = writeln!(u, "        int[] faceVertexIndices = [{}]", list(p.mesh.triangles.iter().flatten().map(u32::to_string).collect()));
        let _ = writeln!(u, "        rel material:binding = </{root}/Materials/Material_{m}>");
        let _ = writeln!(
            u,
            "        normal3f[] normals = [{}] (\n            interpolation = \"vertex\"\n        )",
            list(p.mesh.normals.iter().map(|n| triple(*n)).collect())
        );
        let _ = writeln!(u, "        point3f[] points = [{}]", list(p.mesh.positions.iter().map(|v| triple(*v)).collect()));
        let c = p.color;
        let _ = writeln!(
            u,
            "        color3f[] primvars:displayColor = [({}, {}, {})] (\n            interpolation = \"constant\"\n        )",
            linear(c[0]),
            linear(c[1]),
            linear(c[2])
        );
        let _ = writeln!(u, "        float[] primvars:displayOpacity = [{}] (\n            interpolation = \"constant\"\n        )", c[3]);
        // USD's texture coordinates run up from the picture's bottom.
        if let Some(uv) = scene.coords(k) {
            let st = list(uv.iter().map(|t| format!("({}, {})", num(t[0]), num(1.0 - t[1]))).collect());
            let _ = writeln!(u, "        texCoord2f[] primvars:st = [{st}] (\n            interpolation = \"vertex\"\n        )");
        }
        let _ = writeln!(u, "        uniform token subdivisionScheme = \"none\"\n    }}\n");
    }
    let _ = writeln!(u, "    def Scope \"Materials\"\n    {{");
    for (k, mat) in materials.iter().enumerate() {
        let (m, c) = (k + 1, mat.color);
        let at = format!("/{root}/Materials/Material_{m}");
        let _ = writeln!(u, "        def Material \"Material_{m}\"\n        {{");
        let _ = writeln!(u, "            token outputs:surface.connect = <{at}/Surface.outputs:surface>\n");
        let _ = writeln!(u, "            def Shader \"Surface\"\n            {{");
        let _ = writeln!(u, "                uniform token info:id = \"UsdPreviewSurface\"");
        match mat.image {
            // The picture holds the color, and the opacity when the face is not opaque.
            Some(_) => {
                let _ = writeln!(u, "                color3f inputs:diffuseColor.connect = <{at}/Texture.outputs:rgb>");
                if c[3] < 1.0 {
                    let _ = writeln!(u, "                float inputs:opacity.connect = <{at}/Texture.outputs:a>");
                } else {
                    let _ = writeln!(u, "                float inputs:opacity = 1");
                }
            }
            None => {
                let _ = writeln!(u, "                color3f inputs:diffuseColor = ({}, {}, {})", linear(c[0]), linear(c[1]), linear(c[2]));
                let _ = writeln!(u, "                float inputs:opacity = {}", c[3]);
            }
        }
        let _ = writeln!(u, "                float inputs:roughness = 1");
        let _ = writeln!(u, "                token outputs:surface\n            }}");
        if let Some(i) = mat.image {
            let _ = writeln!(u, "\n            def Shader \"Texture\"\n            {{");
            let _ = writeln!(u, "                uniform token info:id = \"UsdUVTexture\"");
            let _ = writeln!(u, "                asset inputs:file = @{}@", texture_name(stem, i));
            let _ = writeln!(u, "                float2 inputs:st.connect = <{at}/Coordinates.outputs:result>");
            let _ = writeln!(u, "                token inputs:sourceColorSpace = \"sRGB\"");
            let _ = writeln!(u, "                token inputs:wrapS = \"clamp\"");
            let _ = writeln!(u, "                token inputs:wrapT = \"clamp\"");
            let _ = writeln!(u, "                float3 outputs:rgb");
            if c[3] < 1.0 {
                let _ = writeln!(u, "                float outputs:a");
            }
            let _ = writeln!(u, "            }}");
            let _ = writeln!(u, "\n            def Shader \"Coordinates\"\n            {{");
            let _ = writeln!(u, "                uniform token info:id = \"UsdPrimvarReader_float2\"");
            let _ = writeln!(u, "                string inputs:varname = \"st\"");
            let _ = writeln!(u, "                float2 outputs:result\n            }}");
        }
        let _ = writeln!(u, "        }}");
    }
    let _ = writeln!(u, "    }}\n}}");
    u
}

/// The pictures of the graphics on `parts` ([`SolidMesh::graphics`]), one for each part in order
/// (`None` for a part without graphics, or whose graphics draw nothing): the part's color with its
/// graphics drawn over it, flat and unshaded ([`vectorcraft_render::effects::mesh_graphics_art`]),
/// over the bounds of the art points of its corners, `size` pixels along the longer side. Parts with
/// the same color, face, graphics and bounds share one picture. `pixels` counts the pixels drawn so
/// far, and every call of one export shares it: an error when it passes [`MAX_TEXTURE_PIXELS`].
pub(crate) fn textures(doc: &Document, parts: &[SolidMesh], size: u32, pixels: &mut u64) -> Result<Vec<Option<Texture>>, String> {
    let mut renderer: Option<vectorcraft_render::Renderer> = None;
    // Each picture drawn: the part it was drawn for, its bounds, and the PNG with its opacity map
    // (`None`: nothing drawn).
    let mut drawn: Vec<Drawn> = vec![];
    let mut out = vec![];
    for part in parts {
        let Some(g) = &part.graphics else {
            out.push(None);
            continue;
        };
        let Some(frame) = texture_frame(g) else {
            out.push(None);
            continue;
        };
        let scale = f64::from(size) / frame.width().max(frame.height());
        let fit = |side: f64| ((side * scale).ceil() as u32).clamp(1, size);
        let (w, h) = (fit(frame.width()), fit(frame.height()));
        let same = |(p, r, _): &&Drawn| p.color == part.color && *r == frame && p.graphics.as_ref().is_some_and(|o| same_graphics(o, g));
        let png = match drawn.iter().find(same) {
            Some((_, _, png)) => png.clone(),
            None => {
                let png = match vectorcraft_render::effects::mesh_graphics_art(doc, NodeId(0), g)? {
                    None => None,
                    Some(art) => {
                        *pixels = pixels.saturating_add(u64::from(w) * u64::from(h));
                        if *pixels > MAX_TEXTURE_PIXELS {
                            return Err(format!(
                                "the graphics need more than {MAX_TEXTURE_PIXELS} pixels of textures: lower textureSize (now {size})"
                            ));
                        }
                        let renderer = renderer.get_or_insert_with(vectorcraft_render::Renderer::new);
                        Some(picture(doc, renderer, (part.color, g.opacity, g.object), art, (frame, scale), (w, h))?)
                    }
                };
                drawn.push((part, frame, png.clone()));
                png
            }
        };
        out.push(png.map(|(png, opacity)| {
            let uv = g.art.iter().map(|p| [(p.x - frame.x0) * scale / f64::from(w), (p.y - frame.y0) * scale / f64::from(h)]).collect();
            Texture { png, opacity, uv }
        }));
    }
    Ok(out)
}

/// Does [`textures`] draw a picture for one of `parts`: a part with graphics whose corners span an
/// area and whose graphics draw something? Nothing is rendered.
pub fn draws_textures(doc: &Document, parts: &[SolidMesh]) -> bool {
    parts.iter().any(|p| {
        p.graphics
            .as_ref()
            .is_some_and(|g| texture_frame(g).is_some() && matches!(vectorcraft_render::effects::mesh_graphics_art(doc, NodeId(0), g), Ok(Some(_))))
    })
}

/// The part of the art's plane a picture of `g` covers: the bounds of its corners' art points, when
/// they span an area.
fn texture_frame(g: &MeshGraphics) -> Option<Rect> {
    g.art_bounds().filter(|r| r.width() > 1e-9 && r.height() > 1e-9 && r.width().is_finite() && r.height().is_finite())
}

/// A picture [`textures`] drew: the part, the bounds and the PNG with its opacity map.
type Drawn<'a> = (&'a SolidMesh, Rect, Option<Picture>);

/// Do `a` and `b` draw the same graphics on their face?
fn same_graphics(a: &MeshGraphics, b: &MeshGraphics) -> bool {
    a.face == b.face && a.center == b.center && a.graphics == b.graphics && a.opacity == b.opacity && a.object == b.object
}

/// `art` drawn over `color` as a PNG of `w` × `h` pixels whose top left is `frame`'s top left, at
/// `scale` pixels a point, with a grayscale PNG of its alpha when some of it is not opaque. As on
/// the canvas, the art is drawn at `face` (the opacity of the face's fill or stroke), the color at
/// its own opacity without `object`'s, and `object` (the object's opacity) applies to the whole
/// picture.
fn picture(
    doc: &Document,
    renderer: &mut vectorcraft_render::Renderer,
    (color, face, object): ([f32; 4], f32, f32),
    mut art: Node,
    (frame, scale): (Rect, f64),
    (w, h): (u32, u32),
) -> Result<Picture, String> {
    // The color reaches two pixels past the frame, so the picture's edge pixels are covered.
    let reach = 2.0 / scale;
    let back = Rect::new(frame.x0 - reach, frame.y0 - reach, frame.x0 + f64::from(w) / scale + reach, frame.y0 + f64::from(h) / scale + reach);
    let fill = Paint::solid(Color::rgb(color[0], color[1], color[2]));
    let mut ground = Node::path(NodeId(0), vectorcraft_geom::shapes::rectangle(back), Appearance::basic(fill, Paint::None, 0.0));
    let unit = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    let object = unit(object);
    ground.opacity = if object > 0.0 { unit(color[3] / object) } else { 0.0 };
    art.opacity *= unit(face);
    let mut group = Node::group(NodeId(0), vec![Arc::new(ground), Arc::new(art)]);
    group.opacity = object;
    let group = Arc::new(group);
    let (pw, ph) = (u16::try_from(w).map_err(|_| "texture too large")?, u16::try_from(h).map_err(|_| "texture too large")?);
    let view = Affine::scale(scale) * Affine::translate((-frame.x0, -frame.y0));
    let drawn = renderer.render_node(doc, &group, pw, ph, view).ok_or_else(|| "the graphics could not be drawn".to_string())?;
    let alpha: Vec<u8> = drawn.pixels.iter().skip(3).step_by(4).copied().collect();
    let opacity = if alpha.iter().all(|a| *a == 255) {
        None
    } else {
        let gray = image::GrayImage::from_raw(w, h, alpha).ok_or("the texture's opacity could not be drawn")?;
        let mut png = vec![];
        gray.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| format!("PNG encoding failed: {e}"))?;
        Some(Arc::new(png))
    };
    Ok((Arc::new(drawn.to_png()?), opacity))
}

#[cfg(test)]
mod tests;
