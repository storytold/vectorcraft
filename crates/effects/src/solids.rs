//! Extrude & Bevel, Inflate, Rotate and Extrude & Bevel (Classic) as live, shaded vector art,
//! shared by the canvas and every exporter. Like Revolve, the source path stays editable and the effect draws its 3D faces.
use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::{BlendMode, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Effect, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{FillRule, PathData, Rect};
use vectorcraft_three_d::{Bevel, BevelShape, Extrude, Facet, Inflate, Mesh, View};

#[cfg(test)]
use vectorcraft_doc::color::Color;

use crate::util::{flag, num, text};
use crate::{GeomContext, apply_geometry_with, is_geometry};

mod classic;
pub use classic::{CLASSIC_EXTRUDE, SHADING_COLORS, classic_extrude_options, classic_shading};

pub const EXTRUDE: &str = "threeD.extrude";
pub const INFLATE: &str = "threeD.inflate";
pub const ROTATE: &str = "threeD.rotate";
/// The 3D effects this module draws (Revolve has its own).
pub const SOLIDS: [&str; 4] = [EXTRUDE, INFLATE, ROTATE, CLASSIC_EXTRUDE];

/// Is `id` one of the 3D and Materials effects (Revolve included)?
pub fn is_three_d(id: &str) -> bool {
    id == crate::REVOLVE || SOLIDS.contains(&id)
}

/// The 3D and Materials panel's object types and the effect that draws each: (type, effect id).
/// Plane keeps the art flat, which is what Rotate draws.
pub const THREE_D_TYPES: [(&str, &str); 4] = [("plane", ROTATE), ("extrude", EXTRUDE), ("revolve", crate::REVOLVE), ("inflate", INFLATE)];

/// The parameters of 3D effect `to` for an effect `from` with `params`, as switching the object
/// type keeps them: every setting `to` takes keeps its value from `params` (the defaults of `from`
/// under them), each rotation angle wrapped into -180° to 180° when `to` is not Revolve, and the
/// settings only `to` takes start at their defaults. Settings `to` does not take are dropped.
pub fn three_d_params_as(from: &str, params: &Value, to: &str) -> Value {
    let current = crate::merged_params(from, params);
    let mut out = crate::default_params(to).and_then(|v| v.as_object().cloned()).unwrap_or_default();
    for (key, value) in out.iter_mut() {
        if let Some(v) = current.get(key) {
            value.clone_from(v);
        }
    }
    if to != crate::REVOLVE {
        for key in ["rotationX", "rotationY", "rotationZ"] {
            if let Some(a) = out.get(key).and_then(Value::as_f64).filter(|a| !(-180.0..=180.0).contains(a)) {
                out.insert(key.into(), serde_json::json!((a + 180.0).rem_euclid(360.0) - 180.0));
            }
        }
    }
    Value::Object(out)
}

fn solid(fx: &[Effect]) -> Option<(usize, &Effect)> {
    fx.iter().enumerate().find(|(_, e)| e.visible && SOLIDS.contains(&e.id.as_str()))
}

/// Does `n` carry a visible Extrude & Bevel, Inflate or Rotate effect?
pub fn has_solid(n: &Node) -> bool {
    solid(&n.appearance.effects).is_some() || n.appearance.items.iter().any(|i| i.visible() && solid(i.effects()).is_some())
}

/// The Position presets: (id, [rotation X, Y, Z]). Front, Back, Left, Right, Top and Bottom face
/// the art that way; Off-Axis tilts each 20° down and 25° aside (showing the top and right);
/// Isometric lays the art on the left, right, top or bottom plane of an isometric view (the
/// visible axes foreshortened equally: atan(1/√2) ≈ 35.26° and 45°). Our own values, from the
/// geometry of each view.
pub const ROTATION_PRESETS: [(&str, [f64; 3]); 16] = [
    ("front", [0.0, 0.0, 0.0]),
    ("back", [0.0, 180.0, 0.0]),
    ("left", [0.0, 90.0, 0.0]),
    ("right", [0.0, -90.0, 0.0]),
    ("top", [-90.0, 0.0, 0.0]),
    ("bottom", [90.0, 0.0, 0.0]),
    ("offAxisFront", [-21.88, -23.4, 9.06]),
    ("offAxisBack", [-158.12, 23.4, -170.94]),
    ("offAxisLeft", [-40.74, 58.39, -36.26]),
    ("offAxisRight", [-139.26, -58.39, 143.74]),
    ("offAxisTop", [-111.88, -23.4, 9.06]),
    ("offAxisBottom", [68.12, -23.4, 9.06]),
    ("isometricLeft", [-45.0, -35.26, 30.0]),
    ("isometricRight", [-45.0, 35.26, -30.0]),
    ("isometricTop", [45.0, -35.26, 30.0]),
    ("isometricBottom", [-45.0, -35.26, -30.0]),
];

/// The view parameters every 3D effect shares, named as Revolve's.
pub fn view_options(p: &Value) -> View {
    View {
        rotation_x: num(p, "rotationX", ROTATION_PRESETS[6].1[0]),
        rotation_y: num(p, "rotationY", ROTATION_PRESETS[6].1[1]),
        rotation_z: num(p, "rotationZ", ROTATION_PRESETS[6].1[2]),
        perspective: num(p, "perspectiveAngle", 0.0),
        light_rotation: num(p, "lightRotation", vectorcraft_three_d::DEFAULT_LIGHT.0),
        light_height: num(p, "lightHeight", vectorcraft_three_d::DEFAULT_LIGHT.1),
        light_behind: flag(p, "lightBehind", false),
        light_color: crate::lighting::light_color(p),
        lights: crate::lighting::lights(p),
        light_intensity: num(p, "lightIntensity", 80.0) / 100.0,
        light_softness: num(p, "lightSoftness", 0.0) / 100.0,
        ambient: num(p, "ambient", 25.0) / 100.0,
        shade: flag(p, "shade", true),
    }
}

/// Extrude & Bevel's parameters. `bevel` is `"none"` or a [`BevelShape`] id.
pub fn extrude_options(p: &Value) -> Extrude {
    Extrude {
        depth: num(p, "depth", 50.0),
        twist: num(p, "twist", 0.0),
        taper: num(p, "taper", 100.0) / 100.0,
        cap: flag(p, "cap", true),
        bevel: BevelShape::parse(text(p, "bevel", "none")).map(|shape| Bevel {
            shape,
            width: num(p, "bevelWidth", 4.0),
            height: num(p, "bevelHeight", 4.0),
            repeat: num(p, "bevelRepeat", 1.0).clamp(1.0, 32.0) as usize,
            space: num(p, "bevelSpace", 0.0) / 100.0,
            inside: flag(p, "bevelInside", false),
            both_sides: flag(p, "bevelBothSides", false),
        }),
        hidden: false,
    }
}

pub fn inflate_options(p: &Value) -> Inflate {
    Inflate {
        depth: num(p, "depth", 0.0),
        volume: num(p, "volume", 50.0),
        both_sides: flag(p, "bothSides", false),
        twist: num(p, "twist", 0.0),
        taper: num(p, "taper", 100.0) / 100.0,
        cap: flag(p, "cap", true),
    }
}

/// The shape a solid is made from: a path's or a compound path's outline.
pub(crate) fn profile(n: &Node) -> Option<(PathData, FillRule)> {
    match &n.kind {
        NodeKind::Path { path, rule, guide: false, clipping: false, .. } => Some((path.clone(), *rule)),
        NodeKind::Compound { children, rule } => {
            Some((PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect()), *rule))
        }
        // Type extrudes as its glyph outlines and stays type: editing it reshapes the solid (#4715).
        NodeKind::Text(_) => {
            let mut subpaths = vec![];
            crate::outline_text(n)?.walk(&mut |c| subpaths.extend(c.path_data().into_iter().flat_map(|p| p.subpaths.iter().cloned())));
            (!subpaths.is_empty()).then(|| (PathData::new(subpaths), FillRule::NonZero))
        }
        _ => None,
    }
}

