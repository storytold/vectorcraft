//! Shared conversion and vector paint shading for the 3D effects' directional lights.
use serde_json::Value;
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, FillLayer};

fn parse(v: &Value) -> Option<Color> {
    let s = v.as_str()?;
    // Match the command's opaque RGB contract, before any parsing or allocation.
    (s.len() == 7 && s.starts_with('#') && s.as_bytes().iter().skip(1).all(u8::is_ascii_hexdigit)).then(|| Color::from_hex(s)).flatten()
}

pub(crate) fn light_color(p: &Value) -> [f64; 3] {
    p.get("lightColor").and_then(parse).unwrap_or(Color::WHITE).to_rgb_uncalibrated().map(f64::from)
}

pub(crate) fn validate(p: &Value) -> Result<(), String> {
    if p.get("lightColor").is_some_and(|v| parse(v).is_none()) {
        return Err("lightColor must be an opaque RGB color in #rrggbb form".into());
    }
    if let Some(list) = p.get("lights").filter(|v| !v.is_null()) {
        parse_lights(list)?;
    }
    Ok(())
}

pub(crate) fn lights(p: &Value) -> Option<vectorcraft_three_d::Lights> {
    p.get("lights").filter(|v| !v.is_null()).and_then(|v| parse_lights(v).ok())
}

fn parse_lights(v: &Value) -> Result<vectorcraft_three_d::Lights, String> {
    use vectorcraft_three_d::{Light, Lights, MAX_LIGHTS};
    let list = v.as_array().filter(|v| !v.is_empty() && v.len() <= MAX_LIGHTS).ok_or("lights must contain one to ten lights")?;
    let mut out = Vec::with_capacity(list.len());
    for item in list {
        let item = item.as_object().ok_or("each light must be an object")?;
        if item.keys().any(|key| !["color", "intensity", "rotation", "height", "softness", "behind"].contains(&key.as_str())) {
            return Err("unknown light setting: use color, intensity, rotation, height, softness or behind".into());
        }
        let defaults = Light::default();
        let number = |key: &str, default: f64, min: f64, max: f64| -> Result<f64, String> {
            match item.get(key) {
                None => Ok(default),
                Some(v) => v
                    .as_f64()
                    .filter(|n| n.is_finite() && *n >= min && *n <= max)
                    .ok_or_else(|| format!("light {key} must be a number from {min} to {max}")),
            }
        };
        let color = match item.get("color") {
            None => [1.0; 3],
            Some(v) => parse(v).ok_or("light color must use opaque #rrggbb")?.to_rgb_uncalibrated().map(f64::from),
        };
        let behind = item.get("behind").map_or(Ok(false), |v| v.as_bool().ok_or("light behind must be true or false"))?;
        out.push(Light {
            color,
            behind,
            intensity: number("intensity", defaults.intensity * 100.0, 0.0, 100.0)? / 100.0,
            rotation: number("rotation", defaults.rotation, -180.0, 180.0)?,
            height: number("height", defaults.height, 0.0, 90.0)?,
            softness: number("softness", defaults.softness * 100.0, 0.0, 100.0)? / 100.0,
        });
    }
    Lights::new(&out)
}

pub(crate) fn shade(c: Color, rgb: [f64; 3]) -> Color {
    let [r, g, b] = c.to_rgb();
    Color::rgb(r * rgb[0] as f32, g * rgb[1] as f32, b * rgb[2] as f32)
}

pub(crate) fn smooth_paint(g: vectorcraft_three_d::LightGradient, color: Color) -> Paint {
    use vectorcraft_doc::color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops = g.stops().into_iter().map(|(at, rgb)| GradientStop::new(at, shade(color, rgb))).collect();
    let mut p = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    p.geom = Some(GradientGeom { start: g.diffuse.start, end: g.diffuse.end, aspect: 1.0, focal: None });
    Paint::Gradient(Box::new(p))
}

