//! Revolve and Revolve (Classic) as live, shaded vector art, shared by the canvas and every
//! exporter.
use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Effect, FillLayer, LineJoin, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{PathData, PathEl, Point, Rect, Vec2};
use vectorcraft_three_d::{Edge, Revolve};

use crate::solids::{
    Profile, Scope, is_container, members, object_profile, past_effect, profile, reshape_members, resolve_for, solid_paints, union_bounds,
};
use crate::util::{flag, num, text};
use crate::{GeomContext, apply_geometry_with, is_geometry, merged_params};

pub const REVOLVE: &str = "threeD.revolve";

mod classic;
pub use classic::CLASSIC_REVOLVE;

/// Is `id` Revolve or Revolve (Classic), the effects this module draws?
pub fn is_revolve(id: &str) -> bool {
    id == REVOLVE || id == CLASSIC_REVOLVE
}

/// Most bytes the copies of the object's paint in the faces may take together. Each face and the
/// seam stroke around it gets a copy of the paint, so a gradient of many stops times tens of
/// thousands of faces took gigabytes; past this the source path is drawn.
const MAX_PAINT_COPIES: usize = 256 << 20;

/// The most face and box tests a group's transparent members' faces take together to rejoin their
/// members' last runs; past them each starts a run of its own.
pub(crate) const MAX_OVERLAP_TESTS: usize = 1 << 22;

/// The width of the stroke around each face that covers the seams between faces.
const SEAM_WIDTH: f64 = 0.75;

/// About the bytes one copy of `paint` takes, with the strings it owns (swatch and pattern names).
fn paint_bytes(paint: &Paint) -> usize {
    use std::mem::size_of;
    use vectorcraft_doc::color::{FreeformPoint, GradientStop};
    let own = size_of::<Paint>();
    let name = |s: &Option<String>| s.as_ref().map_or(0, String::len);
    let g = match paint {
        Paint::Solid { swatch, .. } => return own + name(swatch),
        Paint::Pattern { pattern, .. } => return own + pattern.len(),
        Paint::Gradient(g) => g,
        _ => return own,
    };
    let freeform = g.freeform.as_ref().map_or(0, |f| {
        // Each line is a Vec: its header too, not only its indices.
        f.points.len() * size_of::<FreeformPoint>()
            + f.lines.len() * size_of::<Vec<usize>>()
            + f.lines.iter().map(|l| l.len() * size_of::<usize>()).sum::<usize>()
    });
    let stops: usize = g.gradient.stops.iter().map(|s| size_of::<GradientStop>() + name(&s.swatch)).sum();
    own + size_of::<vectorcraft_doc::color::GradientPaint>() + name(&g.swatch) + stops + freeform
}

/// Add `copies` copies of `paint` to the running byte total `used`; whether it still fits within
/// [`MAX_PAINT_COPIES`] (for the solid surfaces, whose items' copies count together).
pub(crate) fn paint_copies_add(used: &mut usize, copies: usize, paint: &Paint) -> bool {
    *used = used.saturating_add(copies.saturating_mul(paint_bytes(paint)));
    *used <= MAX_PAINT_COPIES
}

fn visible(fx: &[Effect]) -> bool {
    fx.iter().any(|e| e.visible && is_revolve(&e.id))
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

/// The options Revolve (`id` [`REVOLVE`]) or Revolve (Classic) ([`CLASSIC_REVOLVE`]) with
/// parameters `p` sweeps its surface with.
fn options_of(id: &str, p: &Value) -> Revolve {
    if id == CLASSIC_REVOLVE { classic::options(p) } else { revolve_options(p) }
}

/// The options effect `fx`, a Revolve or a Revolve (Classic), sweeps its surface with.
fn fx_options(fx: &Effect) -> Revolve {
    options_of(&fx.id, &merged_params(&fx.id, &fx.params))
}

/// Validate before a command mutates the document. Loading external files is still bounded
/// by the evaluator; unsupported source kinds remain unchanged when rendered.
pub fn validate_revolve(n: &Node, p: &Value) -> Result<(), String> {
    validate_revolve_in(None, n, p)
}

/// [`validate_revolve`] with the symbols of `doc`: a symbol instance, a blend and a clipping group
/// revolve the art they show ([`resolve_for`]), cut for Revolve whatever 3D effect `n` carries now.
pub fn validate_revolve_in(doc: Option<&Document>, n: &Node, p: &Value) -> Result<(), String> {
    validate_in(doc, n, REVOLVE, p)
}

/// [`validate_revolve_in`] for Revolve or Revolve (Classic), `id`.
pub(crate) fn validate_in(doc: Option<&Document>, n: &Node, id: &str, p: &Value) -> Result<(), String> {
    validate_params_of(id, p)?;
    let n = &*resolve_for(doc, n, "Revolve", false)?;
    let effects = &n.appearance.effects;
    // The profile is measured before the object's one Revolve, hidden or shown.
    let before = effects.iter().position(|e| is_revolve(&e.id)).and_then(|i| effects.get(..i)).unwrap_or(effects);
    if is_container(n) {
        const EMPTY: &str = "Revolve requires a group holding a shape with at least two anchors";
        let mut shapes = members(n, "Revolve")?;
        if !shapes.iter().any(|s| has_two_anchors(&s.path)) {
            return Err(EMPTY.into());
        }
        reshape_members(n, before, &mut shapes);
        let b = union_bounds(&shapes).ok_or(EMPTY)?;
        let options = shared_options(&shapes, options_of(id, p))?;
        let mut copies = 0;
        shapes.iter().try_for_each(|s| check_surface(&s.path, b, options, surface_item(&s.items), &mut copies))?;
        let casting = shapes.iter().filter(|s| surface_item(&s.items).is_some_and(|i| paint_of(i).1 * s.opacity > 0.0));
        return check_shadow(p, casting.map(|s| &s.path), b, options);
    }
    let (path, _) = object_profile(n, "Revolve")?;
    if !has_two_anchors(&path) {
        return Err("Revolve requires a shape with at least two anchors".into());
    }
    let path = geometry(&path, before, &GeomContext::of(n));
    let b = path.bounds().ok_or("Revolve requires a shape with at least two anchors")?;
    let paints = solid_paints(n);
    check_surface(&path, b, options_of(id, p), surface_item(&paints), &mut 0)?;
    check_shadow(p, [&path], b, options_of(id, p))
}

/// With the shadow of Revolve parameters `p` on, an error when the surfaces of `paths` it is cast
/// from, about the axis of `b`, are past the limits together.
fn check_shadow<'a>(p: &Value, paths: impl IntoIterator<Item = &'a PathData>, b: Rect, options: Revolve) -> Result<(), String> {
    if crate::solids::shadow::options(p).is_none() {
        return Ok(());
    }
    let mut caster = vectorcraft_three_d::ShadowCaster::new();
    paths.into_iter().try_for_each(|path| caster.revolve(path, vectorcraft_geom::FillRule::NonZero, b, options).map(|_| ()))
}

