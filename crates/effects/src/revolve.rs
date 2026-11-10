//! Revolve as live, shaded vector art, shared by the canvas and every exporter.
use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Effect, FillLayer, LineJoin, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{PathData, Rect};
use vectorcraft_three_d::{Edge, Revolve};

use crate::solids::{Profile, is_container, members, object_profile, profile, reshape_members, solid_paints, union_bounds};
use crate::util::{flag, num, text};
use crate::{GeomContext, apply_geometry_with, is_geometry, merged_params};

pub const REVOLVE: &str = "threeD.revolve";

/// Most bytes the copies of the object's paint in the faces may take together. Each face and the
/// seam stroke around it gets a copy of the paint, so a gradient of many stops times tens of
/// thousands of faces took gigabytes; past this the source path is drawn.
const MAX_PAINT_COPIES: usize = 256 << 20;

/// About the bytes one copy of `paint` takes.
fn paint_bytes(paint: &Paint) -> usize {
    use std::mem::size_of;
    use vectorcraft_doc::color::{FreeformPoint, GradientStop};
    let own = size_of::<Paint>();
    let Paint::Gradient(g) = paint else { return own };
    let freeform = g
        .freeform
        .as_ref()
        .map_or(0, |f| f.points.len() * size_of::<FreeformPoint>() + f.lines.iter().map(|l| l.len() * size_of::<usize>()).sum::<usize>());
    own + size_of::<vectorcraft_doc::color::GradientPaint>() + g.gradient.stops.len() * size_of::<GradientStop>() + freeform
}

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
        curve_samples: None,
    }
}

/// Validate before a command mutates the document. Loading external files is still bounded
/// by the evaluator; unsupported source kinds remain unchanged when rendered.
pub fn validate_revolve(n: &Node, p: &Value) -> Result<(), String> {
    validate_revolve_params(p)?;
    let effects = &n.appearance.effects;
    let before = effects.iter().position(|e| e.visible && e.id == REVOLVE).and_then(|i| effects.get(..i)).unwrap_or(effects);
    if is_container(n) {
        const EMPTY: &str = "Revolve requires a group holding a shape with at least two anchors";
        let mut shapes = members(n, "Revolve")?;
        if !shapes.iter().any(|s| has_two_anchors(&s.path)) {
            return Err(EMPTY.into());
        }
        reshape_members(n, before, &mut shapes);
        let b = union_bounds(&shapes).ok_or(EMPTY)?;
        let options = shared_options(&shapes, revolve_options(p))?;
        let mut copies = 0;
        return shapes.iter().try_for_each(|s| check_surface(&s.path, b, options, surface_item(&s.items), &mut copies));
    }
    let (path, _) = object_profile(n, "Revolve")?;
    if !has_two_anchors(&path) {
        return Err("Revolve requires a shape with at least two anchors".into());
    }
    let path = geometry(&path, before, &GeomContext::of(n));
    let b = path.bounds().ok_or("Revolve requires a shape with at least two anchors")?;
    let paints = solid_paints(n);
    check_surface(&path, b, revolve_options(p), surface_item(&paints), &mut 0)
}

/// [`validate_revolve`] for a Revolve with parameters `p` at index `index` of the effects of
/// appearance item `target` of `n`. The surfaces of every fill and stroke that carries a Revolve,
/// `target`'s included, share one budget of paint copies, as in [`art`]. A visible object-level
/// Revolve draws in place of those surfaces.
pub(crate) fn validate_item_revolve(n: &Node, p: &Value, target: usize, index: usize) -> Result<(), String> {
    if is_container(n) || visible(&n.appearance.effects) || target >= n.appearance.items.len() {
        return validate_revolve(n, p);
    }
    validate_revolve_params(p)?;
    const EMPTY: &str = "Revolve requires a shape with at least two anchors";
    let (path, _) = object_profile(n, "Revolve")?;
    if !has_two_anchors(&path) {
        return Err(EMPTY.into());
    }
    let ctx = GeomContext::of(n);
    let path = geometry(&path, &n.appearance.effects, &ctx);
    let mut copies = 0;
    for (i, item) in n.appearance.items.iter().enumerate() {
        let effects = item.effects();
        let at = if i == target { Some(index) } else { effects.iter().position(|e| e.visible && e.id == REVOLVE) };
        let options = match at.and_then(|at| effects.get(at)) {
            _ if i == target => revolve_options(p),
            Some(fx) if crate::paints(item) => revolve_options(&merged_params(REVOLVE, &fx.params)),
            _ => continue,
        };
        let path = geometry(&path, at.and_then(|at| effects.get(..at)).unwrap_or(effects), &ctx.item(item));
        let b = path.bounds().ok_or(EMPTY)?;
        check_surface(&path, b, options, crate::paints(item).then_some(item), &mut copies)?;
    }
    Ok(())
}