/// Multiply two independently placed vector gradients before applying the face's coverage
/// once. Separately antialiasing the source and light along each triangle edge leaves seams.
pub(crate) fn gradient_art(
    id: vectorcraft_doc::NodeId,
    path: &vectorcraft_geom::PathData,
    paint: &Paint,
    light: vectorcraft_three_d::LightGradient,
) -> vectorcraft_doc::Node {
    composed_art(id, path, paint, Ramp::Single(light), 0.75)
}

pub(crate) fn rgb_gradient_art(
    id: vectorcraft_doc::NodeId,
    path: &vectorcraft_geom::PathData,
    paint: &Paint,
    light: vectorcraft_three_d::RgbGradient,
) -> vectorcraft_doc::Node {
    composed_art(id, path, paint, Ramp::Rgb(light), 0.75)
}

/// The narrow stroke of a facet's own `paint`, `width` wide, that closes the hairline seam between
/// it and its neighbors where each is antialiased alone. Its joins are round: a facet seen almost
/// edge-on is a sharp sliver, and a mitered seam would reach far past its tip as a thorn, where a
/// round one reaches no more than half its width past the facet. Revolve's meets() counts on a
/// stroked seam reaching no more than half its width; faces drawn as clipped groups (whose clip,
/// in `composed_art`, already outlines the seam with round joins) are measured by their box.
pub(crate) fn seam_stroke(paint: Paint, width: f64) -> vectorcraft_doc::StrokeLayer {
    let mut edge = vectorcraft_doc::StrokeLayer::new(paint, width);
    edge.join = vectorcraft_doc::LineJoin::Round;
    edge
}

/// Smooth Inflate triangles keep the seam within their narrow projected footprint.
pub(crate) fn facet_art(
    id: vectorcraft_doc::NodeId,
    path: &vectorcraft_geom::PathData,
    paint: &Paint,
    light: vectorcraft_three_d::RgbGradient,
    seam: f64,
) -> vectorcraft_doc::Node {
    let [r, g, b] = light.channels;
    if r == g && g == b {
        let light = vectorcraft_three_d::LightGradient { diffuse: r, ambient: 0.0, color: [1.0; 3] };
        if let Some(color) = paint.color() {
            let paint = smooth_paint(light, color);
            let edge = seam_stroke(paint.clone(), seam);
            return vectorcraft_doc::Node::path(
                id,
                path.clone(),
                vectorcraft_doc::Appearance {
                    items: vec![AppearanceItem::Fill(FillLayer::new(paint)), AppearanceItem::Stroke(edge)],
                    ..Default::default()
                },
            );
        }
        return composed_art(id, path, paint, Ramp::Single(light), seam);
    }
    composed_art(id, path, paint, Ramp::Rgb(light), seam)
}

/// The width of the seam stroke of a facet's underlay: a round stroke that reaches 1 pt past each
/// edge, over the pixels that the antialiased edges of two neighboring facets each cover only in
/// part at 1:1. A clip to the surface's outline removes what it paints past the surface.
pub(crate) const UNDERLAY_SEAM: f64 = 2.0;

/// The area facet `path` and its round seam stroke `seam` wide paint together: the stroke's outer
/// contours and the facet, all wound one way with a positive area. The stroke's inner contours,
/// wound the other way, are left out: the facet covers them, and contours that cancel each other
/// leave partial coverage where a union of footprints meets at a pixel.
pub(crate) fn footprint(path: &vectorcraft_geom::BezPath, seam: f64) -> vectorcraft_geom::BezPath {
    use kurbo::{PathEl, Shape};
    let stroke = kurbo::stroke(path.iter(), &kurbo::Stroke::new(seam).with_join(kurbo::Join::Round), &kurbo::StrokeOpts::default(), 0.05);
    let sign = if stroke.area() < 0.0 { -1.0 } else { 1.0 };
    let mut footprint = vectorcraft_geom::BezPath::new();
    let mut contour = vectorcraft_geom::BezPath::new();
    for el in stroke.elements().iter().chain(std::iter::once(&PathEl::MoveTo(kurbo::Point::ZERO))) {
        if matches!(el, PathEl::MoveTo(_)) && !contour.elements().is_empty() {
            if contour.area() * sign >= 0.0 {
                footprint.extend(contour.iter());
            }
            contour = vectorcraft_geom::BezPath::new();
        }
        contour.push(*el);
    }
    let mut boundary = path.clone();
    if boundary.area() * sign < 0.0 {
        boundary = boundary.reverse_subpaths();
    }
    footprint.extend(boundary.iter());
    if sign < 0.0 { footprint.reverse_subpaths() } else { footprint }
}