/// What a 3D effect paints `n` with: its own fills and strokes, else, for type that has none, the
/// fill and stroke of its first painted run (its characters' colours).
pub(crate) fn solid_paints(n: &Node) -> Vec<AppearanceItem> {
    let own: Vec<AppearanceItem> = n.appearance.items.iter().filter(|i| crate::paints(i)).cloned().collect();
    if !own.is_empty() || !matches!(n.kind, NodeKind::Text(_)) {
        return own;
    }
    let mut runs = vec![];
    if let Some(outlined) = crate::outline_text(n) {
        outlined.walk(&mut |c| {
            if runs.is_empty() && c.path_data().is_some() {
                runs = c.appearance.items.iter().filter(|i| crate::paints(i)).cloned().collect();
            }
        });
    }
    runs
}

fn facets(id: &str, path: &PathData, rule: FillRule, bounds: Rect, p: &Value) -> Result<Vec<Facet>, String> {
    let view = view_options(p);
    match id {
        EXTRUDE => vectorcraft_three_d::extrude(path, rule, bounds, extrude_options(p), view),
        CLASSIC_EXTRUDE => vectorcraft_three_d::extrude(path, rule, bounds, classic_extrude_options(p), view),
        INFLATE => vectorcraft_three_d::inflate(path, rule, bounds, inflate_options(p), view),
        _ => vectorcraft_three_d::plane(path, rule, bounds, view),
    }
}