fn has_two_anchors(path: &PathData) -> bool {
    path.subpaths.iter().any(|sp| sp.anchors.len() >= 2)
}

/// The bytes the copies of `paint` take on `faces` faces: a face's fill, its shadow and their seam
/// strokes make up to four copies.
fn paint_copies(faces: usize, paint: &Paint) -> usize {
    faces.saturating_mul(4).saturating_mul(paint_bytes(paint))
}

/// An error when `path` does not revolve or when its surface in `item`'s paint takes `copies`, the
/// bytes of an object's paint copies so far, past [`MAX_PAINT_COPIES`].
fn check_surface(path: &PathData, b: Rect, options: Revolve, item: Option<&AppearanceItem>, copies: &mut usize) -> Result<(), String> {
    let faces = vectorcraft_three_d::revolve(path, b, options)?;
    *copies = copies.saturating_add(item.map_or(0, |i| paint_copies(faces.len(), &paint_of(i).0)));
    if *copies > MAX_PAINT_COPIES {
        return Err("Revolve paint is too complex for this surface; reduce Segments or simplify the gradient".into());
    }
    Ok(())
}

/// `options` with one number of samples for every curve of `shapes`, the members of one group: the
/// members share Revolve's limits.
fn shared_options(shapes: &[Profile], options: Revolve) -> Result<Revolve, String> {
    let samples = vectorcraft_three_d::revolve_curve_samples(shapes.iter().map(|s| &s.path), options)?;
    Ok(Revolve { curve_samples: Some(samples), ..options })
}

/// The item whose paint an object's surface takes: its top fill, else its top stroke.
fn surface_item(items: &[AppearanceItem]) -> Option<&AppearanceItem> {
    items.iter().rev().find(|i| matches!(i, AppearanceItem::Fill(_))).or_else(|| items.last())
}

/// Check Revolve's parameter values `p` alone: its lights, `edge` and `cap`.
pub(crate) fn validate_revolve_params(p: &Value) -> Result<(), String> {
    crate::lighting::validate(p)?;
    if p.get("edge").is_some_and(|v| v.as_str() != Some("left") && v.as_str() != Some("right")) {
        return Err("Revolve edge must be left or right".into());
    }
    if p.get("cap").is_some_and(|v| v.as_bool().is_none()) {
        return Err("Revolve cap must be a boolean".into());
    }
    Ok(())
}

fn geometry(path: &PathData, fx: &[Effect], ctx: &GeomContext) -> PathData {
    apply_geometry_with(fx, path, path.bounds().unwrap_or(Rect::ZERO), ctx)
}

/// One object's surface of revolution about the axis of `b`, and each face's depth. `copies` counts
/// the bytes of the paint copies of every surface of the object.
#[allow(clippy::too_many_arguments)]
fn surface(
    n: &Node,
    path: &PathData,
    b: Rect,
    options: Revolve,
    fx: &Effect,
    paint: Paint,
    opacity: f32,
    blend: vectorcraft_doc::color::BlendMode,
    expand: bool,
    copies: &mut usize,
) -> Option<(Vec<f64>, Node)> {
    // A loaded file or a profile edited past the limits keeps drawing its source path instead
    // of crashing the frame. Commands validate the same evaluator and report its errors.
    let colored = options.light_color != [1.0; 3] || options.lights.is_some();
    let faces = vectorcraft_three_d::revolve(path, b, options).ok()?;
    *copies = copies.saturating_add(paint_copies(faces.len(), &paint));
    if *copies > MAX_PAINT_COPIES {
        return None;
    }
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
    let depths: Vec<f64> = faces.iter().map(|f: &vectorcraft_three_d::VisibleFace| f.depth).collect();
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
    Some((depths, group))
}

/// A surface of one object about the bounds of `path`, built with options derived from `fx`.
/// `copies` is the count [`surface`] adds to.
fn object_surface(n: &Node, path: &PathData, fx: &Effect, item: &AppearanceItem, expand: bool, copies: &mut usize) -> Option<Node> {
    let b = path.bounds()?;
    let (paint, opacity, blend) = paint_of(item);
    let options = revolve_options(&merged_params(REVOLVE, &fx.params));
    surface(n, path, b, options, fx, paint, opacity, blend, expand, copies).map(|(_, art)| art)
}