/// Colored-light facets `faces` (in drawing order) drawn over an underlay of themselves, the same
/// facets grown to seams [`UNDERLAY_SEAM`] wide, all clipped once to the union of the facets'
/// footprints. Where two facets' antialiased edges meet, the underlay fills what they leave
/// uncovered with the neighbors' own colors, so the background does not show through. `None` unless
/// every face is a facet a surface draws: a path filled and stroked by its seam, or a facet composed
/// in a clip of its footprint, neither with an opacity mask.
pub(crate) fn sealed(id: vectorcraft_doc::NodeId, faces: &[std::sync::Arc<vectorcraft_doc::Node>]) -> Option<vectorcraft_doc::Node> {
    use std::sync::Arc;
    use vectorcraft_doc::{Appearance, Node, NodeKind};
    let mut outline = vectorcraft_geom::BezPath::new();
    let mut underlay = Vec::with_capacity(faces.len());
    for face in faces {
        if face.extras().mask.is_some() {
            return None;
        }
        match &face.kind {
            NodeKind::Path { clipping: false, .. } => {
                let path = face.path_data()?.to_bezpath();
                let mut seam = None;
                let mut grown = (**face).clone();
                for item in &mut grown.appearance.items {
                    if let AppearanceItem::Stroke(s) = item {
                        seam = Some(seam.map_or(s.width, |w: f64| w.max(s.width)));
                        s.width = UNDERLAY_SEAM;
                    }
                }
                outline.extend(footprint(&path, seam?).iter());
                underlay.push(Arc::new(grown));
            }
            NodeKind::Group { children, clip: true } if children.len() == 2 => {
                let clip = children.first()?.path_data()?.to_bezpath();
                let wider = Node::path(id, vectorcraft_geom::PathData::from_bezpath(&footprint(&clip, UNDERLAY_SEAM)), Appearance::default());
                let colors = children.get(1)?.clone();
                outline.extend(clip.iter());
                underlay.push(Arc::new(Node::new(id, NodeKind::Group { children: vec![Arc::new(wider), colors], clip: true })));
            }
            _ => return None,
        }
    }
    let clip = Node::path(id, vectorcraft_geom::PathData::from_bezpath(&outline), Appearance::default());
    underlay.extend(faces.iter().cloned());
    Some(Node::new(id, NodeKind::Group { children: vec![Arc::new(clip), Arc::new(Node::group(id, underlay))], clip: true }))
}