/// Check solid `id`'s parameter values `p` alone: the part of [`validate_solid`] that does not
/// need the object.
fn validate_solid_params(id: &str, p: &Value) -> Result<(), String> {
    if id == CLASSIC_EXTRUDE {
        classic::validate(p)?;
    }
    crate::lighting::validate(p)?;
    if let Some(v) = p.get("bevel")
        && !v.as_str().is_some_and(|s| s == "none" || BevelShape::parse(s).is_some())
    {
        let names: Vec<_> = BevelShape::ALL.iter().map(|b| b.id()).collect();
        return Err(format!("bevel must be none or one of {}", names.join(", ")));
    }
    Ok(())
}

/// Validate before a command changes the document; the evaluator bounds loaded files too.
pub fn validate_solid(n: &Node, id: &str, p: &Value) -> Result<(), String> {
    validate_solid_params(id, p)?;
    let label = crate::effect_info(id).map_or("This effect", |e| e.label.trim_end_matches('…'));
    if is_container(n) {
        let shapes = members(n, label)?;
        let b = union_bounds(&shapes).ok_or_else(|| format!("{label} requires a group holding a shape with at least two anchors"))?;
        return shapes.iter().try_for_each(|s| facets(id, &s.path, s.rule, b, p).map(|_| ()));
    }
    let (path, rule) = object_profile(n, label)?;
    let b = path.bounds().ok_or_else(|| format!("{label} requires a shape with at least two anchors"))?;
    facets(id, &path, rule, b, p).map(|_| ())
}

/// [`profile`], or the error a command reports for object `n`: type with no outlines to build from
/// (empty, or only spaces) is a shape without anchors, and any other object is the wrong kind.
pub(crate) fn object_profile(n: &Node, label: &str) -> Result<(PathData, FillRule), String> {
    profile(n).ok_or_else(|| {
        if matches!(n.kind, NodeKind::Text(_)) {
            format!("{label} requires a shape with at least two anchors")
        } else {
            format!("{label} currently requires a path, compound path, type or group. Convert images and other objects to paths first.")
        }
    })
}

/// One shape a solid is built from: its outline and fill rule, the fills and strokes that paint
/// it, and the opacity of the objects it is in (1 for the object with the effect itself).
pub(crate) struct Profile {
    pub(crate) path: PathData,
    pub(crate) rule: FillRule,
    pub(crate) items: Vec<AppearanceItem>,
    pub(crate) opacity: f32,
}

/// Is `n` a group or layer (a 3D effect on it turns its members about one shared axis)?
pub(crate) fn is_container(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { clip: false, .. } | NodeKind::Layer { .. })
}

/// The bounds of every shape together: the centre a group's members turn about.
pub(crate) fn union_bounds(shapes: &[Profile]) -> Option<Rect> {
    shapes.iter().filter_map(|s| s.path.bounds()).reduce(|a, c| a.union(c))
}

/// The most members a 3D effect on a group builds solids from.
const MAX_MEMBERS: usize = 4096;

/// The most triangles the meshes of one 3D effect hold together (about 290 MB of corners and
/// normals): past it the object is too complex to export.
const MAX_MESH_TRIANGLES: usize = 2_000_000;