/// [`validate_revolve`] for a Revolve with parameters `p` at index `index` of the effects of
/// appearance item `target` of `n` ([`validate_item_of`]).
#[cfg(test)]
pub(crate) fn validate_item_revolve(n: &Node, p: &Value, target: usize, index: usize) -> Result<(), String> {
    validate_item_of(n, REVOLVE, p, target, index)
}

/// [`validate_revolve`] for a Revolve or Revolve (Classic), `id`, with parameters `p` at index
/// `index` of the effects of appearance item `target` of `n`. The surfaces of every fill and stroke
/// that carries one, `target`'s included, share one budget of paint copies, as in [`art`]. A visible
/// object-level Revolve draws in place of those surfaces.
pub(crate) fn validate_item_of(n: &Node, id: &str, p: &Value, target: usize, index: usize) -> Result<(), String> {
    if is_container(n) || visible(&n.appearance.effects) || target >= n.appearance.items.len() {
        return validate_in(None, n, id, p);
    }
    validate_params_of(id, p)?;
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
        let at = if i == target { Some(index) } else { effects.iter().position(|e| e.visible && is_revolve(&e.id)) };
        let (kind, params) = match at.and_then(|at| effects.get(at)) {
            _ if i == target => (id, p.clone()),
            Some(fx) if crate::paints(item) => (fx.id.as_str(), merged_params(&fx.id, &fx.params)),
            _ => continue,
        };
        let options = options_of(kind, &params);
        let path = geometry(&path, at.and_then(|at| effects.get(..at)).unwrap_or(effects), &ctx.item(item));
        let b = path.bounds().ok_or(EMPTY)?;
        check_surface(&path, b, options, crate::paints(item).then_some(item), &mut copies)?;
        check_shadow(&params, [&path], b, options)?;
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

/// Check Revolve's parameter values `p` alone: its lights, shadow, `edge` and `cap`.
pub(crate) fn validate_revolve_params(p: &Value) -> Result<(), String> {
    crate::lighting::validate(p)?;
    crate::solids::shadow::validate(p)?;
    if p.get("edge").is_some_and(|v| v.as_str() != Some("left") && v.as_str() != Some("right")) {
        return Err("Revolve edge must be left or right".into());
    }
    if p.get("cap").is_some_and(|v| v.as_bool().is_none()) {
        return Err("Revolve cap must be a boolean".into());
    }
    Ok(())
}

/// [`validate_revolve_params`] for Revolve or Revolve (Classic), `id`; Revolve (Classic) also takes
/// only its own parameters, each in its range.
pub(crate) fn validate_params_of(id: &str, p: &Value) -> Result<(), String> {
    validate_revolve_params(p)?;
    if id == CLASSIC_REVOLVE {
        classic::validate(p)?;
    }
    Ok(())
}

fn geometry(path: &PathData, fx: &[Effect], ctx: &GeomContext) -> PathData {
    apply_geometry_with(fx, path, path.bounds().unwrap_or(Rect::ZERO), ctx)
}

/// The shadow of Revolve `fx` cast by the surface of `path` in `item`'s paint about the axis of
/// `b`, revolved with `options`, as dark as `item` is opaque and in its blending mode; `None` with
/// the shadow off, when `item` is fully transparent or when it casts nothing, and an error when
/// the surface it is cast from is past the limits.
fn shadow_of(id: NodeId, fx: &Effect, path: &PathData, item: &AppearanceItem, b: Rect, options: Revolve) -> Result<Option<Node>, String> {
    let Some(settings) = crate::solids::shadow::options(&merged_params(&fx.id, &fx.params)) else { return Ok(None) };
    let (_, opacity, blend) = paint_of(item);
    if opacity <= 0.0 {
        return Ok(None);
    }
    let mut caster = vectorcraft_three_d::ShadowCaster::new();
    caster.revolve(path, vectorcraft_geom::FillRule::NonZero, b, options)?;
    Ok(crate::solids::shadow::art(id, &caster, settings, crate::solids::shadow::Ink { opacity, blend }))
}

/// The shadow of group Revolve `fx` cast by the surfaces of the members `shapes` that have one,
/// about the axis of `b`, on one plane; each member's part is as dark as its surface is opaque,
/// in its paint's blending mode, and its opacity mask and blending mode apply to that part
/// ([`crate::solids::shadow::scoped_art`]). An error when the surfaces it is cast from are past the
/// limits.
fn group_shadow(id: NodeId, fx: &Effect, shapes: &[Profile], b: Rect, options: Revolve) -> Result<Vec<Arc<Node>>, String> {
    let Some(settings) = crate::solids::shadow::options(&merged_params(&fx.id, &fx.params)) else { return Ok(vec![]) };
    let mut caster = vectorcraft_three_d::ShadowCaster::new();
    let mut cast = vec![];
    for s in shapes {
        let Some((_, opacity, blend)) = surface_item(&s.items).map(paint_of) else { continue };
        // A member whose surface is fully transparent casts nothing.
        let ink = crate::solids::shadow::Ink { opacity: opacity * s.opacity, blend };
        if ink.opacity <= 0.0 {
            continue;
        }
        let faces = caster.revolve(&s.path, vectorcraft_geom::FillRule::NonZero, b, options)?;
        cast.push((s.scopes.as_slice(), ink, faces));
    }
    Ok(crate::solids::shadow::scoped_art(id, &caster, settings, cast, crate::solids::has_masks(shapes)))
}

/// One object's surface of revolution about the axis of `b`, and each face's depth. `copies` counts
/// the bytes of the paint copies of every surface of the object. Revolve (Classic) draws every face
/// with its classic surface, as Extrude & Bevel (Classic) does.
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
    if fx.id == CLASSIC_REVOLVE {
        let facets = vectorcraft_three_d::revolve_facets(path, b, options).ok()?;
        *copies = copies.saturating_add(paint_copies(facets.len(), &paint));
        if *copies > MAX_PAINT_COPIES {
            return None;
        }
        let depths = facets.iter().map(|f| f.depth).collect();
        let p = merged_params(&fx.id, &fx.params);
        return Some((depths, crate::solids::classic::surface(n, facets, &paint, opacity, blend, &p)));
    }
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
                    let mut edge = StrokeLayer::new(fill.paint.clone(), SEAM_WIDTH);
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
    surface(n, path, b, fx_options(fx), fx, paint, opacity, blend, expand, copies).map(|(_, art)| art)
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
    revolve_art_in(None, n)
}