/// Composite smooth solid-fill facets with one clip and three channel groups per surface. In each
/// channel, an underlay of the facets with wide seams lies under the facets themselves, so a
/// shared edge is opaque in that channel's own light before the channels are screened together;
/// the black backdrop shows only outside the surface, which the clip removes.
pub(crate) fn solid_surface(id: vectorcraft_doc::NodeId, facets: &[vectorcraft_three_d::Facet], paint: &Paint) -> vectorcraft_doc::Node {
    use kurbo::Shape;
    use std::sync::Arc;
    use vectorcraft_doc::color::BlendMode;
    use vectorcraft_doc::{Appearance, Node, NodeKind};
    use vectorcraft_geom::{BezPath, PathData, Rect, shapes};

    let mut boundary = BezPath::new();
    let mut bounds = None;
    for face in facets {
        let path = face.path();
        if let Some(b) = path.bounds() {
            bounds = Some(bounds.map_or(b, |r: Rect| r.union(b)));
        }
        let mut contour = path.to_bezpath();
        if contour.area() < 0.0 {
            contour = contour.reverse_subpaths();
        }
        boundary.extend(contour.iter());
    }
    let clip = Node::path(id, PathData::from_bezpath(&boundary), Appearance::default());
    let bounds = bounds.unwrap_or(Rect::ZERO).inflate(1.0, 1.0);
    let cover = |paint| Node::path(id, shapes::rectangle(bounds), Appearance::basic(paint, Paint::None, 0.0));
    let channels = (0..3)
        .map(|channel| {
            let mut color = [0.0; 3];
            if let Some(value) = color.get_mut(channel) {
                *value = 1.0;
            }
            let paints: Vec<Paint> = facets
                .iter()
                .map(|face| {
                    if let Some(ramp) = face.rgb_gradient {
                        smooth_paint(vectorcraft_three_d::LightGradient { diffuse: ramp.channels[channel], ambient: 0.0, color }, Color::WHITE)
                    } else {
                        Paint::solid(shade(Color::WHITE, color.map(|c| c * face.illumination[channel])))
                    }
                })
                .collect();
            let draw = |seam: fn(&vectorcraft_three_d::Facet) -> f64| -> Vec<Arc<Node>> {
                facets
                    .iter()
                    .zip(&paints)
                    .map(|(face, paint)| {
                        let edge = seam_stroke(paint.clone(), seam(face));
                        Arc::new(Node::path(
                            id,
                            face.path(),
                            Appearance {
                                items: vec![AppearanceItem::Fill(FillLayer::new(paint.clone())), AppearanceItem::Stroke(edge)],
                                ..Default::default()
                            },
                        ))
                    })
                    .collect()
            };
            let mut children = vec![Arc::new(cover(Paint::solid(Color::BLACK)))];
            children.extend(draw(|_| UNDERLAY_SEAM));
            children.extend(draw(|face| (face.thickness() / 2.0).clamp(0.05, 0.75)));
            let mut group = Node::group(id, children);
            group.isolate = true;
            group.blend = BlendMode::Screen;
            Arc::new(group)
        })
        .collect();
    let mut light = Node::group(id, channels);
    light.isolate = true;
    light.blend = BlendMode::Multiply;
    let mut colors = Node::group(id, vec![Arc::new(cover(paint.clone())), Arc::new(light)]);
    colors.isolate = true;
    Node::new(id, NodeKind::Group { children: vec![Arc::new(clip), Arc::new(colors)], clip: true })
}

enum Ramp {
    Single(vectorcraft_three_d::LightGradient),
    Rgb(vectorcraft_three_d::RgbGradient),
}

/// Separate a gradient's color and coverage so a tessellated surface can apply its opacity
/// once, instead of building opacity at the overlapping seam strokes of adjacent faces.
pub(crate) fn opaque_paint(paint: &Paint) -> (Paint, Option<Paint>) {
    let mut base = paint.clone();
    let mut mask = paint.clone();
    let mut transparent = false;
    if let (Paint::Gradient(b), Paint::Gradient(m)) = (&mut base, &mut mask) {
        b.swatch = None;
        m.swatch = None;
        for (b, m) in b.gradient.stops.iter_mut().zip(&mut m.gradient.stops) {
            transparent |= b.opacity != 1.0;
            b.opacity = 1.0;
            m.color = Color::WHITE;
            m.swatch = None;
            m.tint = 1.0;
        }
        if let (Some(b), Some(m)) = (&mut b.freeform, &mut m.freeform) {
            for (b, m) in b.points.iter_mut().zip(&mut m.points) {
                transparent |= b.opacity != 1.0;
                b.opacity = 1.0;
                m.color = Color::WHITE;
            }
        }
    }
    (base, transparent.then_some(mask))
}