/// The shapes a 3D effect on group or layer `n` builds its solids from: every visible path,
/// compound path and type in it, at any depth of its groups, each through its own geometry
/// effects and painted with its own fills and strokes. Like the effect on a group in the
/// reference app, they turn about one shared axis (the centre of their bounds together) rather
/// than each about its own. An error names what the group holds that can't become a solid.
pub(crate) fn members(n: &Node, label: &str) -> Result<Vec<Profile>, String> {
    fn walk(n: &Node, opacity: f32, label: &str, out: &mut Vec<Profile>) -> Result<(), String> {
        for c in n.children().into_iter().flatten() {
            if !c.visible || matches!(c.kind, NodeKind::Path { guide: true, .. }) {
                continue;
            }
            let opacity = opacity * c.opacity;
            if is_container(c) {
                walk(c, opacity, label, out)?;
                continue;
            }
            let Some((path, rule)) = profile(c) else {
                let what = match &c.kind {
                    NodeKind::Group { clip: true, .. } => "clipping groups",
                    NodeKind::Image(_) => "images",
                    NodeKind::SymbolInstance { .. } => "symbol instances",
                    NodeKind::Path { clipping: true, .. } => "clipping paths",
                    _ => "objects other than paths, compound paths and type",
                };
                return Err(format!("{label} on a group needs paths, compound paths and type: release or expand its {what} first."));
            };
            if out.len() >= MAX_MEMBERS {
                return Err(format!("{label} on a group takes at most {MAX_MEMBERS} objects"));
            }
            let path = geometry(&path, &c.appearance.effects, &GeomContext::of(c));
            if path.bounds().is_some() {
                out.push(Profile { path, rule, items: solid_paints(c), opacity });
            }
        }
        Ok(())
    }
    let mut out = vec![];
    walk(n, 1.0, label, &mut out)?;
    Ok(out)
}

/// Can 3D effect `id` (Revolve included, [`is_three_d`]) with parameters `p` draw `n`? Checked
/// before a command changes the document: Revolve by [`crate::validate_revolve`], the others by
/// [`validate_solid`].
pub fn validate_three_d(n: &Node, id: &str, p: &Value) -> Result<(), String> {
    if id == crate::REVOLVE { crate::validate_revolve(n, p) } else { validate_solid(n, id, p) }
}

/// [`validate_three_d`] for an effect at `(item, index)`, index `index` of the effects of
/// appearance item `item` of `n` (the effects' length for a new effect), or on the object itself
/// when `at` is `None`. A Revolve on a fill or stroke is checked in that item's paint, after the
/// item's geometry effects before it, together with the object's other items that carry a Revolve.
pub fn validate_three_d_at(n: &Node, id: &str, p: &Value, at: Option<(usize, usize)>) -> Result<(), String> {
    match at {
        Some((item, index)) if id == crate::REVOLVE => crate::revolve::validate_item_revolve(n, p, item, index),
        _ => validate_three_d(n, id, p),
    }
}

/// Are 3D effect `id`'s parameter values `p` valid? The checks of [`validate_three_d`] that do not
/// build the art, for an edit that hides the effect: hidden, it draws nothing, so its object does
/// not need to fit.
pub fn validate_three_d_params(id: &str, p: &Value) -> Result<(), String> {
    if id == crate::REVOLVE { crate::revolve::validate_revolve_params(p) } else { validate_solid_params(id, p) }
}

