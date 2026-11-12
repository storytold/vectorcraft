//! Extrude & Bevel (Classic): the solid Extrude & Bevel builds from a path, drawn with the classic
//! surfaces (Wireframe, No Shading, Diffuse and Plastic), their highlight, blend steps and shading
//! color. Rotate (Classic): the plane Rotate turns, drawn with the No Shading and Diffuse surfaces.
use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::{BlendMode, Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, FillLayer, Node};
use vectorcraft_geom::Rect;
use vectorcraft_three_d::{Bevel, BevelShape, ClassicShading, ClassicSurface, Extrude, Facet, Light};

use crate::util::{flag, num, text};

pub const CLASSIC_EXTRUDE: &str = "threeD.classicExtrude";
pub const CLASSIC_ROTATE: &str = "threeD.classicRotate";

/// The surfaces Rotate (Classic) takes: No Shading and Diffuse.
pub const ROTATE_SURFACES: [&str; 2] = ["none", "diffuse"];

/// The width of a Wireframe edge, in points.
const WIRE: f64 = 0.5;

/// The Shading Color choices: black, a custom color, or none.
pub const SHADING_COLORS: [&str; 3] = ["black", "custom", "none"];

/// The surface settings of Extrude & Bevel (Classic)'s parameters.
pub fn classic_shading(p: &Value) -> ClassicShading {
    let d = ClassicShading::default();
    ClassicShading {
        surface: ClassicSurface::parse(text(p, "surface", "plastic")).unwrap_or_default(),
        ambient: num(p, "ambient", d.ambient * 100.0) / 100.0,
        highlight_intensity: num(p, "highlightIntensity", d.highlight_intensity * 100.0) / 100.0,
        highlight_size: num(p, "highlightSize", d.highlight_size * 100.0) / 100.0,
        blend_steps: num(p, "blendSteps", f64::from(d.blend_steps)).round().clamp(1.0, 256.0) as u32,
    }
}

/// The solid of Extrude & Bevel (Classic)'s parameters: its depth, cap and bevel (as tall as it is
/// wide, added around the shape or carved into it), with the faces turned away from the viewer kept
/// for Draw Hidden Faces and for the Wireframe surface.
pub fn classic_extrude_options(p: &Value) -> Extrude {
    let height = num(p, "bevelHeight", 4.0);
    Extrude {
        depth: num(p, "depth", 50.0),
        cap: flag(p, "cap", true),
        bevel: BevelShape::parse(text(p, "bevel", "none")).map(|shape| Bevel {
            shape,
            width: height,
            height,
            inside: flag(p, "bevelInside", false),
            ..Bevel::default()
        }),
        hidden: flag(p, "drawHiddenFaces", false) || classic_shading(p).surface == ClassicSurface::Wireframe,
        ..Extrude::default()
    }
}

/// The color faces darken toward: black, the custom color, or none (faces keep their paint and only
/// the highlight lightens them).
fn shading_color(p: &Value) -> Option<Color> {
    match text(p, "shadingColor", "black") {
        "none" => None,
        "custom" => Some(p.get("shadingCustom").and_then(Value::as_str).and_then(parse_hex).unwrap_or(Color::BLACK)),
        _ => Some(Color::BLACK),
    }
}

fn parse_hex(s: &str) -> Option<Color> {
    (s.len() == 7 && s.starts_with('#') && s.bytes().skip(1).all(|b| b.is_ascii_hexdigit())).then(|| Color::from_hex(s)).flatten()
}

/// The lights: the `lights` list when it is given, else the one light of `lightRotation`,
/// `lightHeight`, `lightBehind` and `lightIntensity`. Each is its direction toward the light in view
/// space and its intensity (0..1).
fn lights(p: &Value) -> Vec<([f64; 3], f64)> {
    if let Some(list) = crate::lighting::lights(p) {
        return list.iter().map(|l| (l.direction(), l.intensity)).collect();
    }
    let light = Light {
        rotation: num(p, "lightRotation", vectorcraft_three_d::DEFAULT_LIGHT.0),
        height: num(p, "lightHeight", vectorcraft_three_d::DEFAULT_LIGHT.1),
        behind: flag(p, "lightBehind", false),
        ..Light::default()
    };
    vec![(light.direction(), num(p, "lightIntensity", 100.0) / 100.0)]
}

