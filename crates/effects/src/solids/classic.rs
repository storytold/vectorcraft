//! Extrude & Bevel (Classic): the solid Extrude & Bevel builds from a path, drawn with the classic
//! surfaces (Wireframe, No Shading, Diffuse and Plastic), their highlight, blend steps and shading
//! color. Rotate (Classic): the plane Rotate turns, drawn with the No Shading and Diffuse surfaces.
use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::{BlendMode, Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, FillLayer, Node};
use vectorcraft_geom::Rect;
use vectorcraft_three_d::{Bevel, BevelShape, ClassicShading, ClassicSurface, Extrude, Facet, Light};

use super::graphics::map_art::{self, MapArt, MapBudget, SolidLayout};
use crate::util::{flag, num, text};

pub const CLASSIC_EXTRUDE: &str = "threeD.classicExtrude";
pub const CLASSIC_ROTATE: &str = "threeD.classicRotate";

/// The surfaces Rotate (Classic) takes: No Shading and Diffuse.
pub const ROTATE_SURFACES: [&str; 2] = ["none", "diffuse"];

/// The width of a Wireframe edge, in points.
const WIRE: f64 = 0.5;

/// The Shading Color choices: black, a custom color, none, or black overprinting the paint.
pub const SHADING_COLORS: [&str; 4] = ["black", "custom", "none", "blackOverprint"];

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
        // The numbered surfaces are laid out only for an effect that maps art onto them.
        map_surfaces: !map_art::surface_maps(p).is_empty(),
        ..Extrude::default()
    }
}

/// The mapped art of one solid for [`surface`]: the effect's maps, the solid's place among the
/// object's numbered surfaces, and the limits the object's mapped art shares.
pub(crate) struct Mapped<'a, 'd> {
    pub(crate) art: &'a MapArt<'d>,
    pub(crate) layout: &'a SolidLayout,
    pub(crate) budget: &'a mut MapBudget,
    pub(crate) copies: &'a mut usize,
}

/// The color faces darken toward: black (for Black Overprint too), the custom color, or none (faces
/// keep their paint and only the highlight lightens them).
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

const FLAGS: [&str; 7] = ["cap", "bevelInside", "lightBehind", "drawHiddenFaces", "preserveSpotColors", map_art::MAP_SHADE, map_art::MAP_INVISIBLE];