/// One surface: each facet a path shaded from `paint`, as a group with the paint's opacity and
/// blend mode (so overlapping facets don't add up their transparency).
fn surface(n: &Node, facets: Vec<Facet>, paint: &Paint, opacity: f32, blend: BlendMode, colored: bool) -> Node {
    if colored && paint.color().is_some() && facets.iter().any(|face| face.rgb_gradient.is_some()) {
        let mut group = crate::lighting::solid_surface(n.id, &facets, paint);
        group.opacity = opacity;
        group.blend = blend;
        return group;
    }
    // Preserve existing white-light paints, alpha composition and seam joins exactly.
    let (opaque, mask) = if colored { crate::lighting::opaque_paint(paint) } else { (paint.clone(), None) };
    let paint = &opaque;
    let children = facets
        .into_iter()
        .map(|face| {
            // Adjacent facets, antialiased on their own, would leave hairline seams: a narrow
            // stroke of the same paint closes them. No wider than the facet is thick, though: seen
            // at a grazing angle a facet can be thinner than the stroke, and its stroke would then
            // paint over its neighbours.
            let seam = (face.thickness() / 2.0).clamp(0.05, 0.75);
            if let Some(g) = face.rgb_gradient
                && (paint.color().is_some() || matches!(paint, Paint::Gradient(_)))
            {
                return Arc::new(crate::lighting::facet_art(n.id, &face.path(), paint, g, seam));
            }
            let items = crate::lighting::fills(paint, face.illumination, face.brightness, colored);
            let items = items
                .into_iter()
                .flat_map(|item| {
                    let AppearanceItem::Fill(fill) = &item else { return vec![item] };
                    let mut edge = StrokeLayer::new(fill.paint.clone(), seam);
                    if colored {
                        edge.join = vectorcraft_doc::LineJoin::Round;
                    }
                    edge.opacity = fill.opacity;
                    vec![item, AppearanceItem::Stroke(edge)]
                })
                .collect();
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

/// `item`'s paint, its opacity times `member` (the opacity of the objects it is in) and its blend.
fn paint_of(item: &AppearanceItem, member: f32) -> (Paint, f32, BlendMode) {
    match item {
        AppearanceItem::Fill(f) => (f.paint.clone(), f.opacity * member, f.blend),
        AppearanceItem::Stroke(s) => (s.paint.clone(), s.opacity * member, s.blend),
    }
}

/// The area `item` paints on `path`, and the rule that fills it: the path itself for a fill, the
/// stroke's outline (whose contours nest as its holes do) for a stroke.
fn region(path: &PathData, rule: FillRule, item: &AppearanceItem) -> (PathData, FillRule) {
    match item {
        AppearanceItem::Stroke(s) if s.width > 0.0 && s.width.is_finite() => (crate::stroke::outline_region(path, rule, s), FillRule::EvenOdd),
        _ => (path.clone(), rule),
    }
}

/// The parts Extrude & Bevel and Inflate build solids from: each shape's top fill and its strokes,
/// a group's members from the back up, each with its shape's opacity and the region it paints.
/// The solids share the bounds of all their parts (`fallback` when none has any): one center to
/// turn, twist and taper about, and Inflate's grid reaches over the strokes' outlines.
fn solid_parts(shapes: &[Profile], fallback: Rect) -> (Vec<(&AppearanceItem, f32, PathData, FillRule)>, Rect) {
    let parts: Vec<(&AppearanceItem, f32, PathData, FillRule)> = shapes
        .iter()
        .flat_map(|s| {
            let top_fill = s.items.iter().rposition(|i| matches!(i, AppearanceItem::Fill(_)));
            s.items.iter().enumerate().filter(move |(k, i)| Some(*k) == top_fill || matches!(i, AppearanceItem::Stroke(_))).map(|(_, i)| {
                let (area, rule) = region(&s.path, s.rule, i);
                (i, s.opacity, area, rule)
            })
        })
        .collect();
    let bounds = parts.iter().filter_map(|(_, _, area, _)| area.bounds()).reduce(|a, c| a.union(c)).unwrap_or(fallback);
    (parts, bounds)
}

/// `shapes`, the members of group or layer `n`, reshaped by the visible geometry effects `before`
/// its 3D effect, about the bounds of all of them.
pub(crate) fn reshape_members(n: &Node, before: &[Effect], shapes: &mut [Profile]) {
    if before.iter().any(|e| e.visible && is_geometry(&e.id))
        && let Some(b) = union_bounds(shapes)
    {
        let ctx = GeomContext::of(n);
        for s in shapes {
            s.path = apply_geometry_with(before, &s.path, b, &ctx);
        }
    }
}

/// The 3D art of one effect `fx` on `path` (already through the effects before it).
/// Rotate turns every fill and stroke, each in its own paint. Extrude & Bevel and Inflate build a
/// solid from the path for the top visible fill and one from the outline of each visible stroke,
/// each in its own paint. When every one is opaque with normal blending and no colored light makes
/// a surface one composite or masks it, their faces are drawn in one back-to-front order (where
/// depths tie, as for the caps turned toward the viewer, in the order of the appearance);
/// otherwise each solid is a surface of its own, in that order.
fn art_for(n: &Node, fx: &Effect, shapes: &[Profile]) -> Option<Vec<Arc<Node>>> {
    let b = union_bounds(shapes)?;
    let p = crate::merged_params(&fx.id, &fx.params);
    let classic = fx.id == CLASSIC_EXTRUDE;
    let colored = !classic && view_options(&p).has_colored_lights();
    if fx.id == ROTATE {
        // Every part turns about the centre of the object's own bounds (a group's members about
        // the centre of theirs), so they stay together.
        let mut out = vec![];
        for s in shapes {
            for item in &s.items {
                let (paint, opacity, blend) = paint_of(item, s.opacity);
                let (area, rule) = region(&s.path, s.rule, item);
                let f = facets(ROTATE, &area, rule, b, &p).ok()?;
                out.push(Arc::new(surface(n, f, &paint, opacity, blend, colored)));
            }
        }
        return Some(out);
    }
    let (parts, bounds) = solid_parts(shapes, b);
    let built = if fx.id == INFLATE {
        // One eye for every puff, as far away as the highest needs.
        let shapes: Vec<(&PathData, FillRule)> = parts.iter().map(|(_, _, area, rule)| (area, *rule)).collect();
        vectorcraft_three_d::inflate_together(&shapes, bounds, inflate_options(&p), view_options(&p))
    } else {
        parts.iter().map(|(_, _, area, rule)| facets(&fx.id, area, *rule, bounds, &p)).collect()
    };
    let mut solids = vec![];
    for ((item, opacity, _, _), f) in parts.iter().zip(built) {
        let item: &AppearanceItem = item;
        match f {
            Ok(f) => solids.push((item, *opacity, f)),
            // A stroke's outline past the limits is left out; the fill's solid still draws.
            Err(_) if matches!(item, AppearanceItem::Stroke(_)) => {}
            // A loaded file or a shape edited past the limits keeps drawing its source path
            // instead of failing the frame; commands validate the same evaluator and report its
            // errors.
            Err(_) => return None,
        }
    }
    if solids.is_empty() {
        return None;
    }
    let opaque = solids.iter().all(|(item, member, _)| {
        let (_, opacity, blend) = paint_of(item, *member);
        opacity >= 1.0 && blend == BlendMode::Normal
    });
    let surfaces: Vec<(Vec<f64>, Node)> = solids
        .into_iter()
        .map(|(item, member, f)| {
            let depths: Vec<f64> = f.iter().map(|f| f.depth).collect();
            let (paint, opacity, blend) = paint_of(item, member);
            let art = if classic { classic::surface(n, f, &paint, opacity, blend, &p) } else { surface(n, f, &paint, opacity, blend, colored) };
            (depths, art)
        })
        .collect();
    // One order takes a plain group of faces from each solid, one face for each facet. With
    // colored lights a surface can instead be one clipped composite of all its facets, or carry an
    // opacity mask, and then it stays whole.
    let plain = surfaces.iter().all(|(depths, s)| {
        s.extras().mask.is_none() && matches!(&s.kind, NodeKind::Group { children, clip: false } if children.len() == depths.len())
    });
    if !opaque || !plain {
        return Some(surfaces.into_iter().map(|(_, s)| Arc::new(s)).collect());
    }
    let mut faces = vec![];
    for (depths, s) in surfaces {
        if let NodeKind::Group { children, .. } = s.kind {
            faces.extend(depths.into_iter().zip(children));
        }
    }
    // A stable sort: faces at the same depth keep the order of the appearance.
    faces.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(vec![Arc::new(Node::group(n.id, faces.into_iter().map(|(_, face)| face).collect()))])
}

fn geometry(path: &PathData, fx: &[Effect], ctx: &GeomContext) -> PathData {
    apply_geometry_with(fx, path, path.bounds().unwrap_or(Rect::ZERO), ctx)
}

/// `n` drawn with its Extrude & Bevel, Inflate or Rotate effect: a group of shaded faces keeping
/// the object's id and transparency, the source path untouched. `None` without such an effect.
/// On a group or layer the effect builds a solid from each path, compound path and type in it,
/// all turned about one shared axis ([`members`]).
pub fn solid_art(n: &Node) -> Option<Node> {
    if !has_solid(n) {
        return None;
    }
    if is_container(n) {
        return group_solid_art(n);
    }
    let (path, rule) = profile(n)?;
    let ctx = GeomContext::of(n);
    let paints = solid_paints(n);
    let mut children = vec![];
    let remaining: Vec<Effect>;
    if let Some((index, fx)) = solid(&n.appearance.effects) {
        let path = geometry(&path, n.appearance.effects.get(..index)?, &ctx);
        children.extend(art_for(n, fx, &[Profile { path, rule, items: paints, opacity: 1.0 }])?);
        // Later geometry effects act on the projected art as a whole.
        remaining = n
            .appearance
            .effects
            .iter()
            .enumerate()
            .filter(|(i, e)| *i != index && (!is_geometry(&e.id) || *i > index))
            .map(|(_, e)| e.clone())
            .collect();
    } else {
        let path = geometry(&path, &n.appearance.effects, &ctx);
        remaining = n.appearance.effects.iter().filter(|e| !is_geometry(&e.id)).cloned().collect();
        for item in &paints {
            if let Some((index, fx)) = solid(item.effects()) {
                let p = geometry(&path, item.effects().get(..index)?, &ctx.item(item));
                for mut art in art_for(n, fx, &[Profile { path: p, rule, items: vec![item.clone()], opacity: 1.0 }])? {
                    Arc::make_mut(&mut art).appearance.effects =
                        item.effects().iter().skip(index + 1).filter(|e| !is_three_d(&e.id)).cloned().collect();
                    children.push(art);
                }
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

/// [`solid_art`] of group or layer `n`: its object-level 3D effect on its members together. The
/// geometry effects before it reshape the members, those after it bend the result as a whole; the
/// group's own fills and strokes are not drawn. `None` when the effect is only on one of the
/// group's own fills or strokes, or when its members can't become solids.
fn group_solid_art(n: &Node) -> Option<Node> {
    let (index, fx) = solid(&n.appearance.effects)?;
    let mut shapes = members(n, "").ok()?;
    reshape_members(n, n.appearance.effects.get(..index)?, &mut shapes);
    let children = art_for(n, fx, &shapes)?;
    let remaining =
        n.appearance.effects.iter().enumerate().filter(|(i, e)| *i != index && (!is_geometry(&e.id) || *i > index)).map(|(_, e)| e.clone()).collect();
    let mut art = n.clone();
    art.kind = NodeKind::Group { children, clip: false };
    art.appearance = Appearance { effects: remaining, ..Default::default() }.into();
    Some(art)
}

/// One solid of a 3D object as a mesh, with the color it is painted.
#[derive(Clone, Debug, PartialEq)]
pub struct SolidMesh {
    pub mesh: Mesh,
    /// The paint's display color (sRGB, 0..1) and its opacity: a gradient's stops averaged, gray
    /// for a pattern.
    pub color: [f32; 4],
}

/// The color a mesh of `paint` at `opacity` takes ([`SolidMesh::color`]).
pub(crate) fn mesh_color(paint: &Paint, opacity: f32) -> [f32; 4] {
    let (rgb, alpha) = match paint {
        Paint::Solid { color, .. } => (color.to_rgb(), 1.0),
        Paint::Gradient(g) if !g.gradient.stops.is_empty() => {
            let n = g.gradient.stops.len() as f32;
            let mut sum = [0.0f32; 4];
            for s in &g.gradient.stops {
                let [r, gr, b] = s.color.to_rgb();
                sum = [sum[0] + r, sum[1] + gr, sum[2] + b, sum[3] + s.opacity];
            }
            ([sum[0] / n, sum[1] / n, sum[2] / n], sum[3] / n)
        }
        _ => ([0.5; 3], 1.0),
    };
    let unit = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    [unit(rgb[0]), unit(rgb[1]), unit(rgb[2]), unit(alpha * opacity)]
}

/// Add `mesh` to `out`, counting its triangles into `total`, or an error when that passes `limit`.
pub(crate) fn keep_mesh(out: &mut Vec<SolidMesh>, total: &mut usize, mesh: SolidMesh, limit: usize) -> Result<(), String> {
    *total = total.saturating_add(mesh.mesh.triangles.len());
    if *total > limit {
        return Err("The 3D object is too complex to export".into());
    }
    out.push(mesh);
    Ok(())
}

/// The solids effect `fx` builds from `shapes`, as [`art_for`] draws them, as meshes: Rotate turns
/// every fill and stroke; Extrude & Bevel and Inflate build one from each shape's top fill and one
/// from the outline of each stroke. A stroke whose solid is past the limits is left out. Each mesh
/// is counted against `limit` ([`keep_mesh`]) before the next is built.
fn meshes_for(fx: &Effect, shapes: &[Profile], total: &mut usize, limit: usize) -> Result<Vec<SolidMesh>, String> {
    let label = crate::effect_info(&fx.id).map_or("This effect", |e| e.label.trim_end_matches('…'));
    let b = union_bounds(shapes).ok_or_else(|| format!("{label} requires a shape with at least two anchors"))?;
    let p = crate::merged_params(&fx.id, &fx.params);
    let view = view_options(&p);
    let mut out = vec![];
    let mut keep = |out: &mut Vec<SolidMesh>, mesh: Mesh, color: [f32; 4]| keep_mesh(out, total, SolidMesh { mesh, color }, limit);
    if fx.id == ROTATE {
        for s in shapes {
            for item in &s.items {
                let (paint, opacity, _) = paint_of(item, s.opacity);
                let (area, rule) = region(&s.path, s.rule, item);
                keep(&mut out, vectorcraft_three_d::plane_mesh(&area, rule, b, view)?, mesh_color(&paint, opacity))?;
            }
        }
        return Ok(out);
    }
    let (parts, bounds) = solid_parts(shapes, b);
    // Inflate lays out every puff at once (they share one eye); each Inflate and Extrude & Bevel
    // solid and its mesh are built as the loop reaches them.
    let mut inflated = (fx.id == INFLATE).then(|| {
        let shapes: Vec<(&PathData, FillRule)> = parts.iter().map(|(_, _, area, rule)| (area, *rule)).collect();
        vectorcraft_three_d::inflate_meshes(&shapes, bounds, inflate_options(&p), view)
    });
    let extrude = extrude_options(&p);
    for (item, member, area, rule) in &parts {
        let item: &AppearanceItem = item;
        let mesh = match inflated.as_mut() {
            Some(built) => built.next().unwrap_or_else(|| Ok(Mesh::default())),
            None => vectorcraft_three_d::extrude_mesh(area, *rule, bounds, extrude, view),
        };
        match mesh {
            Ok(mesh) => {
                let (paint, opacity, _) = paint_of(item, *member);
                keep(&mut out, mesh, mesh_color(&paint, opacity))?;
            }
            Err(_) if matches!(item, AppearanceItem::Stroke(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// The solids `n`'s Extrude & Bevel, Inflate, Rotate or Revolve effect builds, as meshes with the
/// colors they are painted ([`SolidMesh`]), faces turned away from the viewer included; meshes
/// with no triangles are left out. On a group or layer, the effect's solids of its members
/// ([`members`]). The object's opacity multiplies every color's. An error when `n` carries no
/// such effect, its solids can't be built, none has a triangle, or together they have more than
/// [`MAX_MESH_TRIANGLES`].
pub fn three_d_meshes(n: &Node) -> Result<Vec<SolidMesh>, String> {
    meshes_within(n, &mut 0, MAX_MESH_TRIANGLES)
}

/// The meshes of every object in `nodes` ([`three_d_meshes`]), in order, as one 3D asset exports
/// them: the objects share one budget of [`MAX_MESH_TRIANGLES`].
pub fn three_d_meshes_of<'a>(nodes: impl IntoIterator<Item = &'a Node>) -> Result<Vec<SolidMesh>, String> {
    meshes_of_within(nodes, MAX_MESH_TRIANGLES)
}

/// [`three_d_meshes_of`] with a budget of `limit` triangles.
fn meshes_of_within<'a>(nodes: impl IntoIterator<Item = &'a Node>, limit: usize) -> Result<Vec<SolidMesh>, String> {
    let mut total = 0usize;
    let mut out = vec![];
    for n in nodes {
        out.extend(meshes_within(n, &mut total, limit)?);
    }
    Ok(out)
}

/// [`three_d_meshes`], with the triangles of every solid added to `total` as it is built and an
/// error as soon as `total` passes `limit`.
fn meshes_within(n: &Node, total: &mut usize, limit: usize) -> Result<Vec<SolidMesh>, String> {
    let meshes = if crate::has_revolve(n) {
        crate::revolve::revolve_meshes(n, total, limit)?
    } else if !has_solid(n) {
        return Err("the object has no 3D effect (Extrude & Bevel, Revolve, Inflate or Rotate)".into());
    } else if is_container(n) {
        let (index, fx) = solid(&n.appearance.effects).ok_or("a 3D effect on a group's fill or stroke draws nothing to export")?;
        let label = crate::effect_info(&fx.id).map_or("This effect", |e| e.label.trim_end_matches('…'));
        let mut shapes = members(n, label)?;
        reshape_members(n, n.appearance.effects.get(..index).unwrap_or_default(), &mut shapes);
        meshes_for(fx, &shapes, total, limit)?
    } else {
        let (path, rule) = profile(n).ok_or("3D export needs a path, compound path, type or group")?;
        let ctx = GeomContext::of(n);
        let paints = solid_paints(n);
        if let Some((index, fx)) = solid(&n.appearance.effects) {
            let path = geometry(&path, n.appearance.effects.get(..index).unwrap_or_default(), &ctx);
            meshes_for(fx, &[Profile { path, rule, items: paints, opacity: 1.0 }], total, limit)?
        } else {
            let path = geometry(&path, &n.appearance.effects, &ctx);
            let mut out = vec![];
            for item in &paints {
                if let Some((index, fx)) = solid(item.effects()) {
                    let p = geometry(&path, item.effects().get(..index).unwrap_or_default(), &ctx.item(item));
                    out.extend(meshes_for(fx, &[Profile { path: p, rule, items: vec![item.clone()], opacity: 1.0 }], total, limit)?);
                }
            }
            out
        }
    };
    let meshes: Vec<SolidMesh> = meshes
        .into_iter()
        .filter(|m| !m.mesh.is_empty())
        .map(|mut m| {
            m.color[3] *= n.opacity.clamp(0.0, 1.0);
            m
        })
        .collect();
    if meshes.is_empty() {
        return Err("its 3D effect builds no solid".into());
    }
    Ok(meshes)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_mesh;