/// The numeric parameters: (key, lowest, highest, whole numbers only).
const RANGES: [(&str, f64, f64, bool); 13] = [
    ("depth", 0.0, 2000.0, false),
    ("bevelHeight", 1.0, 100.0, false),
    ("rotationX", -180.0, 180.0, false),
    ("rotationY", -180.0, 180.0, false),
    ("rotationZ", -180.0, 180.0, false),
    ("perspectiveAngle", 0.0, 160.0, false),
    ("lightRotation", -180.0, 180.0, false),
    ("lightHeight", 0.0, 90.0, false),
    ("lightIntensity", 0.0, 100.0, false),
    ("ambient", 0.0, 100.0, false),
    ("highlightIntensity", 0.0, 100.0, false),
    ("highlightSize", 0.0, 100.0, false),
    ("blendSteps", 1.0, 256.0, true),
];

const FLAGS: [&str; 4] = ["cap", "bevelInside", "lightBehind", "drawHiddenFaces"];

/// Check Extrude & Bevel (Classic)'s parameters `p` (its defaults filled in): every key is one of
/// its parameters, numbers are in range, flags are true or false, and `surface`, `shadingColor` and
/// `shadingCustom` name a surface, a shading color and a #rrggbb color.
pub fn validate(p: &Value) -> Result<(), String> {
    validate_classic(CLASSIC_EXTRUDE, p, &RANGES, &FLAGS)
}

/// Rotate (Classic)'s numeric parameters: (key, lowest, highest, whole numbers only).
const ROTATE_RANGES: [(&str, f64, f64, bool); 9] = [
    ("rotationX", -180.0, 180.0, false),
    ("rotationY", -180.0, 180.0, false),
    ("rotationZ", -180.0, 180.0, false),
    ("perspectiveAngle", 0.0, 160.0, false),
    ("lightRotation", -180.0, 180.0, false),
    ("lightHeight", 0.0, 90.0, false),
    ("lightIntensity", 0.0, 100.0, false),
    ("ambient", 0.0, 100.0, false),
    ("blendSteps", 1.0, 256.0, true),
];

/// Check Rotate (Classic)'s parameters `p` (its defaults filled in) as [`validate`] checks Extrude &
/// Bevel (Classic)'s, with `surface` No Shading or Diffuse only.
pub fn validate_rotate(p: &Value) -> Result<(), String> {
    validate_classic(CLASSIC_ROTATE, p, &ROTATE_RANGES, &["lightBehind"])?;
    if p.get("surface").is_some_and(|v| !v.as_str().is_some_and(|s| ROTATE_SURFACES.contains(&s))) {
        return Err(format!("`surface` must be one of {}", ROTATE_SURFACES.join(", ")));
    }
    Ok(())
}

/// Check the parameters `p` of classic 3D effect `id` (its defaults filled in): every key is one of
/// its parameters, the numbers `ranges` lists, as (key, lowest, highest, whole numbers only), are in
/// range, the `flags` are true or false, and `surface`, `shadingColor` and `shadingCustom` name a
/// surface, a shading color and a #rrggbb color.
pub(crate) fn validate_classic(id: &str, p: &Value, ranges: &[(&str, f64, f64, bool)], flags: &[&str]) -> Result<(), String> {
    let Some(map) = p.as_object() else { return Ok(()) };
    let defaults = crate::default_params(id).unwrap_or(Value::Null);
    let known: Vec<&str> = defaults.as_object().map(|m| m.keys().map(String::as_str).collect()).unwrap_or_default();
    for key in map.keys() {
        if !known.contains(&key.as_str()) {
            let keys = known.join(", ");
            return Err(format!("unknown parameter `{key}`; the command takes: {keys}"));
        }
    }
    for &(key, min, max, whole) in ranges {
        let Some(v) = map.get(key) else { continue };
        let Some(x) = v.as_f64() else { return Err(format!("`{key}` must be a number")) };
        if whole && (x.fract() != 0.0 || !(min..=max).contains(&x)) {
            return Err(format!("`{key}` must be a whole number between {min} and {max} (got {x})"));
        }
        if !(min..=max).contains(&x) {
            return Err(format!("`{key}` must be between {min} and {max} (got {x})"));
        }
    }
    for &key in flags {
        if map.get(key).is_some_and(|v| !v.is_boolean()) {
            return Err(format!("`{key}` must be true or false"));
        }
    }
    let one_of = |key: &str, ids: &[&str]| -> Result<(), String> {
        match map.get(key) {
            Some(v) if !v.as_str().is_some_and(|s| ids.contains(&s)) => Err(format!("`{key}` must be one of {}", ids.join(", "))),
            _ => Ok(()),
        }
    };
    let surfaces: Vec<&str> = ClassicSurface::ALL.iter().map(|s| s.id()).collect();
    one_of("surface", &surfaces)?;
    one_of("shadingColor", &SHADING_COLORS)?;
    let key = "shadingCustom";
    if map.get(key).is_some_and(|v| !v.as_str().is_some_and(|s| parse_hex(s).is_some())) {
        return Err(format!("`{key}` must be a #rrggbb color"));
    }
    Ok(())
}