fn composed_art(id: vectorcraft_doc::NodeId, path: &vectorcraft_geom::PathData, paint: &Paint, light: Ramp, seam: f64) -> vectorcraft_doc::Node {
    use std::sync::Arc;
    use vectorcraft_doc::color::BlendMode;
    use vectorcraft_doc::{Appearance, Node, NodeKind, OpacityMask};
    use vectorcraft_geom::{Rect, shapes};
    let (base, mask) = opaque_paint(paint);
    // Clip once to the union of this single-contour face and its round seam stroke.
    let clip = Node::path(id, vectorcraft_geom::PathData::from_bezpath(&footprint(&path.to_bezpath(), seam)), Appearance::default());
    // Opaque rectangles cover the complete seam stroke, without separately antialiasing
    // either ramp. Opaque sources need only vector clipping, not full-frame opacity masks. They
    // reach at least half a point past the seam stroke grown for an underlay ([`sealed`]), so
    // their own edges stay outside either clip.
    let margin = seam.max(1.0) + UNDERLAY_SEAM / 2.0;
    let bounds = path.bounds().unwrap_or(Rect::ZERO).inflate(margin, margin);
    let cover = |paint| Node::path(id, shapes::rectangle(bounds), Appearance::basic(paint, Paint::None, 0.0));
    let mut illumination = match light {
        Ramp::Single(light) => cover(smooth_paint(light, Color::WHITE)),
        Ramp::Rgb(light) => {
            let channels = light
                .channels
                .iter()
                .enumerate()
                .map(|(channel, gradient)| {
                    let mut color = [0.0; 3];
                    if let Some(value) = color.get_mut(channel) {
                        *value = 1.0;
                    }
                    let mut layer = cover(smooth_paint(vectorcraft_three_d::LightGradient { diffuse: *gradient, ambient: 0.0, color }, Color::WHITE));
                    // Screen adds disjoint R, G and B channels exactly; no light is clamped
                    // separately. Each ramp already holds the sum of all directional lights.
                    layer.blend = BlendMode::Screen;
                    Arc::new(layer)
                })
                .collect();
            let mut group = Node::group(id, channels);
            group.isolate = true;
            group
        }
    };
    illumination.blend = BlendMode::Multiply;
    let mut colors = Node::group(id, vec![Arc::new(cover(base)), Arc::new(illumination)]);
    colors.isolate = true;
    // Keep isolation inside the clip: the clip then applies to the finished colors,
    // rather than separately antialiasing each of the two gradient drawings.
    let mut face = Node::new(id, NodeKind::Group { children: vec![Arc::new(clip), Arc::new(colors)], clip: true });
    if let Some(mask) = mask {
        face.extras_mut().mask = Some(Box::new(OpacityMask::new(cover(mask), true)));
    }
    face
}

