//! Revolve as live, shaded vector art, shared by the canvas and every exporter.
use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Effect, FillLayer, LineJoin, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{PathData, Rect};
use vectorcraft_three_d::{Edge, Revolve};

use crate::util::{flag, num, text};
use crate::{GeomContext, apply_geometry_with, is_geometry, merged_params};

pub const REVOLVE: &str = "threeD.revolve";

fn visible(fx: &[Effect]) -> bool {
    fx.iter().any(|e| e.visible && e.id == REVOLVE)
}

pub fn has_revolve(n: &Node) -> bool {
    visible(&n.appearance.effects) || n.appearance.items.iter().any(|i| i.visible() && visible(i.effects()))
}

/// Shared document-parameter conversion for rendering and interactive preview guides.
pub fn revolve_options(p: &Value) -> Revolve {
    Revolve {
        angle: num(p, "angle", 360.0),
        offset: num(p, "offset", 0.0),
        edge: if text(p, "edge", "left") == "right" { Edge::Right } else { Edge::Left },
        rotation_x: num(p, "rotationX", 0.0),
        rotation_y: num(p, "rotationY", 0.0),
        rotation_z: num(p, "rotationZ", 0.0),
        perspective: num(p, "perspectiveAngle", 0.0),
        segments: num(p, "segments", 64.0).clamp(8.0, 128.0) as usize,
        light_rotation: num(p, "lightRotation", vectorcraft_three_d::DEFAULT_LIGHT.0),
        light_height: num(p, "lightHeight", vectorcraft_three_d::DEFAULT_LIGHT.1),
        light_behind: flag(p, "lightBehind", false),
        light_color: crate::lighting::light_color(p),
        lights: crate::lighting::lights(p),
        light_intensity: num(p, "lightIntensity", 80.0) / 100.0,
        light_softness: num(p, "lightSoftness", 0.0) / 100.0,
        ambient: num(p, "ambient", 25.0) / 100.0,
        shade: flag(p, "shade", true),
        smooth_shading: true,
        cap_ends: flag(p, "cap", true),
    }
}

/// Validate before a command mutates the document. Loading external files is still bounded
/// by the evaluator; unsupported source kinds remain unchanged when rendered.
pub fn validate_revolve(n: &Node, p: &Value) -> Result<(), String> {
    crate::lighting::validate(p)?;
    let path = profile(n).ok_or("Revolve currently requires a path or compound path. Select a profile drawn with the Pen tool.")?;
    let b = path.bounds().ok_or("Revolve requires a profile with at least two anchors")?;
    if !path.subpaths.iter().any(|sp| sp.anchors.len() >= 2) {
        return Err("Revolve requires at least two anchors".into());
    }
    if p.get("edge").is_some_and(|v| v.as_str() != Some("left") && v.as_str() != Some("right")) {
        return Err("Revolve edge must be left or right".into());
    }
    if p.get("cap").is_some_and(|v| v.as_bool().is_none()) {
        return Err("Revolve cap must be a boolean".into());
    }
    vectorcraft_three_d::revolve(&path, b, revolve_options(p)).map(|_| ())
}

fn profile(n: &Node) -> Option<PathData> {
    match &n.kind {
        NodeKind::Path { path, guide: false, clipping: false, .. } => Some(path.clone()),
        NodeKind::Compound { children, .. } => {
            Some(PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect()))
        }
        _ => None,
    }
}

fn geometry(path: &PathData, fx: &[Effect], ctx: &GeomContext) -> PathData {
    apply_geometry_with(fx, path, path.bounds().unwrap_or(Rect::ZERO), ctx)
}