/// `c` with tone `tone` (0 the shading color, 1 the color itself) and highlight `spec` (toward
/// white) on top.
fn shaded(c: Color, tone: f64, spec: f64, shade: Option<Color>) -> Color {
    let base = c.to_rgb();
    let dark = shade.map_or(base, |s| s.to_rgb());
    let (tone, spec) = (tone as f32, spec as f32);
    let [r, g, b] = [0, 1, 2].map(|i| {
        let mixed = dark[i] + (base[i] - dark[i]) * tone;
        (mixed + (1.0 - mixed) * spec).clamp(0.0, 1.0)
    });
    Color::rgb(r, g, b)
}

/// One solid's faces in paint `paint`, as a group with the paint's opacity and blend mode, one child
/// for each facet: the edges of the faces on Wireframe, else each face filled in its paint shaded by
/// its tone and highlight. A gradient with transparent stops is shaded opaque and then masked by
/// its own transparency, so the shading follows the gradient's coverage.
pub(crate) fn surface(n: &Node, facets: Vec<Facet>, source: &Paint, opacity: f32, blend: BlendMode, p: &Value) -> Node {
    let shading = classic_shading(p);
    let lights = lights(p);
    let shade = shading_color(p);
    let wire = shading.surface == ClassicSurface::Wireframe;
    let (paint, mask) = if wire { (source.clone(), None) } else { crate::lighting::opaque_paint(source) };
    let paint = &paint;
    let children = facets
        .into_iter()
        .map(|face| {
            if wire {
                // A wire edge is drawn as a seam is: a round-joined stroke of the paint.
                let edge = crate::lighting::seam_stroke(paint.clone(), WIRE);
                return Arc::new(Node::path(n.id, face.path(), Appearance { items: vec![AppearanceItem::Stroke(edge)], ..Default::default() }));
            }
            let (tone, spec) = shading.tone(face.normal, &lights);
            // A narrow stroke of the face's own paint closes the hairline seams between faces.
            let seam = (face.thickness() / 2.0).clamp(0.05, 0.75);
            let items = match paint.color() {
                // Unshaded (No Shading, no shading color, or a face in full light) with no
                // highlight: the paint itself, its color space and swatch kept.
                Some(_) if (tone >= 1.0 || shade.is_none()) && spec <= 0.0 => {
                    vec![
                        AppearanceItem::Fill(FillLayer::new(paint.clone())),
                        AppearanceItem::Stroke(crate::lighting::seam_stroke(paint.clone(), seam)),
                    ]
                }
                Some(c) => {
                    let fill = Paint::solid(shaded(c, tone, spec, shade));
                    vec![AppearanceItem::Fill(FillLayer::new(fill.clone())), AppearanceItem::Stroke(crate::lighting::seam_stroke(fill, seam))]
                }
                // Gradients and patterns keep their paint, darkened by a layer of the shading color
                // and lightened by one of white. As with the other 3D effects' pattern shadow, the
                // layers cover the whole face, transparent pattern cells included.
                None => {
                    let mut items = vec![
                        AppearanceItem::Fill(FillLayer::new(paint.clone())),
                        AppearanceItem::Stroke(crate::lighting::seam_stroke(paint.clone(), seam)),
                    ];
                    for (color, share) in [(shade, 1.0 - tone), (Some(Color::WHITE), spec)] {
                        if let Some(color) = color.filter(|_| share > 0.0) {
                            let mut layer = FillLayer::new(Paint::solid(color));
                            layer.opacity = share as f32;
                            items.push(AppearanceItem::Fill(layer));
                        }
                    }
                    items
                }
            };
            Arc::new(Node::path(n.id, face.path(), Appearance { items, ..Default::default() }))
        })
        .collect();
    let mut group = Node::group(n.id, children);
    group.opacity = opacity;
    group.blend = blend;
    if let Some(mask) = mask {
        let bounds = group.visual_bounds().unwrap_or(Rect::ZERO).inflate(1.0, 1.0);
        let coverage = Node::path(n.id, vectorcraft_geom::shapes::rectangle(bounds), Appearance::basic(mask, Paint::None, 0.0));
        group.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(coverage, true)));
    }
    group
}