pub(crate) fn fills(paint: &Paint, rgb: [f64; 3], brightness: f64, colored: bool) -> Vec<AppearanceItem> {
    let shade = |c: Color| shade(c, rgb);
    if let Some(c) = paint.color() {
        return vec![AppearanceItem::Fill(FillLayer::new(Paint::solid(shade(c))))];
    }
    // Keep the white-light gradient treatment identical to existing documents. Colored light
    // shades the ramp itself, keeping placement, stops, midpoint and opacity as vector data.
    if matches!(paint, Paint::Gradient(_)) && colored {
        let mut shaded = paint.clone();
        vectorcraft_doc::recolor::map_paint(&mut shaded, &shade);
        if let Paint::Gradient(g) = &mut shaded {
            g.swatch = None;
        }
        return vec![AppearanceItem::Fill(FillLayer::new(shaded))];
    }
    // Pattern tiles remain referenced artwork. Retain their existing neutral shadow
    // approximation; tinting arbitrary pattern contents needs document-level evaluation.
    let mut shadow = FillLayer::new(Paint::solid(Color::BLACK));
    shadow.opacity = (1.0 - brightness) as f32;
    vec![AppearanceItem::Fill(FillLayer::new(paint.clone())), AppearanceItem::Fill(shadow)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vectorcraft_doc::color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    use vectorcraft_geom::Point;

    #[test]
    fn command_contract_is_strict_and_loaded_invalid_colors_fall_back_to_white() {
        assert_eq!(light_color(&json!({})), [1.0; 3]);
        assert_eq!(light_color(&json!({"lightColor":"#FF0000"})), [1.0, 0.0, 0.0]);
        for v in [json!("#ff0"), json!("#ffffff80"), json!("blue"), json!("#gg0000"), json!(null), json!([1, 0, 0])] {
            let p = json!({"lightColor":v});
            assert!(validate(&p).is_err(), "{p}");
            assert_eq!(light_color(&p), [1.0; 3]);
        }
    }

    #[test]
    fn malformed_loaded_lists_keep_the_legacy_light_and_valid_lists_override_it() {
        for value in [json!([]), json!(vec![json!({}); 11]), json!([false]), json!([{"height": 91}]), json!([{"color":"#ff0"}])] {
            let p = json!({"lights": value, "lightColor":"#ff0000"});
            assert!(lights(&p).is_none());
            assert_eq!(light_color(&p), [1.0, 0.0, 0.0]);
            assert!(validate(&p).is_err());
        }
        let p = json!({"lights":[{}, {"color":"#0000ff","intensity":40}], "lightColor":"#ff0000"});
        let list = lights(&p).unwrap();
        assert_eq!(list.iter().count(), 2);
        assert_eq!(*list.iter().next().unwrap(), vectorcraft_three_d::Light::default());
        assert_eq!(list.iter().nth(1).unwrap().color, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn colored_gradient_keeps_placement_alpha_and_midpoints() {
        let mut stop = GradientStop::new(0.0, Color::WHITE);
        stop.opacity = 0.4;
        stop.midpoint = 0.3;
        let g = GradientPaint {
            gradient: Gradient::new(GradientKind::Linear, vec![stop, GradientStop::new(1.0, Color::BLACK)]),
            geom: Some(GradientGeom { start: Point::new(20.0, 30.0), end: Point::new(100.0, 30.0), aspect: 1.0, focal: None }),
            angle: 0.0,
            swatch: None,
            freeform: None,
        };
        let paint = Paint::Gradient(Box::new(g.clone()));
        let items = fills(&paint, [1.0, 0.25, 0.25], 1.0, true);
        let Paint::Gradient(shaded) = items[0].paint() else { panic!("vector gradient") };
        assert_eq!(shaded.geom, g.geom);
        assert_eq!(shaded.gradient.stops[0].color.to_rgb(), [1.0, 0.25, 0.25]);
        assert_eq!(shaded.gradient.stops[0].opacity, 0.4);
        assert_eq!(shaded.gradient.stops[0].midpoint, 0.3);
        assert_eq!(shaded.gradient.stops[1].color, Color::BLACK);
        assert_eq!(paint, Paint::Gradient(Box::new(g)));
        let white = fills(&paint, [0.4; 3], 0.4, false);
        assert_eq!(white[0].paint(), &paint, "white retains the previous gradient treatment");
        assert_eq!(white.len(), 2);
        let unlit = fills(&paint, [0.25; 3], 0.25, true);
        assert_eq!(unlit.len(), 1, "neutral ambient under colored light still tints once");
        let mut freeform = if let Paint::Gradient(g) = paint { g } else { panic!("gradient") };
        freeform.gradient.kind = GradientKind::Freeform;
        freeform.gradient.stops = vec![GradientStop::new(0.0, Color::BLACK)];
        freeform.swatch = Some("Original ramp".into());
        freeform.freeform = Some(vectorcraft_doc::color::Freeform {
            points: vec![vectorcraft_doc::color::FreeformPoint::new(Point::new(12.0, 18.0), Color::WHITE)],
            ..Default::default()
        });
        let original = Paint::Gradient(freeform);
        let items = fills(&original, [1.0, 0.25, 0.25], 1.0, true);
        let Paint::Gradient(shaded) = items[0].paint() else { panic!("freeform gradient") };
        let point = shaded.freeform.as_ref().unwrap().points[0];
        assert_eq!(point.at, Point::new(12.0, 18.0));
        assert_eq!(point.color.to_rgb(), [1.0, 0.25, 0.25]);
        assert_eq!(shaded.swatch, None, "a changed freeform point detaches the gradient swatch");
    }
}