fn shading_paint(g: vectorcraft_three_d::ShadeGradient, color: impl Fn(f64) -> Color, opacity: impl Fn(f64) -> f32) -> Paint {
    use vectorcraft_doc::color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    let stops =
        g.brightness.into_iter().enumerate().map(|(i, s)| GradientStop { opacity: opacity(s), ..GradientStop::new(i as f32, color(s)) }).collect();
    let mut paint = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
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

/// The surfaces [`revolve_art`] draws, as meshes ([`crate::three_d_meshes`]): with the effect on
/// the object, one in its top visible fill (else its stroke); with it on fills and strokes, one for
/// each of them. Each mesh adds its triangles to `total` as it is built, and the first that takes
/// `total` past `limit` stops with an error before the next is built.
pub(crate) fn revolve_meshes(n: &Node, total: &mut usize, limit: usize) -> Result<Vec<crate::SolidMesh>, String> {
    if is_container(n) {
        return group_meshes(n, total, limit);
    }
    let (path, rule) = object_profile(n, "Revolve")?;
    let ctx = GeomContext::of(n);
    let mesh = |path: &PathData, fx: &Effect, item: &AppearanceItem| -> Result<crate::SolidMesh, String> {
        let b = path.bounds().ok_or("Revolve requires a shape with at least two anchors")?;
        let (paint, opacity, _) = paint_of(item);
        let mesh = vectorcraft_three_d::revolve_mesh(path, rule, b, revolve_options(&merged_params(REVOLVE, &fx.params)))?;
        Ok(crate::SolidMesh { mesh, color: crate::solids::mesh_color(&paint, opacity) })
    };
    if let Some(index) = n.appearance.effects.iter().position(|e| e.visible && e.id == REVOLVE) {
        let before = n.appearance.effects.get(..index).unwrap_or_default();
        let Some(fx) = n.appearance.effects.get(index) else { return Ok(vec![]) };
        let path = geometry(&path, before, &ctx);
        let paints = solid_paints(n);
        let item = surface_item(&paints).ok_or("the revolved object has no fill or stroke")?;
        let mut out = vec![];
        crate::solids::keep_mesh(&mut out, total, mesh(&path, fx, item)?, limit)?;
        return Ok(out);
    }
    let path = geometry(&path, &n.appearance.effects, &ctx);
    let mut out = vec![];
    for item in n.appearance.items.iter().filter(|i| crate::paints(i)) {
        if let Some(index) = item.effects().iter().position(|e| e.visible && e.id == REVOLVE)
            && let Some(fx) = item.effects().get(index)
        {
            let p = geometry(&path, item.effects().get(..index).unwrap_or_default(), &ctx.item(item));
            crate::solids::keep_mesh(&mut out, total, mesh(&p, fx, item)?, limit)?;
        }
    }
    Ok(out)
}

/// [`revolve_meshes`] of group or layer `n`: one mesh for each member, about one shared axis.
fn group_meshes(n: &Node, total: &mut usize, limit: usize) -> Result<Vec<crate::SolidMesh>, String> {
    let index = n
        .appearance
        .effects
        .iter()
        .position(|e| e.visible && e.id == REVOLVE)
        .ok_or("a 3D effect on a group's fill or stroke draws nothing to export")?;
    let Some(fx) = n.appearance.effects.get(index) else { return Ok(vec![]) };
    let mut shapes = members(n, "Revolve")?;
    reshape_members(n, n.appearance.effects.get(..index).unwrap_or_default(), &mut shapes);
    let b = union_bounds(&shapes).ok_or("Revolve requires a group holding a shape with at least two anchors")?;
    let options = shared_options(&shapes, revolve_options(&merged_params(REVOLVE, &fx.params)))?;
    let mut out = vec![];
    for s in &shapes {
        let Some(item) = surface_item(&s.items) else { continue };
        let (paint, opacity, _) = paint_of(item);
        let mesh = vectorcraft_three_d::revolve_mesh(&s.path, s.rule, b, options)?;
        crate::solids::keep_mesh(&mut out, total, crate::SolidMesh { mesh, color: crate::solids::mesh_color(&paint, opacity * s.opacity) }, limit)?;
    }
    Ok(out)
}

fn art(n: &Node, expand: bool) -> Option<Node> {
    if !has_revolve(n) {
        return None;
    }
    if is_container(n) {
        return group_art(n, expand);
    }
    let (path, _) = profile(n)?;
    let ctx = GeomContext::of(n);
    let mut children = vec![];
    let mut copies = 0;
    let mut remaining = n.appearance.effects.clone();
    if let Some(index) = n.appearance.effects.iter().position(|e| e.visible && e.id == REVOLVE) {
        let fx = n.appearance.effects.get(index)?;
        let path = geometry(&path, n.appearance.effects.get(..index)?, &ctx);
        // An object's surface uses its top visible fill, falling back to its stroke.
        let paints = solid_paints(n);
        let item = surface_item(&paints)?;
        let expand = expand && !n.appearance.effects.iter().skip(index + 1).any(|e| e.visible && is_geometry(&e.id));
        children.push(Arc::new(object_surface(n, &path, fx, item, expand, &mut copies)?));
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
                let expand = expand && !item.effects().iter().skip(index + 1).any(|e| e.visible && is_geometry(&e.id));
                let mut art = object_surface(n, &p, fx, item, expand, &mut copies)?;
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

/// [`art`] of group or layer `n`: every member swept about one axis on the bounds of all of them,
/// each in its own paint. Opaque members' faces are drawn in one back-to-front order. `None` when
/// the effect is only on the group's own fills or strokes or when its members can't be revolved.
fn group_art(n: &Node, expand: bool) -> Option<Node> {
    let index = n.appearance.effects.iter().position(|e| e.visible && e.id == REVOLVE)?;
    let fx = n.appearance.effects.get(index)?;
    let mut shapes = members(n, "").ok()?;
    reshape_members(n, n.appearance.effects.get(..index)?, &mut shapes);
    let b = union_bounds(&shapes)?;
    let options = shared_options(&shapes, revolve_options(&merged_params(REVOLVE, &fx.params))).ok()?;
    let expand = expand && !n.appearance.effects.iter().skip(index + 1).any(|e| e.visible && is_geometry(&e.id));
    let mut copies = 0;
    let mut surfaces = vec![];
    let mut opaque = true;
    for s in &shapes {
        let Some(item) = surface_item(&s.items) else { continue };
        let (paint, opacity, blend) = paint_of(item);
        let opacity = opacity * s.opacity;
        opaque &= opacity >= 1.0 && blend == Default::default();
        surfaces.push(surface(n, &s.path, b, options, fx, paint, opacity, blend, expand, &mut copies)?);
    }
    if surfaces.is_empty() {
        return None;
    }
    let plain = surfaces.iter().all(|(depths, s)| matches!(&s.kind, NodeKind::Group { children, clip: false } if children.len() == depths.len()));
    let children = if opaque && plain {
        let mut faces = vec![];
        for (depths, s) in surfaces {
            if let NodeKind::Group { children, .. } = s.kind {
                faces.extend(depths.into_iter().zip(children));
            }
        }
        // A stable sort: faces at the same depth keep the members' order.
        faces.sort_by(|a, b| a.0.total_cmp(&b.0));
        vec![Arc::new(Node::group(n.id, faces.into_iter().map(|(_, face)| face).collect()))]
    } else {
        surfaces.into_iter().map(|(_, s)| Arc::new(s)).collect()
    };
    let remaining =
        n.appearance.effects.iter().enumerate().filter(|(i, e)| *i != index && (!is_geometry(&e.id) || *i > index)).map(|(_, e)| e.clone()).collect();
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
        assert_eq!(crate::bake_appearance(None, &n), Some(live.clone()), "other baking retains the full surface");
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

    /// A closed rectangle `x` to `x + 10` across, filled solid.
    fn member(id: u64, x: f64) -> Arc<Node> {
        let mut b = BezPath::new();
        b.move_to((x, 0.0));
        b.line_to((x + 10.0, 0.0));
        b.line_to((x + 10.0, 40.0));
        b.line_to((x, 40.0));
        b.close_path();
        Arc::new(Node::path(NodeId(id), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(Color::rgb(0.8, 0.2, 0.1)), Paint::None, 0.0)))
    }

    fn group_with(params: Value, children: Vec<Arc<Node>>) -> Node {
        let mut g = Node::group(NodeId(9), children);
        g.appearance.effects.push(crate::new_effect(REVOLVE, &params).unwrap());
        g
    }

    #[test]
    fn a_groups_members_share_the_axis_on_the_edge_of_them_all() {
        let mut hidden = (*member(3, -500.0)).clone();
        hidden.visible = false;
        let nested = Arc::new(Node::group(NodeId(5), vec![member(4, 90.0)]));
        let children = vec![member(1, 0.0), nested, Arc::new(hidden)];
        // Left edge, 5 pt out: the axis at -5, the far wall (100) 105 from it.
        let g = group_with(json!({"segments":32,"offset":5,"rotationX":0}), children.clone());
        let b = revolve_art(&g).unwrap().visual_bounds().unwrap();
        assert!((b.x0 + 110.0).abs() < 1.5 && (b.x1 - 100.0).abs() < 1.5, "{b:?}");
        // Right edge, 5 pt out: the axis at 105, the far wall (0) 105 from it.
        let g = group_with(json!({"segments":32,"offset":5,"edge":"right","rotationX":0}), children);
        let b = revolve_art(&g).unwrap().visual_bounds().unwrap();
        assert!((b.x1 - 210.0).abs() < 1.5 && (b.x0 - 0.0).abs() < 1.5, "{b:?}");
    }

    #[test]
    fn opaque_members_share_one_back_to_front_order_and_others_keep_their_own_surfaces() {
        let g = group_with(json!({"segments":16}), vec![member(1, 0.0), member(2, 50.0)]);
        let art = revolve_art(&g).unwrap();
        assert_eq!(art.children().unwrap().len(), 1, "one order for every face");
        assert!(art.children().unwrap()[0].children().unwrap().len() > 16);
        let mut faded = (*member(2, 50.0)).clone();
        faded.opacity = 0.5;
        let g = group_with(json!({"segments":16}), vec![member(1, 0.0), Arc::new(faded)]);
        let art = revolve_art(&g).unwrap();
        let surfaces = art.children().unwrap();
        assert_eq!(surfaces.len(), 2, "a transparent member stays a surface of its own");
        assert_eq!(surfaces[1].opacity, 0.5);
    }

    fn star(id: u64) -> Arc<Node> {
        let anchors = (0..1200)
            .map(|i| {
                let (s, c) = (i as f64 * std::f64::consts::PI / 600.0).sin_cos();
                let r = if i % 2 == 0 { 100.0 } else { 60.0 };
                vectorcraft_geom::Anchor::corner(vectorcraft_geom::Point::new(200.0 + r * c, 200.0 + r * s))
            })
            .collect();
        Arc::new(Node::path(
            NodeId(id),
            PathData::new(vec![vectorcraft_geom::SubPath::new(anchors, true)]),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        ))
    }

    #[test]
    fn a_groups_members_share_revolves_limits() {
        let alone = group_with(json!({}), vec![star(1)]);
        assert!(validate_revolve(&alone, &json!({})).is_ok());
        let both = group_with(json!({}), vec![star(1), star(2)]);
        let refused = validate_revolve(&both, &json!({})).unwrap_err();
        assert!(refused.contains("reduce Segments"), "{refused}");
        assert_eq!(revolve_art(&both), None, "past the limits the source draws");
    }

    #[test]
    fn a_3d_effect_on_a_groups_fill_is_refused() {
        let mut group = Node::group(NodeId(9), vec![member(1, 0.0)]);
        group.appearance = Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0).into();
        for id in [REVOLVE, crate::EXTRUDE] {
            assert!(crate::validate_three_d(&group, id, &json!({})).is_ok(), "{id} on the group itself");
            let refused = crate::validate_three_d_at(&group, id, &json!({}), Some((0, 0))).unwrap_err();
            assert!(refused.contains("on the group itself"), "{id}: {refused}");
        }
    }

    #[test]
    fn validation_measures_the_profile_after_the_geometry_effects_before_revolve() {
        let roughen = crate::new_effect("distort.roughen", &json!({"size": 1, "detail": 100})).unwrap();
        let mut group = group_with(json!({}), vec![star(1)]);
        assert!(validate_revolve(&group, &json!({})).is_ok());
        group.appearance.effects.insert(0, roughen.clone());
        assert!(validate_revolve(&group, &json!({})).is_err(), "the group's reshaped member");
        assert_eq!(revolve_art(&group), None);
        let mut object = (*star(1)).clone();
        assert!(validate_revolve(&object, &json!({})).is_ok());
        object.appearance.effects.insert(0, roughen);
        assert!(validate_revolve(&object, &json!({})).is_err(), "a reshaped object not yet revolved");
    }

    #[test]
    fn paint_copies_past_the_budget_are_refused_for_the_whole_group() {
        use vectorcraft_doc::color::{Gradient, GradientKind, GradientPaint, GradientStop};
        let painted = |id: u64, x: f64, stops: usize| {
            let stops = (0..stops).map(|i| GradientStop::new(i as f32 / stops as f32, Color::BLACK)).collect();
            let paint = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
            let mut node = (*member(id, x)).clone();
            node.appearance = Appearance::basic(paint, Paint::None, 0.0).into();
            Arc::new(node)
        };
        let plain = member(1, 0.0);
        let NodeKind::Path { path: profile, .. } = &plain.kind else { panic!("a path") };
        let faces = vectorcraft_three_d::revolve(profile, profile.bounds().unwrap(), revolve_options(&json!({}))).unwrap().len();
        let stops = MAX_PAINT_COPIES * 6 / 10 / (faces * 4) / std::mem::size_of::<GradientStop>();
        let one = group_with(json!({}), vec![painted(1, 0.0, stops)]);
        assert_eq!(validate_revolve(&one, &json!({})), Ok(()));
        let two = group_with(json!({}), vec![painted(1, 0.0, stops), painted(2, 20.0, stops)]);
        let refused = validate_revolve(&two, &json!({})).unwrap_err();
        assert!(refused.contains("simplify the gradient"), "{refused}");
        assert_eq!(revolve_art(&two), None, "past the budget the source draws");
        let object = painted(3, 0.0, stops * 2);
        assert!(validate_revolve(&object, &json!({})).is_err(), "one object past the budget");
    }

    #[test]
    fn the_fills_of_one_object_share_the_budget_of_paint_copies() {
        use vectorcraft_doc::color::{Gradient, GradientKind, GradientPaint, GradientStop};
        let source = profile_node();
        let NodeKind::Path { path: profile, .. } = &source.kind else { panic!("a path") };
        let faces = vectorcraft_three_d::revolve(profile, profile.bounds().unwrap(), revolve_options(&json!({}))).unwrap().len();
        let stops = MAX_PAINT_COPIES * 6 / 10 / (faces * 4) / std::mem::size_of::<GradientStop>();
        let fill = |stops: usize, revolved: bool| {
            let stops = (0..stops).map(|i| GradientStop::new(i as f32 / stops as f32, Color::BLACK)).collect();
            let paint = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
            let mut item = AppearanceItem::Fill(FillLayer::new(paint));
            if revolved {
                item.effects_mut().push(fx());
            }
            item
        };
        let with = |items: Vec<AppearanceItem>| {
            let mut n = source.clone();
            n.appearance = Appearance { items, ..Default::default() }.into();
            n
        };
        let one = with(vec![fill(stops, true)]);
        assert!(revolve_art(&one).is_some());
        assert_eq!(validate_item_revolve(&one, &json!({}), 0, 0), Ok(()));
        let two = with(vec![fill(stops, true), fill(stops, true)]);
        assert_eq!(revolve_art(&two), None, "past the budget the source draws");
        let refused = validate_item_revolve(&two, &json!({}), 1, 0).unwrap_err();
        assert!(refused.contains("simplify the gradient"), "{refused}");
        let second = with(vec![fill(stops, true), fill(stops, false)]);
        assert!(validate_item_revolve(&second, &json!({}), 1, 0).is_err(), "the item the effect goes on counts");
        assert_eq!(validate_item_revolve(&second, &json!({}), 0, 0), Ok(()), "an item without a Revolve does not");
        let below = with(vec![fill(stops * 2, false), fill(1, false)]);
        assert!(validate_item_revolve(&below, &json!({}), 0, 0).is_err(), "the item's own paint, not the top fill");
        assert_eq!(validate_item_revolve(&below, &json!({}), 1, 0), Ok(()));
    }

    #[test]
    fn an_items_revolve_is_validated_after_only_the_geometry_effects_before_it() {
        let roughen = crate::new_effect("distort.roughen", &json!({"size": 1, "detail": 100})).unwrap();
        let mut object = (*star(1)).clone();
        let mut hidden = fx();
        hidden.visible = false;
        let item = object.appearance.items.first_mut().unwrap();
        item.effects_mut().push(hidden);
        item.effects_mut().push(roughen);
        assert_eq!(validate_item_revolve(&object, &json!({}), 0, 0), Ok(()), "the roughen after it is not swept");
        assert!(validate_item_revolve(&object, &json!({}), 0, 2).is_err(), "a Revolve added after the roughen");
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