#[allow(clippy::too_many_arguments)]
fn surface(
    n: &Node,
    path: &PathData,
    fx: &Effect,
    paint: Paint,
    opacity: f32,
    blend: vectorcraft_doc::color::BlendMode,
    expand: bool,
) -> Option<Node> {
    let b = path.bounds()?;
    // A loaded file or a profile edited past the limits keeps drawing its source path instead
    // of crashing the frame. Commands validate the same evaluator and report its errors.
    let options = revolve_options(&merged_params(REVOLVE, &fx.params));
    let colored = options.light_color != [1.0; 3] || options.lights.is_some();
    let faces = vectorcraft_three_d::revolve(path, b, options).ok()?;
    // Visibility is an expansion option, never a change to live geometry or export baking.
    // Keep all faces for paints whose coverage we cannot prove opaque. A bounded visibility
    // failure also retains the complete surface rather than losing any of the user's art.
    let visible = (expand
        && flag(&fx.params, "expandVisibleOnly", true)
        && paint.color().is_some()
        && opacity == 1.0
        && blend == Default::default()
        && n.has_default_transparency())
    .then(|| vectorcraft_three_d::visible_faces(&faces).ok())
    .flatten();
    let faces = visible.unwrap_or_else(|| {
        faces
            .into_iter()
            .map(|f| vectorcraft_three_d::VisibleFace {
                path: f.path(),
                brightness: f.brightness,
                depth: f.depth,
                gradient: f.gradient,
                illumination: f.illumination,
                light_gradient: f.light_gradient,
                rgb_gradient: f.rgb_gradient,
            })
            .collect()
    });
    let children = faces
        .into_iter()
        .map(|face| {
            if let Some(g) = face.rgb_gradient
                && (paint.color().is_some() || matches!(paint, Paint::Gradient(_)))
            {
                return Arc::new(crate::lighting::rgb_gradient_art(n.id, &face.path, &paint, g));
            }
            if let (true, Paint::Gradient(_), Some(g)) = (colored, &paint, face.light_gradient) {
                return Arc::new(crate::lighting::gradient_art(n.id, &face.path, &paint, g));
            }
            let mut fill = FillLayer::new(paint.clone());
            let mut items = vec![];
            if let Some(c) = paint.color() {
                let [r, g, b] = c.to_rgb();
                let color = |s: f64| Color::rgb(r * s as f32, g * s as f32, b * s as f32);
                fill.paint = if colored {
                    face.light_gradient
                        .map_or_else(|| Paint::solid(crate::lighting::shade(c, face.illumination)), |g| crate::lighting::smooth_paint(g, c))
                } else {
                    face.gradient.map_or_else(|| Paint::solid(color(face.brightness)), |g| shading_paint(g, color, |_| 1.0))
                };
                items.push(AppearanceItem::Fill(fill));
            } else if colored && matches!(paint, Paint::Gradient(_)) {
                items = crate::lighting::fills(&paint, face.illumination, face.brightness, true);
            } else {
                items.push(AppearanceItem::Fill(fill));
                let mut shadow = FillLayer::new(Paint::solid(Color::BLACK));
                if let Some(g) = face.gradient {
                    shadow.paint = shading_paint(g, |_| Color::BLACK, |s| (1.0 - s) as f32);
                } else {
                    shadow.opacity = (1.0 - face.brightness) as f32;
                }
                items.push(AppearanceItem::Fill(shadow));
            }
            // Independently antialiased adjacent polygons otherwise leave a hairline grid.
            // Matching narrow edge paint overlaps their coverage in the canvas and exporters.
            // Keep opacity on the surface group so these edges never accumulate transparency.
            let items = items
                .into_iter()
                .flat_map(|item| {
                    let AppearanceItem::Fill(fill) = &item else { return vec![item] };
                    let mut edge = StrokeLayer::new(fill.paint.clone(), 0.75);
                    // Acute face corners must stay within the overlap width. Miter joins
                    // amplify this seam stroke into visible spikes along the silhouette.
                    edge.join = LineJoin::Round;
                    edge.opacity = fill.opacity;
                    vec![item, AppearanceItem::Stroke(edge)]
                })
                .collect();
            Arc::new(Node::path(n.id, face.path, Appearance { items, ..Default::default() }))
        })
        .collect();
    let mut group = Node::group(n.id, children);
    group.opacity = opacity;
    group.blend = blend;
    Some(group)
}

fn shading_paint(g: vectorcraft_three_d::ShadeGradient, color: impl Fn(f64) -> Color, opacity: impl Fn(f64) -> f32) -> Paint {
    use vectorcraft_doc::color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops =
        g.brightness.into_iter().enumerate().map(|(i, s)| GradientStop { opacity: opacity(s), ..GradientStop::new(i as f32, color(s)) }).collect();
    let mut paint = GradientPaint::new(Gradient { kind: GradientKind::Linear, stops });
    paint.geom = Some(GradientGeom { start: g.start, end: g.end, aspect: 1.0, focal: None });
    Paint::Gradient(Box::new(paint))
}