/// [`revolve_art`] with the symbols of `doc`: a symbol instance, a blend and a clipping group, alone
/// or in a group, revolve the art they show ([`resolve_for`]). `None` too when that art can't be
/// resolved.
pub fn revolve_art_in(doc: Option<&Document>, n: &Node) -> Option<Node> {
    art(doc, n, false)
}

/// [`expand_revolve_art_in`] without a document.
#[cfg(test)]
pub(crate) fn expand_revolve_art(n: &Node) -> Option<Node> {
    expand_revolve_art_in(None, n)
}

/// Revolve's own Expand Appearance path, with the symbols of `doc` ([`revolve_art_in`]). The shared
/// command and UI remain unchanged.
pub(crate) fn expand_revolve_art_in(doc: Option<&Document>, n: &Node) -> Option<Node> {
    art(doc, n, true)
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
        let mesh = vectorcraft_three_d::revolve_mesh(path, rule, b, fx_options(fx))?;
        Ok(crate::SolidMesh { mesh, color: crate::solids::mesh_color(&paint, opacity) })
    };
    if let Some(index) = n.appearance.effects.iter().position(|e| e.visible && is_revolve(&e.id)) {
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
        if let Some(index) = item.effects().iter().position(|e| e.visible && is_revolve(&e.id))
            && let Some(fx) = item.effects().get(index)
        {
            let p = geometry(&path, item.effects().get(..index).unwrap_or_default(), &ctx.item(item));
            crate::solids::keep_mesh(&mut out, total, mesh(&p, fx, item)?, limit)?;
        }
    }
    Ok(out)
}