/// Check Extrude & Bevel (Classic)'s parameters `p` (its defaults filled in): every key is one of
/// its parameters, numbers are in range, flags are true or false, `surface`, `shadingColor` and
/// `shadingCustom` name a surface, a shading color and a #rrggbb color, and `mapArt` holds valid
/// maps.
pub fn validate(p: &Value) -> Result<(), String> {
    validate_classic(CLASSIC_EXTRUDE, p, &RANGES, &FLAGS)?;
    map_art::validate(CLASSIC_EXTRUDE, p)
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
    validate_classic(CLASSIC_ROTATE, p, &ROTATE_RANGES, &["lightBehind", "preserveSpotColors"])?;
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

/// Whether Preserve Spot Colors applies with parameters `p`: `preserveSpotColors` is on and the
/// shading color is not Custom, which cannot keep a spot color.
fn preserves_spot_colors(p: &Value) -> bool {
    flag(p, "preserveSpotColors", false) && text(p, "shadingColor", "black") != "custom"
}

/// A face of `paint`, a color linked to a swatch, with Preserve Spot Colors: the swatch at a lower
/// tint for the highlight `spec`, its link kept so a spot color stays on its own plate, and, where
/// tone `tone` is below full light and the faces darken (`shade`), black over it as
/// [`overprinting_black`] draws it, so it prints on the black plate. `seam` is the width of the seam
/// stroke each layer draws around the face.
fn linked_face(paint: &Paint, tone: f64, spec: f64, shade: Option<Color>, seam: f64) -> Vec<AppearanceItem> {
    let Paint::Solid { color, swatch, tint } = paint else { return vec![] };
    let keep = (1.0 - spec).clamp(0.0, 1.0) as f32;
    let fill = Paint::Solid { color: color.tinted(keep), swatch: swatch.clone(), tint: tint * keep };
    let mut items = vec![AppearanceItem::Fill(FillLayer::new(fill.clone())), AppearanceItem::Stroke(crate::lighting::seam_stroke(fill, seam))];
    if shade.is_some() {
        items.extend(overprinting_black(tone, seam));
    }
    items
}

/// The black that shades a face at tone `tone` on the black plate, when the tone is below full
/// light: a fill of black at `1 - tone`, multiplied and overprinting, and its seam stroke `seam`
/// wide, overprinting with Darken, so where the stroke lies over the face's own black, or over a
/// neighbor's, the face is not darkened twice.
fn overprinting_black(tone: f64, seam: f64) -> Vec<AppearanceItem> {
    if tone >= 1.0 {
        return vec![];
    }
    let black = Paint::solid(Color::gray((1.0 - tone).clamp(0.0, 1.0) as f32));
    let mut layer = FillLayer::new(black.clone());
    layer.blend = BlendMode::Multiply;
    layer.overprint = true;
    let mut edge = crate::lighting::seam_stroke(black, seam);
    edge.blend = BlendMode::Darken;
    edge.overprint = true;
    vec![AppearanceItem::Fill(layer), AppearanceItem::Stroke(edge)]
}

/// Whether the shading color of parameters `p` is Black Overprint.
fn black_overprint(p: &Value) -> bool {
    text(p, "shadingColor", "black") == "blackOverprint"
}

/// A face of `paint` with Black Overprint: the paint unchanged, and where tone `tone` is below full
/// light, black over it as [`overprinting_black`] draws it, so the paint's own inks print as they
/// are and the shading prints on the black plate. The highlight is not drawn. `seam` is the width of
/// the seam stroke each layer draws around the face.
fn overprinted_face(paint: &Paint, tone: f64, seam: f64) -> Vec<AppearanceItem> {
    let mut items =
        vec![AppearanceItem::Fill(FillLayer::new(paint.clone())), AppearanceItem::Stroke(crate::lighting::seam_stroke(paint.clone(), seam))];
    items.extend(overprinting_black(tone, seam));
    items
}

/// The width of the stroke [`surface`] draws around `face` with parameters `p`: a wire edge on
/// Wireframe, else the seam between faces.
pub(crate) fn stroke_width(face: &Facet, p: &Value) -> f64 {
    if classic_shading(p).surface == ClassicSurface::Wireframe { WIRE } else { super::seam_width(face) }
}

/// One solid's faces in paint `paint`, as a group with the paint's opacity and blend mode, one child
/// for each facet: the edges of the faces on Wireframe, else each face filled in its paint shaded by
/// its tone and highlight (with Black Overprint, as [`overprinted_face`] draws it; with Preserve Spot
/// Colors, a color linked to a swatch as [`linked_face`] draws it). A gradient with transparent stops
/// is shaded opaque and then masked by its own transparency, so the shading follows the gradient's
/// coverage; without a placement of its own, it is placed once on all the faces
/// ([`crate::lighting::opaque_surface_paint`]).
///
/// With `mapped`, a face that carries mapped art is a group of the face and that art, which the
/// surface's opacity mask covers too. Shade Artwork gives the art its face's tone and highlight,
/// and Invisible Geometry leaves the faces out: a face is then a group of its art alone. Wireframe
/// draws no mapped art. Without `mapped`, `hide` leaves the faces out the same way, for a solid
/// that shares Invisible Geometry with another solid's mapped art; Wireframe still draws.
///
/// Returns the group, whether its faces were left out, and whether `mapped` art was left out past
/// the limits.
pub(crate) fn surface(
    n: &Node,
    facets: &[Facet],
    source: &Paint,
    (opacity, blend): (f32, BlendMode),
    p: &Value,
    mapped: Option<Mapped<'_, '_>>,
    hide: bool,
) -> (Node, bool, bool) {
    let shading = classic_shading(p);
    let lights = lights(p);
    let shade = shading_color(p);
    let preserve = preserves_spot_colors(p);
    let overprint = black_overprint(p);
    let wire = shading.surface == ClassicSurface::Wireframe;
    let (paint, mask) = if wire { (source.clone(), None) } else { crate::lighting::opaque_surface_paint(source, || super::facets_bounds(facets)) };
    let paint = &paint;
    // The mapped art of each facet, and whether the faces themselves are left out. Art past the
    // limits is left out, and the faces draw; the copies and the points of art left out do not
    // count against the art of later solids.
    let (mut parts, invisible, dropped) = match mapped.filter(|_| !wire) {
        Some(m) => {
            let shaded_as = |f: &Facet, c: Color| {
                let (tone, spec) = shading.tone(f.normal, &lights);
                shaded(c, tone, spec, shade)
            };
            let (mut used, mut budget) = (*m.copies, m.budget.clone());
            match m.art.parts(n.id, facets, m.layout, &shaded_as, &mut budget, &mut used) {
                Some(parts) => {
                    (*m.copies, *m.budget) = (used, budget);
                    (parts, m.art.invisible, false)
                }
                None => (vec![], false, true),
            }
        }
        None => (vec![], hide && !wire, false),
    };
    let children = facets
        .iter()
        .enumerate()
        .map(|(k, face)| {
            let art = parts.get_mut(k).map(std::mem::take).unwrap_or_default();
            if !art.is_empty() || invisible {
                let drawn = (!invisible).then(|| Arc::new(shaded_face(n, face, paint, &shading, &lights, shade, preserve, overprint)));
                return Arc::new(Node::group(n.id, drawn.into_iter().chain(art).collect()));
            }
            if wire {
                // A wire edge is drawn as a seam is: a round-joined stroke of the paint.
                let edge = crate::lighting::seam_stroke(paint.clone(), WIRE);
                return Arc::new(Node::path(n.id, face.path(), Appearance { items: vec![AppearanceItem::Stroke(edge)], ..Default::default() }));
            }
            Arc::new(shaded_face(n, face, paint, &shading, &lights, shade, preserve, overprint))
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
    (group, invisible, dropped)
}

/// One face of a solid in paint `paint` (opaque: [`surface`] masks a gradient's transparency),
/// filled in the paint shaded by the face's tone and highlight under `lights`, toward shading
/// color `shade`. With `preserve` (Preserve Spot Colors), a color linked to a swatch is drawn as
/// [`linked_face`] draws it. With `overprint` (Black Overprint), the face is drawn as
/// [`overprinted_face`] draws it.
#[allow(clippy::too_many_arguments)]
fn shaded_face(
    n: &Node,
    face: &Facet,
    paint: &Paint,
    shading: &ClassicShading,
    lights: &[([f64; 3], f64)],
    shade: Option<Color>,
    preserve: bool,
    overprint: bool,
) -> Node {
    let (tone, spec) = shading.tone(face.normal, lights);
    // A narrow stroke of the face's own paint closes the hairline seams between faces.
    let seam = super::seam_width(face);
    if overprint {
        let items = overprinted_face(paint, tone, seam);
        return Node::path(n.id, face.path(), Appearance { items, ..Default::default() });
    }
    let items = match paint.color() {
        // Unshaded (No Shading, no shading color, or a face in full light) with no
        // highlight: the paint itself, its color space and swatch kept.
        Some(_) if (tone >= 1.0 || shade.is_none()) && spec <= 0.0 => {
            vec![AppearanceItem::Fill(FillLayer::new(paint.clone())), AppearanceItem::Stroke(crate::lighting::seam_stroke(paint.clone(), seam))]
        }
        // Preserve Spot Colors: a color linked to a swatch stays a tint of that swatch.
        Some(_) if preserve && matches!(paint, Paint::Solid { swatch: Some(_), .. }) && !paint.is_registration() => {
            linked_face(paint, tone, spec, shade, seam)
        }
        Some(c) => {
            let fill = Paint::solid(shaded(c, tone, spec, shade));
            vec![AppearanceItem::Fill(FillLayer::new(fill.clone())), AppearanceItem::Stroke(crate::lighting::seam_stroke(fill, seam))]
        }
        // Gradients and patterns keep their paint, darkened by a layer of the shading color
        // and lightened by one of white. As with the other 3D effects' pattern shadow, the
        // layers cover the whole face, transparent pattern cells included.
        None => {
            let mut items =
                vec![AppearanceItem::Fill(FillLayer::new(paint.clone())), AppearanceItem::Stroke(crate::lighting::seam_stroke(paint.clone(), seam))];
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
    Node::path(n.id, face.path(), Appearance { items, ..Default::default() })
}