fn paint_of(item: &AppearanceItem) -> (Paint, f32, vectorcraft_doc::color::BlendMode) {
    match item {
        AppearanceItem::Fill(f) => (f.paint.clone(), f.opacity, f.blend),
        AppearanceItem::Stroke(s) => (s.paint.clone(), s.opacity, s.blend),
    }
}

/// Evaluated art keeps the source's identity and transparency. Each item's opacity belongs
/// to its entire surface, not to each face, so overlapping faces don't accumulate alpha.
pub fn revolve_art(n: &Node) -> Option<Node> {
    art(n, false)
}

/// Revolve's own Expand Appearance path. The shared command and UI remain unchanged.
pub(crate) fn expand_revolve_art(n: &Node) -> Option<Node> {
    art(n, true)
}

fn art(n: &Node, expand: bool) -> Option<Node> {
    if !has_revolve(n) {
        return None;
    }
    let path = profile(n)?;
    let ctx = GeomContext::of(n);
    let mut children = vec![];
    let mut remaining = n.appearance.effects.clone();
    if let Some(index) = n.appearance.effects.iter().position(|e| e.visible && e.id == REVOLVE) {
        let fx = n.appearance.effects.get(index)?;
        let path = geometry(&path, n.appearance.effects.get(..index)?, &ctx);
        // An object's surface uses its top visible fill, falling back to its stroke.
        let item = n
            .appearance
            .items
            .iter()
            .rev()
            .find(|i| matches!(i, AppearanceItem::Fill(_)) && crate::paints(i))
            .or_else(|| n.appearance.items.iter().rev().find(|i| crate::paints(i)))?;
        let (paint, opacity, blend) = paint_of(item);
        let expand = expand && !n.appearance.effects.iter().skip(index + 1).any(|e| e.visible && is_geometry(&e.id));
        children.push(Arc::new(surface(n, &path, fx, paint, opacity, blend, expand)?));
        // Later geometry acts on the projected art as a whole through container evaluation.
        remaining = n
            .appearance
            .effects
            .iter()
            .enumerate()
            .filter(|(i, e)| e.id != REVOLVE && (!is_geometry(&e.id) || *i > index))
            .map(|(_, e)| e.clone())
            .collect();
    } else {
        let path = geometry(&path, &n.appearance.effects, &ctx);
        remaining.retain(|e| !is_geometry(&e.id));
        for item in n.appearance.items.iter().filter(|i| crate::paints(i)) {
            if let Some(index) = item.effects().iter().position(|e| e.visible && e.id == REVOLVE) {
                let fx = item.effects().get(index)?;
                let p = geometry(&path, item.effects().get(..index)?, &ctx.item(item));
                let (paint, opacity, blend) = paint_of(item);
                let expand = expand && !item.effects().iter().skip(index + 1).any(|e| e.visible && is_geometry(&e.id));
                let mut art = surface(n, &p, fx, paint, opacity, blend, expand)?;
                art.appearance.effects = item.effects().iter().skip(index + 1).filter(|e| e.id != REVOLVE).cloned().collect();
                children.push(Arc::new(art));
            } else {
                children.push(Arc::new(Node::path(n.id, path.clone(), Appearance { items: vec![item.clone()], ..Default::default() })));
            }
        }
    }
    let mut art = n.clone();
    art.kind = NodeKind::Group { children, clip: false };
    art.appearance = Appearance { effects: remaining, ..Default::default() }.into();
    Some(art)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vectorcraft_doc::{Document, NodeId};
    use vectorcraft_geom::{Affine, BezPath};
    fn profile_node() -> Node {
        let mut b = BezPath::new();
        b.move_to((100.0, 40.0));
        b.line_to((130.0, 60.0));
        b.line_to((130.0, 140.0));
        b.line_to((100.0, 160.0));
        Node::path(NodeId(1), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(Color::rgb(0.8, 0.2, 0.1)), Paint::None, 0.0))
    }
    fn fx() -> Effect {
        crate::new_effect(REVOLVE, &json!({})).unwrap()
    }
    #[test]
    fn seam_strokes_stay_within_half_a_stroke_width_of_each_face() {
        use kurbo::Shape;
        for shade in [false, true] {
            for angle in [110.0, 360.0] {
                let mut n = profile_node();
                n.appearance.effects.push(
                    crate::new_effect(
                        REVOLVE,
                        &json!({
                            "offset":20,"rotationX":-18,"segments":24,"cap":true,
                            "shade":shade,"angle":angle
                        }),
                    )
                    .unwrap(),
                );
                for art in [revolve_art(&n).unwrap(), expand_revolve_art(&n).unwrap()] {
                    let mut checked = 0;
                    art.walk(&mut |face| {
                        let Some(path) = face.path_data() else { return };
                        for item in &face.appearance.items {
                            let AppearanceItem::Stroke(stroke) = item else { continue };
                            let bounds = path.bounds().unwrap().inflate(stroke.width / 2.0 + 0.005, stroke.width / 2.0 + 0.005);
                            let ink = crate::stroke::line_outline(&path.to_bezpath(), stroke, stroke.width, 0.001).bounding_box();
                            assert!(
                                bounds.contains_rect(ink),
                                "seam protrudes beyond its overlap: {ink:?}, {bounds:?}, shade {shade}, sweep {angle}"
                            );
                            checked += 1;
                        }
                    });
                    assert!(checked > 20);
                }
            }
        }
    }
    #[test]
    fn smooth_shading_is_automatic_and_non_solid_paints_keep_their_base_paint() {
        let mut n = profile_node();
        n.appearance.effects.push(fx());
        assert!(!n.appearance.effects[0].params.as_object().unwrap().contains_key("smoothShading"));
        let smooth = revolve_art(&n).unwrap();
        assert!(
            smooth.children().unwrap()[0].children().unwrap().iter().any(|face| {
                matches!(face.appearance.items.first(), Some(AppearanceItem::Fill(fill)) if matches!(fill.paint, Paint::Gradient(_)))
            })
        );
        n.appearance.effects[0].params["smoothShading"] = json!(false);
        assert_eq!(revolve_art(&n), Some(smooth.clone()));
        n.appearance.effects[0].params["smoothShading"] = json!(true);
        assert_eq!(revolve_art(&n), Some(smooth));
        use vectorcraft_doc::color::{Gradient, GradientPaint};
        let base = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
        n.appearance.set_fill(base.clone());
        let art = revolve_art(&n).unwrap();
        let faces = art.children().unwrap()[0].children().unwrap();
        assert!(faces.iter().all(|f| matches!(f.appearance.items.first(), Some(AppearanceItem::Fill(fill)) if fill.paint == base)));
        assert!(faces.iter().any(|f| f.appearance.items.iter().any(|item| {
            matches!(item, AppearanceItem::Fill(fill) if matches!(&fill.paint, Paint::Gradient(g) if g.gradient.stops.iter().all(|s| s.color == Color::BLACK) && g.gradient.stops[0].opacity != g.gradient.stops[1].opacity))
        })));
        assert_eq!(expand_revolve_art(&n), Some(art), "unknown paint opacity keeps complete surfaces");
    }
    #[test]
    fn missing_cap_parameter_defaults_on_and_explicit_false_restores_open_ends() {
        let mut n = profile_node();
        let mut effect = fx();
        effect.params["offset"] = json!(20);
        effect.params["rotationX"] = json!(-35);
        effect.params.as_object_mut().unwrap().remove("cap");
        n.appearance.effects.push(effect);
        let capped = revolve_art(&n).unwrap();
        n.appearance.effects[0].params["cap"] = json!(false);
        let uncapped = revolve_art(&n).unwrap();
        assert_ne!(uncapped, capped);
        n.appearance.effects[0].params["cap"] = json!(true);
        assert_eq!(revolve_art(&n), Some(capped));
        n.appearance.effects[0].params["cap"] = json!(false);
        assert_eq!(revolve_art(&n), Some(uncapped));
    }
    #[test]
    fn source_stays_editable_surface_moves_and_hidden_effect_restores_it() {
        let mut n = profile_node();
        let before = n.path_data().unwrap().clone();
        n.appearance.effects.push(fx());
        let a = revolve_art(&n).unwrap();
        assert!(!a.children().unwrap().is_empty());
        assert_eq!(n.path_data().unwrap(), &before);
        let b = a.visual_bounds().unwrap();
        n.transform(Affine::translate((80.0, 20.0)), false);
        let moved = revolve_art(&n).unwrap().visual_bounds().unwrap();
        assert!((moved.x0 - b.x0 - 80.0).abs() < 1e-6 && (moved.y0 - b.y0 - 20.0).abs() < 1e-6);
        n.appearance.effects[0].visible = false;
        assert!(revolve_art(&n).is_none());
    }
    #[test]
    fn item_effect_and_transparency_are_scoped_to_whole_surface() {
        let mut n = profile_node();
        n.opacity = 0.6;
        n.appearance.items[0].effects_mut().push(fx());
        if let AppearanceItem::Fill(f) = &mut n.appearance.items[0] {
            f.opacity = 0.4;
        }
        let a = revolve_art(&n).unwrap();
        assert_eq!(a.opacity, 0.6);
        let surface = &a.children().unwrap()[0];
        assert_eq!(surface.opacity, 0.4);
        assert!(surface.children().unwrap().iter().all(|c| c.opacity == 1.0 && !has_revolve(c)));
    }
    #[test]
    fn export_bakes_faces_with_unique_ids_and_no_revolve_effects() {
        let mut d = Document::new(300.0, 300.0);
        let mut n = profile_node();
        n.id = d.alloc_id();
        n.appearance.effects.push(fx());
        let mut stroke = |_: &mut Document, _: &PathData, _: vectorcraft_geom::FillRule, _: &vectorcraft_doc::StrokeLayer| None;
        let art = crate::expand_leaf(&mut d, &n, &mut stroke).unwrap();
        let mut ids = std::collections::HashSet::new();
        fn visit(n: &Node, ids: &mut std::collections::HashSet<NodeId>) {
            assert!(ids.insert(n.id));
            assert!(!has_revolve(n));
            for c in n.children().into_iter().flatten() {
                visit(c, ids);
            }
        }
        visit(&art, &mut ids);
        assert!(ids.len() > 50);
    }

    #[test]
    fn visible_surface_option_only_affects_expansion_and_can_keep_all_faces() {
        let mut n = profile_node();
        n.appearance.effects.push(fx());
        let source = n.path_data().unwrap().clone();
        let live = revolve_art(&n).unwrap();
        let expanded = expand_revolve_art(&n).unwrap();
        let count = |n: &Node| n.children().unwrap()[0].children().unwrap().len();
        assert!(count(&expanded) < count(&live) * 3 / 4, "hidden back surfaces are removed");
        assert_eq!(crate::bake_appearance(&n), Some(live.clone()), "other baking retains the full surface");
        n.appearance.effects[0].params["expandVisibleOnly"] = json!(false);
        assert_eq!(revolve_art(&n), Some(live.clone()), "the live view ignores the expansion setting");
        assert_eq!(expand_revolve_art(&n), Some(live));
        assert_eq!(n.path_data(), Some(&source));
        n.appearance.effects[0].params["expandVisibleOnly"] = json!(true);
        assert_eq!(expand_revolve_art(&n), Some(expanded.clone()));
        // Per-item Revolve uses the same option, without trimming another item's artwork.
        let effect = n.appearance.effects.remove(0);
        n.appearance.items[0].effects_mut().push(effect);
        assert_eq!(expand_revolve_art(&n).unwrap().children().unwrap()[0], expanded.children().unwrap()[0]);
    }

    #[test]
    fn transparency_unknown_paints_and_later_distortion_keep_complete_surfaces() {
        let mut n = profile_node();
        n.appearance.effects.push(fx());
        for change in 0..4 {
            let mut sample = n.clone();
            match change {
                0 => sample.opacity = 0.5,
                1 => {
                    if let AppearanceItem::Fill(f) = &mut sample.appearance.items[0] {
                        f.opacity = 0.4;
                    }
                }
                2 => sample.blend = vectorcraft_doc::color::BlendMode::Multiply,
                _ => sample.appearance.effects.push(crate::new_effect("distort.twist", &json!({"angle":45})).unwrap()),
            }
            assert_eq!(expand_revolve_art(&sample), revolve_art(&sample), "preserve surface for case {change}");
        }
        let mut sample = n;
        if let AppearanceItem::Fill(f) = &mut sample.appearance.items[0] {
            f.paint = Paint::Pattern { pattern: "unknown transparency".into(), xf: Affine::IDENTITY };
        }
        assert_eq!(expand_revolve_art(&sample), revolve_art(&sample));
    }
}
