//! 3D objects as mesh files, for a 3D asset of Window › Asset Export (`assets.export3D`): OBJ with
//! its MTL materials, glTF 2.0 (one `.gltf` file, its buffer embedded as a data URI), USDA and
//! USDZ (the USDA layer in an uncompressed ZIP, its data on a 64-byte boundary).
//!
//! Each solid of the object ([`vectorcraft_render::effects::three_d_meshes`]) is its own mesh, with a
//! material in the color it is painted (solids of one color share it). Positions are y up and
//! about the point the object turns about: in points in OBJ and USD (whose layer says how many
//! meters a unit is), in meters in glTF, as its specification asks. glTF and USD colors are linear,
//! as their specifications ask; MTL colors are the display colors. Materials are two-sided, since
//! an open solid or a turned plane is seen from both sides.
//!
//! Formats: glTF 2.0 <https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html>, USD
//! <https://openusd.org/release/index.html> (USDZ: <https://openusd.org/release/spec_usdz.html>).

use std::fmt::Write as _;

use serde_json::{Value, json};
use vectorcraft_render::effects::SolidMesh;

/// The formats a 3D asset exports to, as `assets.set3DFormat` names them: USDA first, the default.
pub const FORMATS_3D: [&str; 4] = ["usda", "usdz", "gltf", "obj"];
/// The format a new 3D asset exports to.
pub const DEFAULT_FORMAT_3D: &str = "usda";
/// The meters in a point (1/72 inch).
const METERS_PER_POINT: f64 = 0.0254 / 72.0;

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

/// The names of the files [`mesh_files`] writes for `stem` in `format`.
pub(crate) fn mesh_file_names(stem: &str, format: &str) -> Result<Vec<String>, String> {
    match format {
        "obj" => {
            let stem = obj_stem(stem);
            Ok(vec![format!("{stem}.obj"), format!("{stem}.mtl")])
        }
        "gltf" | "usda" | "usdz" => Ok(vec![format!("{stem}.{format}")]),
        _ => {
            let key = "format";
            Err(format!("`{key}` must be one of {}", FORMATS_3D.join(", ")))
        }
    }
}

/// The files of a 3D object named `stem` in `format` ([`FORMATS_3D`]): `(file name, bytes)`, the
/// OBJ with its MTL after it.
pub(crate) fn mesh_files(stem: &str, format: &str, parts: &[SolidMesh]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let names = mesh_file_names(stem, format)?;
    let bytes = match format {
        "obj" => {
            let (obj, mtl) = obj(&obj_stem(stem), parts);
            vec![obj.into_bytes(), mtl.into_bytes()]
        }
        "gltf" => vec![gltf(stem, parts).into_bytes()],
        "usda" => vec![usda(stem, parts).into_bytes()],
        _ => vec![super::zip::store_aligned(&[(format!("{stem}.usda"), usda(stem, parts).into_bytes())], 64)?],
    };
    Ok(names.into_iter().zip(bytes).collect())
}

/// The distinct colors of `parts` in order, and each part's index among them.
fn materials(parts: &[SolidMesh]) -> (Vec<[f32; 4]>, Vec<usize>) {
    let mut colors: Vec<[f32; 4]> = vec![];
    let index = parts
        .iter()
        .map(|p| {
            colors.iter().position(|c| *c == p.color).unwrap_or_else(|| {
                colors.push(p.color);
                colors.len() - 1
            })
        })
        .collect();
    (colors, index)
}

/// An sRGB channel (0..1) as linear light.
fn linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// A number as text: the shortest decimal that reads back as the same value.
fn num(v: f64) -> String {
    format!("{v}")
}

fn obj(stem: &str, parts: &[SolidMesh]) -> (String, String) {
    let (colors, material) = materials(parts);
    let mut o = String::new();
    let _ = writeln!(o, "mtllib {stem}.mtl");
    let _ = writeln!(o, "o {stem}");
    for p in parts {
        for v in &p.mesh.positions {
            let _ = writeln!(o, "v {} {} {}", num(v[0]), num(v[1]), num(v[2]));
        }
    }
    for p in parts {
        for n in &p.mesh.normals {
            let _ = writeln!(o, "vn {} {} {}", num(n[0]), num(n[1]), num(n[2]));
        }
    }
    let mut base = 1usize;
    for (k, p) in parts.iter().enumerate() {
        let _ = writeln!(o, "g Solid_{}", k + 1);
        let _ = writeln!(o, "usemtl Material_{}", material.get(k).copied().unwrap_or(0) + 1);
        for t in &p.mesh.triangles {
            let [a, b, c] = t.map(|i| i as usize + base);
            let _ = writeln!(o, "f {a}//{a} {b}//{b} {c}//{c}");
        }
        base += p.mesh.positions.len();
    }
    let mut m = String::new();
    for (k, c) in colors.iter().enumerate() {
        let _ = writeln!(m, "newmtl Material_{}", k + 1);
        let _ = writeln!(m, "Kd {} {} {}", num(f64::from(c[0])), num(f64::from(c[1])), num(f64::from(c[2])));
        let _ = writeln!(m, "d {}", num(f64::from(c[3])));
        let _ = writeln!(m, "illum 1");
    }
    (o, m)
}

fn gltf(stem: &str, parts: &[SolidMesh]) -> String {
    let (colors, material) = materials(parts);
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
        primitives.push(json!({
            "attributes": { "POSITION": at, "NORMAL": at + 1 },
            "indices": at + 2,
            "material": material.get(k).copied().unwrap_or(0),
        }));
    }
    let materials: Vec<Value> = colors
        .iter()
        .enumerate()
        .map(|(k, c)| {
            let mut m = json!({
                "name": format!("Material_{}", k + 1),
                "pbrMetallicRoughness": {
                    "baseColorFactor": [linear(c[0]), linear(c[1]), linear(c[2]), c[3]],
                    "metallicFactor": 0.0,
                    "roughnessFactor": 1.0,
                },
                "doubleSided": true,
            });
            if c[3] < 1.0 {
                m["alphaMode"] = json!("BLEND");
            }
            m
        })
        .collect();
    let doc = json!({
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

fn usda(stem: &str, parts: &[SolidMesh]) -> String {
    let (colors, material) = materials(parts);
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
        let _ = writeln!(u, "        uniform token subdivisionScheme = \"none\"\n    }}\n");
    }
    let _ = writeln!(u, "    def Scope \"Materials\"\n    {{");
    for (k, c) in colors.iter().enumerate() {
        let m = k + 1;
        let _ = writeln!(u, "        def Material \"Material_{m}\"\n        {{");
        let _ = writeln!(u, "            token outputs:surface.connect = </{root}/Materials/Material_{m}/Surface.outputs:surface>\n");
        let _ = writeln!(u, "            def Shader \"Surface\"\n            {{");
        let _ = writeln!(u, "                uniform token info:id = \"UsdPreviewSurface\"");
        let _ = writeln!(u, "                color3f inputs:diffuseColor = ({}, {}, {})", linear(c[0]), linear(c[1]), linear(c[2]));
        let _ = writeln!(u, "                float inputs:opacity = {}", c[3]);
        let _ = writeln!(u, "                float inputs:roughness = 1");
        let _ = writeln!(u, "                token outputs:surface\n            }}\n        }}");
    }
    let _ = writeln!(u, "    }}\n}}");
    u
}

#[cfg(test)]
mod tests;