/// The outline of every face [`revolve_art`] draws for `n`'s Revolve or Revolve (Classic) (its symbol
/// instances, blends and clipping groups already resolved), in document coordinates: with the
/// effect on the object, the surface in its top visible fill (else its stroke); with it on fills and strokes, one surface for each of
/// them, reshaped by the geometry effects after each one's Revolve; on a group or layer, one for
/// each member, about one shared axis. The swatches and patterns named by a live-effect plug-in
/// after a fill's or stroke's Revolve are looked up in `doc`.
pub(crate) fn revolve_face_paths(doc: Option<&Document>, n: &Node) -> Result<Vec<PathData>, String> {
    let faces = |path: &PathData, b: Rect, options: Revolve| -> Result<Vec<PathData>, String> {
        Ok(vectorcraft_three_d::revolve(path, b, options)?.iter().map(vectorcraft_three_d::Face::path).collect())
    };
    if is_container(n) {
        let index = n
            .appearance
            .effects
            .iter()
            .position(|e| e.visible && is_revolve(&e.id))
            .ok_or("a 3D effect on a group's fill or stroke draws nothing")?;
        let Some(fx) = n.appearance.effects.get(index) else { return Ok(vec![]) };
        let mut shapes = members(n, "Revolve")?;
        reshape_members(n, n.appearance.effects.get(..index).unwrap_or_default(), &mut shapes);
        let b = union_bounds(&shapes).ok_or("Revolve requires a group holding a shape with at least two anchors")?;
        let options = shared_options(&shapes, fx_options(fx))?;
        let mut out = vec![];
        for s in shapes.iter().filter(|s| surface_item(&s.items).is_some()) {
            out.extend(faces(&s.path, b, options)?);
        }
        return Ok(out);
    }
    let (path, _) = object_profile(n, "Revolve")?;
    let ctx = GeomContext::of(n);
    if let Some(index) = n.appearance.effects.iter().position(|e| e.visible && is_revolve(&e.id)) {
        let Some(fx) = n.appearance.effects.get(index) else { return Ok(vec![]) };
        let path = geometry(&path, n.appearance.effects.get(..index).unwrap_or_default(), &ctx);
        if surface_item(&solid_paints(n)).is_none() {
            return Err("the revolved object has no fill or stroke".into());
        }
        let b = path.bounds().ok_or("Revolve requires a shape with at least two anchors")?;
        return faces(&path, b, fx_options(fx));
    }
    let path = geometry(&path, &n.appearance.effects, &ctx);
    let mut out = vec![];
    for item in n.appearance.items.iter().filter(|i| crate::paints(i)) {
        if let Some(index) = item.effects().iter().position(|e| e.visible && is_revolve(&e.id))
            && let Some(fx) = item.effects().get(index)
        {
            let p = geometry(&path, item.effects().get(..index).unwrap_or_default(), &ctx.item(item));
            let b = p.bounds().ok_or("Revolve requires a shape with at least two anchors")?;
            let surface = faces(&p, b, fx_options(fx))?;
            out.extend(crate::solids::wireframe::reshaped(surface, item.effects().get(index + 1..).unwrap_or_default(), &GeomContext { doc, ..ctx }));
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
        .position(|e| e.visible && is_revolve(&e.id))
        .ok_or("a 3D effect on a group's fill or stroke draws nothing to export")?;
    let Some(fx) = n.appearance.effects.get(index) else { return Ok(vec![]) };
    let mut shapes = members(n, "Revolve")?;
    reshape_members(n, n.appearance.effects.get(..index).unwrap_or_default(), &mut shapes);
    let b = union_bounds(&shapes).ok_or("Revolve requires a group holding a shape with at least two anchors")?;
    let options = shared_options(&shapes, fx_options(fx))?;
    let mut out = vec![];
    for s in &shapes {
        let Some(item) = surface_item(&s.items) else { continue };
        let (paint, opacity, _) = paint_of(item);
        let mesh = vectorcraft_three_d::revolve_mesh(&s.path, s.rule, b, options)?;
        crate::solids::keep_mesh(&mut out, total, crate::SolidMesh { mesh, color: crate::solids::mesh_color(&paint, opacity * s.opacity) }, limit)?;
    }
    Ok(out)
}

fn art(doc: Option<&Document>, n: &Node, expand: bool) -> Option<Node> {
    if !has_revolve(n) {
        return None;
    }
    let n = &*resolve_for(doc, n, "Revolve", false).ok()?;
    if is_container(n) {
        return group_art(n, expand);
    }
    let (path, rule) = profile(n)?;
    let ctx = GeomContext::of(n);
    let mut children = vec![];
    let mut copies = 0;
    let mut remaining = n.appearance.effects.clone();
    if let Some(index) = n.appearance.effects.iter().position(|e| e.visible && is_revolve(&e.id)) {
        let fx = n.appearance.effects.get(index)?;
        let path = geometry(&path, n.appearance.effects.get(..index)?, &ctx);
        // An object's surface uses its top visible fill, falling back to its stroke.
        let paints = solid_paints(n);
        let item = surface_item(&paints)?;
        let expand = expand && !n.appearance.effects.iter().skip(index + 1).any(|e| e.visible && is_geometry(&e.id));
        let surface = object_surface(n, &path, fx, item, expand, &mut copies)?;
        if let Some(b) = path.bounds()
            && let Some(shadow) = shadow_of(n.id, fx, &path, item, b, fx_options(fx)).ok()?
        {
            children.push(Arc::new(shadow));
        }
        children.push(Arc::new(surface));
        // Later geometry acts on the projected art as a whole through container evaluation.
        remaining = past_effect(&n.appearance.effects, index);
        remaining.retain(|e| !is_revolve(&e.id));
    } else {
        let path = geometry(&path, &n.appearance.effects, &ctx);
        remaining = crate::without_geometry(&remaining);
        // The object's Transform effects, applied to the path above, transform the items' raster
        // effects too. Each copy keeps its item beside it, which plug-in effects are told they sit
        // on.
        let originals: Vec<&AppearanceItem> = n.appearance.items.iter().filter(|i| crate::paints(i)).collect();
        let mut items: Vec<AppearanceItem> = originals.iter().map(|i| (*i).clone()).collect();
        crate::raster::follow_paints(&mut items, &n.appearance.effects);
        // Every item's shadow goes behind every item's art.
        let mut shadows = vec![];
        for (original, item) in originals.into_iter().zip(&items) {
            if let Some(index) = item.effects().iter().position(|e| e.visible && is_revolve(&e.id)) {
                let fx = item.effects().get(index)?;
                let p = geometry(&path, item.effects().get(..index)?, &ctx.item(original));
                let expand = expand && !item.effects().iter().skip(index + 1).any(|e| e.visible && is_geometry(&e.id));
                let mut art = object_surface(n, &p, fx, item, expand, &mut copies)?;
                art.appearance.effects = item.effects().iter().skip(index + 1).filter(|e| !is_revolve(&e.id)).cloned().collect();
                if let Some(b) = p.bounds()
                    && let Some(mut shadow) = shadow_of(n.id, fx, &p, item, b, fx_options(fx)).ok()?
                {
                    shadow.appearance.effects = art.appearance.effects.clone();
                    shadows.push(Arc::new(shadow));
                }
                children.push(Arc::new(art));
            } else {
                let flat = Appearance { items: vec![item.clone()], ..Default::default() };
                children.push(Arc::new(crate::group::shape_node(n.id, path.clone(), rule, flat)));
            }
        }
        children.splice(0..0, shadows);
    }
    let mut art = n.clone();
    art.kind = NodeKind::Group { children, clip: false };
    art.appearance = Appearance { effects: remaining, ..Default::default() }.into();
    Some(art)
}

/// The members' `surfaces`, or runs of their faces, in order, from the [`Scope`] at `level` of each
/// one's `Profile::scopes` inward: consecutive surfaces that share a scope go in one group with its
/// opacity, blending mode and opacity mask, so each applies once, to the faces it holds together.
pub(crate) fn scoped<'a>(id: NodeId, surfaces: Vec<(&'a [Scope], Arc<Node>)>, level: usize) -> Vec<Arc<Node>> {
    fn close(id: NodeId, run: &mut Vec<(&[Scope], Arc<Node>)>, scope: Option<&Scope>, level: usize, out: &mut Vec<Arc<Node>>) {
        if let Some(scope) = scope {
            out.push(Arc::new(scope.group(id, scoped(id, std::mem::take(run), level + 1))));
        }
    }
    let mut out = vec![];
    let mut run = vec![];
    let mut scope: Option<&'a Scope> = None;
    for (scopes, s) in surfaces {
        let at = scopes.get(level);
        if at.map(|a| a.key) != scope.map(|s| s.key) {
            close(id, &mut run, scope, level, &mut out);
            scope = at;
        }
        if scope.is_some() {
            run.push((scopes, s));
        } else {
            out.push(s);
        }
    }
    close(id, &mut run, scope, level, &mut out);
    out
}

/// What a face can paint, to tell whether two faces overlap: the convex hull of the points and
/// control points of its outline, which holds the outline, and the hull's box grown by half the
/// seam stroke.
pub(crate) struct Coverage {
    pub(crate) bounds: Rect,
    hull: Vec<Point>,
}

impl Coverage {
    /// `None` for a face with no outline and no box.
    pub(crate) fn of(face: &Node) -> Option<Coverage> {
        let points: Vec<Point> = match face.path_data() {
            Some(path) => path
                .to_bezpath()
                .elements()
                .iter()
                .flat_map(|e| match *e {
                    PathEl::MoveTo(p) | PathEl::LineTo(p) => vec![p],
                    PathEl::QuadTo(a, b) => vec![a, b],
                    PathEl::CurveTo(a, b, c) => vec![a, b, c],
                    PathEl::ClosePath => vec![],
                })
                .collect(),
            None => {
                let b = face.visual_bounds()?;
                vec![Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)]
            }
        };
        let hull = convex_hull(points);
        let first = *hull.first()?;
        let reach = SEAM_WIDTH / 2.0;
        let bounds = hull.iter().fold(Rect::from_points(first, first), |b, p| b.union_pt(*p)).inflate(reach, reach);
        Some(Coverage { bounds, hull })
    }

    /// Do the two faces' hulls, each grown by half the seam stroke, overlap? A test of the
    /// separating axes of their edges and of the page's axes: `true` whenever they overlap, and
    /// sometimes near a corner when they don't.
    pub(crate) fn meets(&self, other: &Coverage) -> bool {
        fn normals(hull: &[Point]) -> impl Iterator<Item = Vec2> + '_ {
            (0..hull.len()).filter_map(|i| {
                let (a, b) = (hull.get(i)?, hull.get((i + 1) % hull.len())?);
                Some(Vec2::new(a.y - b.y, b.x - a.x))
            })
        }
        let axes = [Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)].into_iter().chain(normals(&self.hull)).chain(normals(&other.hull));
        let span = |hull: &[Point], axis: Vec2| {
            hull.iter().map(|p| p.to_vec2().dot(axis)).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)))
        };
        axes.filter(|axis| axis.hypot() > 0.0).all(|axis| {
            let ((a0, a1), (b0, b1)) = (span(&self.hull, axis), span(&other.hull, axis));
            let gap = SEAM_WIDTH * axis.hypot();
            b0 - a1 < gap && a0 - b1 < gap
        })
    }
}

/// The convex hull of `points`, counterclockwise in a y-up frame, without repeated points.
fn convex_hull(mut points: Vec<Point>) -> Vec<Point> {
    points.retain(|p| p.is_finite());
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    // One side of the hull from the first point to the last, without the last.
    fn side<'a>(points: impl Iterator<Item = &'a Point>) -> Vec<Point> {
        let mut chain: Vec<Point> = vec![];
        for &p in points {
            while let [.., o, a] = chain[..]
                && (a - o).cross(p - o) <= 0.0
            {
                chain.pop();
            }
            chain.push(p);
        }
        chain.pop();
        chain
    }
    let mut hull = side(points.iter());
    hull.extend(side(points.iter().rev()));
    hull
}

/// Do boxes `a` and `b` overlap with some area?
pub(crate) fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

/// [`art`] of group or layer `n`: every member swept about one axis on the bounds of all of them,
/// each in its own paint. Every member's faces are drawn in one back-to-front order. `None` when
/// the effect is only on the group's own fills or strokes or when its members can't be revolved.
fn group_art(n: &Node, expand: bool) -> Option<Node> {
    let index = n.appearance.effects.iter().position(|e| e.visible && is_revolve(&e.id))?;
    let fx = n.appearance.effects.get(index)?;
    let mut shapes = members(n, "").ok()?;
    reshape_members(n, n.appearance.effects.get(..index)?, &mut shapes);
    let b = union_bounds(&shapes)?;
    let options = shared_options(&shapes, fx_options(fx)).ok()?;
    let expand = expand && !n.appearance.effects.iter().skip(index + 1).any(|e| e.visible && is_geometry(&e.id));
    let mut copies = 0;
    let mut surfaces = vec![];
    let mut opaque = true;
    for s in &shapes {
        let Some(item) = surface_item(&s.items) else { continue };
        // The opacity and blending of the member and the groups it is in apply through its scopes.
        let (paint, opacity, blend) = paint_of(item);
        opaque &= opacity >= 1.0 && blend == Default::default() && s.scopes.is_empty();
        // A member with transparency of its own keeps every face, as a transparent object does.
        let expand = expand && s.scopes.is_empty();
        let (depths, art) = surface(n, &s.path, b, options, fx, paint, opacity, blend, expand, &mut copies)?;
        surfaces.push((depths, art, s.scopes.as_slice()));
    }
    if surfaces.is_empty() {
        return None;
    }
    let plain = surfaces.iter().all(|(depths, s, _)| matches!(&s.kind, NodeKind::Group { children, clip: false } if children.len() == depths.len()));
    // A classic surface whose gradient has transparent stops carries an opacity mask: its faces go
    // in runs, as a transparent member's do, and each run carries the mask.
    let masks: Vec<Option<Box<vectorcraft_doc::OpacityMask>>> = surfaces.iter().map(|(_, s, _)| s.extras().mask.clone()).collect();
    let opaque = opaque && masks.iter().all(Option::is_none);
    let children = if !plain {
        scoped(n.id, surfaces.into_iter().map(|(_, s, scopes)| (scopes, Arc::new(s))).collect(), 0)
    } else {
        let count = surfaces.len();
        let mut faces = vec![];
        for (member, (depths, s, scopes)) in surfaces.into_iter().enumerate() {
            let (opacity, blend) = (s.opacity, s.blend);
            if let NodeKind::Group { children, .. } = s.kind {
                faces.extend(depths.into_iter().zip(children).map(|(depth, face)| (depth, member, opacity, blend, scopes, face)));
            }
        }
        // A stable sort: faces at the same depth keep the members' order.
        faces.sort_by(|a, b| a.0.total_cmp(&b.0));
        if opaque {
            vec![Arc::new(Node::group(n.id, faces.into_iter().map(|(.., face)| face).collect()))]
        } else {
            // An opaque member's face goes in the order on its own. The faces of a member with
            // transparency share a run, one group with the opacity and blending mode of its fill or
            // stroke, and its scopes group runs further. Such a face joins its member's last run
            // unless a face drawn after that run overlaps it: only then does the member split,
            // where one group can't be both behind and in front of another face.
            struct Run<'a> {
                member: Option<usize>,
                scopes: &'a [Scope],
                bounds: Rect,
                faces: Vec<Arc<Node>>,
                cover: Vec<Coverage>,
                opacity: f32,
                blend: vectorcraft_doc::color::BlendMode,
            }
            let mut runs: Vec<Run> = vec![];
            let mut last_run = vec![None; count];
            // Past this many tests a face starts a run of its own, which never draws out of order.
            let mut budget = MAX_OVERLAP_TESTS;
            for (_, member, opacity, blend, scopes, face) in faces {
                let paints = opacity > 0.0 && scopes.iter().all(|s| s.opacity > 0.0);
                let Some(cover) = Coverage::of(&face).filter(|_| paints) else {
                    // A face that paints nothing goes in the order on its own and overlaps nothing.
                    runs.push(Run { member: None, scopes, bounds: Rect::ZERO, faces: vec![face], cover: vec![], opacity, blend });
                    continue;
                };
                let masked = masks.get(member).is_some_and(Option::is_some);
                if opacity >= 1.0 && blend == Default::default() && scopes.is_empty() && !masked {
                    runs.push(Run { member: None, scopes, bounds: cover.bounds, faces: vec![face], cover: vec![cover], opacity, blend });
                    continue;
                }
                let mut clear = |r: &Run| {
                    budget = budget.saturating_sub(1);
                    if !overlaps(r.bounds, cover.bounds) {
                        return budget > 0;
                    }
                    budget = budget.saturating_sub(r.cover.len());
                    budget > 0 && r.cover.iter().all(|c| !overlaps(c.bounds, cover.bounds) || !c.meets(&cover))
                };
                let joins =
                    last_run.get(member).copied().flatten().filter(|&i: &usize| runs.get(i + 1..).is_some_and(|later| later.iter().all(&mut clear)));
                match joins.and_then(|i| runs.get_mut(i)) {
                    Some(run) => {
                        run.bounds = run.bounds.union(cover.bounds);
                        run.faces.push(face);
                        run.cover.push(cover);
                    }
                    None => {
                        if let Some(last) = last_run.get_mut(member) {
                            *last = Some(runs.len());
                        }
                        runs.push(Run { member: Some(member), scopes, bounds: cover.bounds, faces: vec![face], cover: vec![cover], opacity, blend });
                    }
                }
            }
            let runs = runs.into_iter().filter_map(|mut r| {
                // A face on its own needs a group only for the opacity or blending mode of its fill or stroke.
                if r.member.is_none() && r.opacity >= 1.0 && r.blend == Default::default() {
                    return r.faces.pop().map(|face| (r.scopes, face));
                }
                let mut group = Node::group(n.id, r.faces);
                group.opacity = r.opacity;
                group.blend = r.blend;
                if let Some(mask) = r.member.and_then(|m| masks.get(m)).cloned().flatten() {
                    group.extras_mut().mask = Some(mask);
                }
                Some((r.scopes, Arc::new(group)))
            });
            scoped(n.id, runs.collect(), 0)
        }
    };
    // One shadow for every member's surface, behind them all.
    let mut children = children;
    children.splice(0..0, group_shadow(n.id, fx, &shapes, b, options).ok()?);
    let remaining = past_effect(&n.appearance.effects, index);
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
    fn a_fill_without_the_revolve_keeps_the_even_odd_hole() {
        use vectorcraft_geom::FillRule;
        use vectorcraft_geom::hit::fill_contains;
        // Two nested squares drawn in the same direction: only the even-odd rule leaves a hole.
        let [outer, inner] = [(100.0, 200.0), (130.0, 170.0)].map(|(lo, hi)| {
            let mut b = BezPath::new();
            b.move_to((lo, lo));
            b.line_to((hi, lo));
            b.line_to((hi, hi));
            b.line_to((lo, hi));
            b.close_path();
            PathData::from_bezpath(&b)
        });
        let flat = AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::rgb(0.1, 0.3, 0.9))));
        for id in [REVOLVE, CLASSIC_REVOLVE] {
            let mut top = AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::rgb(0.8, 0.2, 0.1))));
            top.effects_mut().push(crate::new_effect(id, &json!({})).unwrap());
            let appearance = Appearance { items: vec![flat.clone(), top], ..Default::default() };
            let members = vec![
                Arc::new(Node::path(NodeId(2), outer.clone(), Appearance::default())),
                Arc::new(Node::path(NodeId(3), inner.clone(), Appearance::default())),
            ];
            let mut compound = Node::new(NodeId(1), NodeKind::Compound { children: members, rule: FillRule::EvenOdd });
            compound.appearance = appearance.clone().into();
            let both = PathData::new(outer.subpaths.iter().chain(&inner.subpaths).cloned().collect());
            let path = crate::group::shape_node(NodeId(1), both, FillRule::EvenOdd, appearance);
            for (n, shape) in [(compound, "compound path"), (path, "path")] {
                for (art, how) in [(revolve_art(&n), "live"), (expand_revolve_art(&n), "expanded")] {
                    let what = format!("{id} on the top fill of a {shape}, {how}");
                    let art = art.unwrap_or_else(|| panic!("{what}"));
                    let children = art.children().unwrap();
                    let plain: Vec<usize> = (0..children.len())
                        .filter(|&i| children[i].appearance.items == [flat.clone()] && children[i].path_data().is_some())
                        .collect();
                    let [index] = plain[..] else { panic!("{what}: one flat child, got {plain:?}") };
                    let NodeKind::Path { path, rule, .. } = &children[index].kind else { panic!("{what}: a path") };
                    assert_eq!(*rule, FillRule::EvenOdd, "{what}");
                    let bez = path.to_bezpath();
                    assert!(fill_contains(&bez, *rule, Point::new(110.0, 150.0)), "{what}: the ring is filled");
                    assert!(!fill_contains(&bez, *rule, Point::new(150.0, 150.0)), "{what}: the hole stays open");
                    assert!(index + 1 < children.len(), "{what}: the revolved art is drawn above");
                }
            }
        }
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
    fn opaque_members_share_one_back_to_front_order() {
        let g = group_with(json!({"segments":16}), vec![member(1, 0.0), member(2, 50.0)]);
        let art = revolve_art(&g).unwrap();
        assert_eq!(art.children().unwrap().len(), 1, "one order for every face");
        assert!(art.children().unwrap()[0].children().unwrap().len() > 16);
    }

    /// The opacities and blending modes of the groups a face is drawn in, outermost first, leaving
    /// out groups with neither.
    type Chain = Vec<(f32, vectorcraft_doc::color::BlendMode)>;

    /// The faces of `art` in drawing order, each with its [`Chain`].
    fn faces_in_order(art: &Node) -> Vec<(Node, Chain)> {
        fn visit(n: &Node, mut scopes: Chain, out: &mut Vec<(Node, Chain)>) {
            match n.children() {
                Some(children) => {
                    if n.opacity != 1.0 || n.blend != Default::default() {
                        scopes.push((n.opacity, n.blend));
                    }
                    for c in children {
                        visit(c, scopes.clone(), out);
                    }
                }
                None => out.push((n.clone(), scopes)),
            }
        }
        let mut out = vec![];
        visit(art, vec![], &mut out);
        out
    }

    #[test]
    fn a_transparent_member_leaves_every_face_in_one_back_to_front_order() {
        let faces_of = |members| faces_in_order(&revolve_art(&group_with(json!({"segments":16}), members)).unwrap());
        let alone = faces_of(vec![member(1, 0.0), member(2, 50.0)]);
        let mut faded = (*member(3, 20.0)).clone();
        faded.opacity = 0.5;
        let faces = faces_of(vec![member(1, 0.0), member(2, 50.0), Arc::new(faded)]);
        let opaque: Vec<_> = faces.iter().filter(|(_, s)| s.is_empty()).map(|(f, _)| f).collect();
        assert_eq!(opaque, alone.iter().map(|(f, _)| f).collect::<Vec<_>>(), "the opaque members' faces keep their shared order");
        assert!(faces.iter().all(|(_, s)| s.is_empty() || s == &[(0.5, Default::default())]), "the member's opacity applies once to each face");
        let first = faces.iter().position(|(_, s)| !s.is_empty()).unwrap();
        let last = faces.iter().rposition(|(_, s)| !s.is_empty()).unwrap();
        assert!(first > 0 && last + 1 < faces.len(), "the ring between the others draws in front of the back ones and behind the front ones");
        let groups = |art: &Node| {
            let mut count = 0;
            art.walk(&mut |n| count += usize::from(n.children().is_some() && n.opacity == 0.5));
            count
        };
        // Rings one above the other: their faces alternate in the order but none overlaps the
        // other ring, so each ring's opacity applies once, to all its faces.
        let faded = |id, y: f64| {
            let mut n = (*member(id, 40.0)).clone();
            n.opacity = 0.5;
            if let NodeKind::Path { path, .. } = &mut n.kind {
                *path = PathData::from_bezpath(&(Affine::translate((0.0, y)) * path.to_bezpath()));
            }
            Arc::new(n)
        };
        let art = revolve_art(&group_with(json!({"segments":16}), vec![faded(1, 0.0), faded(2, 200.0)])).unwrap();
        assert!(faces_in_order(&art).iter().all(|(_, s)| s == &[(0.5, Default::default())]));
        assert_eq!(groups(&art), 2, "one group for each ring");
        // A tall cylinder inside both that paints nothing splits neither.
        let mut hidden = (*member(3, 0.0)).clone();
        hidden.opacity = 0.0;
        if let NodeKind::Path { path, .. } = &mut hidden.kind {
            *path = PathData::from_bezpath(&(Affine::translate((0.0, -20.0)) * Affine::scale_non_uniform(1.0, 7.0) * path.to_bezpath()));
        }
        let art = revolve_art(&group_with(json!({"segments":16}), vec![faded(1, 0.0), faded(2, 200.0), Arc::new(hidden)])).unwrap();
        assert_eq!(groups(&art), 2, "one group for each faded ring");
    }

    #[test]
    fn a_member_whose_fill_or_stroke_paints_nothing_stays_invisible() {
        let clear_fill = {
            let mut n = (*member(3, 20.0)).clone();
            if let Some(AppearanceItem::Fill(f)) = n.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Fill(_))) {
                f.opacity = 0.0;
            }
            n
        };
        let clear_stroke = {
            let mut n = (*member(4, 20.0)).clone();
            n.appearance = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0).into();
            for item in n.appearance.items.iter_mut() {
                if let AppearanceItem::Stroke(s) = item {
                    s.opacity = 0.0;
                }
            }
            n
        };
        for clear in [clear_fill, clear_stroke] {
            let art = revolve_art(&group_with(json!({"segments":16}), vec![member(1, 0.0), member(2, 50.0), Arc::new(clear)])).unwrap();
            let faces = faces_in_order(&art);
            let alone = faces_in_order(&revolve_art(&group_with(json!({"segments":16}), vec![member(1, 0.0), member(2, 50.0)])).unwrap());
            let opaque: Vec<_> = faces.iter().filter(|(_, s)| s.is_empty()).map(|(f, _)| f).collect();
            assert_eq!(opaque, alone.iter().map(|(f, _)| f).collect::<Vec<_>>(), "only the opaque members' faces draw without transparency");
            assert!(faces.len() > alone.len(), "the clear member's faces stay in the art");
            assert!(faces.iter().all(|(_, s)| s.is_empty() || s.iter().any(|(o, _)| *o == 0.0)), "each clear face is drawn at 0% opacity");
        }
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
    fn faces_overlap_by_their_painted_shapes_and_seams_not_only_their_boxes() {
        let face = |points: &[(f64, f64)]| {
            let mut b = BezPath::new();
            b.move_to(points[0]);
            for &p in &points[1..] {
                b.line_to(p);
            }
            b.close_path();
            Coverage::of(&Node::path(NodeId(1), PathData::from_bezpath(&b), Appearance::default())).unwrap()
        };
        let a = face(&[(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)]);
        assert_eq!(a.hull.len(), 3);
        assert_eq!(
            face(&[(0.0, 0.0), (5.0, 0.0), (10.0, 0.0), (10.0, 10.0), (5.0, 5.0), (0.0, 10.0)]).hull.len(),
            4,
            "a point on an edge or inside is left out"
        );
        // Across the diagonal 2 / sqrt(2) apart: their boxes overlap, their shapes and seams don't.
        let b = face(&[(10.0, 10.0), (10.0, 2.0), (2.0, 10.0)]);
        assert!(overlaps(a.bounds, b.bounds) && !a.meets(&b) && !b.meets(&a));
        // 1 / sqrt(2) apart, within the two half seam strokes.
        let c = face(&[(10.0, 10.0), (10.0, 1.0), (1.0, 10.0)]);
        assert!(a.meets(&c) && c.meets(&a));
        assert!(a.meets(&a), "a face meets itself");
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
    fn blending_modes_apply_once_to_the_member_or_group_that_has_them() {
        use vectorcraft_doc::color::BlendMode;
        let blend = |n: Arc<Node>, mode: BlendMode| {
            let mut n = (*n).clone();
            n.blend = mode;
            Arc::new(n)
        };
        let inner = blend(member(4, 60.0), BlendMode::Multiply);
        let screened = blend(Arc::new(Node::group(NodeId(5), vec![member(3, 40.0), inner])), BlendMode::Screen);
        let group = group_with(json!({}), vec![blend(member(1, 0.0), BlendMode::Multiply), member(2, 20.0), screened]);
        let art = revolve_art(&group).expect("revolved");
        let faces = faces_in_order(&art);
        let modes = |s: &[(f32, BlendMode)]| s.iter().map(|(_, b)| *b).collect::<Vec<_>>();
        let chains = [vec![BlendMode::Multiply], vec![], vec![BlendMode::Screen], vec![BlendMode::Screen, BlendMode::Multiply]];
        for chain in &chains {
            assert!(faces.iter().any(|(_, s)| &modes(s) == chain), "faces drawn in {chain:?}");
        }
        assert!(faces.iter().all(|(_, s)| chains.contains(&modes(s))), "each mode applies once, to the member or group that has it");
        // Faces of the screened group's members that come together in the drawing order share one Screen group.
        let runs = |faces: &[(Node, Chain)], inside: &dyn Fn(&Chain) -> bool| {
            faces.windows(2).filter(|w| inside(&w[1].1) && !inside(&w[0].1)).count() + usize::from(faces.first().is_some_and(|f| inside(&f.1)))
        };
        let groups = |art: &Node, pick: &dyn Fn(&Node) -> bool| {
            let mut count = 0;
            art.walk(&mut |n| count += usize::from(n.children().is_some() && pick(n)));
            count
        };
        let screened = |s: &Chain| s.first().is_some_and(|(_, b)| *b == BlendMode::Screen);
        assert_eq!(groups(&art, &|n| n.blend == BlendMode::Screen), runs(&faces, &screened));
        let plain = group_with(json!({}), vec![member(1, 0.0)]);
        assert_ne!(expand_revolve_art(&plain), revolve_art(&plain), "an opaque member's hidden faces are removed");
        let multiplied = group_with(json!({}), vec![blend(member(1, 0.0), BlendMode::Multiply)]);
        assert_eq!(expand_revolve_art(&multiplied), revolve_art(&multiplied), "a blended member keeps every face");
        // A nested group's opacity applies once, to its members' faces that come together.
        let mut faded = Node::group(NodeId(6), vec![member(3, 40.0), member(4, 45.0)]);
        faded.opacity = 0.5;
        let art = revolve_art(&group_with(json!({}), vec![member(1, 0.0), Arc::new(faded)])).expect("revolved");
        let faces = faces_in_order(&art);
        assert!(faces.iter().all(|(_, s)| s.is_empty() || s == &[(0.5, BlendMode::Normal)]), "the faded group's opacity applies once");
        let half = |s: &Chain| !s.is_empty();
        assert!(faces.iter().any(|(_, s)| half(s)) && faces.iter().any(|(_, s)| !half(s)));
        assert_eq!(groups(&art, &|n| n.opacity == 0.5), runs(&faces, &half), "the faded group's members, together");
    }

    #[test]
    fn a_3d_effect_on_a_groups_fill_is_refused() {
        let mut group = Node::group(NodeId(9), vec![member(1, 0.0)]);
        group.appearance = Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0).into();
        for id in [REVOLVE, crate::EXTRUDE] {
            assert!(crate::validate_three_d(&group, id, &json!({})).is_ok(), "{id} on the group itself");
            let refused = crate::validate_three_d_at(&group, id, &json!({}), (Some(0), 0)).unwrap_err();
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
    fn validation_of_a_hidden_revolve_leaves_out_the_geometry_effects_after_it() {
        let roughen = crate::new_effect("distort.roughen", &json!({"size": 1, "detail": 100})).unwrap();
        let mut hidden = crate::new_effect(REVOLVE, &json!({})).unwrap();
        hidden.visible = false;
        let mut object = (*star(1)).clone();
        object.appearance.effects = vec![hidden.clone(), roughen.clone()];
        assert_eq!(validate_revolve(&object, &json!({})), Ok(()), "an object whose Revolve is shown again");
        let mut group = group_with(json!({}), vec![star(1)]);
        group.appearance.effects = vec![hidden, roughen];
        assert_eq!(validate_revolve(&group, &json!({})), Ok(()), "a group whose Revolve is shown again");
        for n in [&mut object, &mut group] {
            n.appearance.effects[0].visible = true;
            assert!(revolve_art(n).is_some(), "shown, the Revolve draws");
        }
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
    fn the_art_of_symbol_instances_and_blends_shares_the_budget_of_paint_copies() {
        use vectorcraft_doc::color::{Gradient, GradientKind, GradientPaint, GradientStop};
        use vectorcraft_doc::{BlendSpacing, BlendSpec, Symbol};
        let plain = member(1, 0.0);
        let NodeKind::Path { path: profile, .. } = &plain.kind else { panic!("a path") };
        let faces = vectorcraft_three_d::revolve(profile, profile.bounds().unwrap(), revolve_options(&json!({}))).unwrap().len();
        let stops = MAX_PAINT_COPIES * 6 / 10 / (faces * 4) / std::mem::size_of::<GradientStop>();
        let painted = |id: u64, x: f64| {
            let stops = (0..stops).map(|i| GradientStop::new(i as f32 / stops as f32, Color::BLACK)).collect();
            let paint = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
            let mut node = (*member(id, x)).clone();
            node.appearance = Appearance::basic(paint, Paint::None, 0.0).into();
            Arc::new(node)
        };
        let mut d = Document::new(500.0, 500.0);
        d.symbols.push(Symbol { dynamic: false, name: "P".into(), art: painted(0, 0.0) });
        let instance = |id: u64| Arc::new(Node::new(NodeId(id), NodeKind::SymbolInstance { symbol: "P".into(), xf: Affine::IDENTITY }));
        // One instance's art fits; two in the same place, each of whose surfaces fits alone, pass
        // the budget together, as two paths would.
        assert_eq!(validate_revolve_in(Some(&d), &group_with(json!({}), vec![instance(1)]), &json!({})), Ok(()));
        let two = group_with(json!({}), vec![instance(1), instance(2)]);
        let refused = validate_revolve_in(Some(&d), &two, &json!({})).unwrap_err();
        assert!(refused.contains("simplify the gradient"), "{refused}");
        assert_eq!(revolve_art_in(Some(&d), &two), None, "past the budget the source draws");
        // A blend's keys and step count together too.
        let spec = Box::new(BlendSpec { spacing: BlendSpacing::Steps(1), ..Default::default() });
        let blend = Arc::new(Node::new(NodeId(3), NodeKind::Blend { children: vec![painted(4, 0.0), painted(5, 0.0)], spec }));
        let refused = validate_revolve_in(Some(&d), &group_with(json!({}), vec![blend]), &json!({})).unwrap_err();
        assert!(refused.contains("simplify the gradient"), "{refused}");
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
