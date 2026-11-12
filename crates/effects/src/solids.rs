//! Extrude & Bevel, Inflate, Rotate, Extrude & Bevel (Classic) and Rotate (Classic) as live, shaded vector art,
//! shared by the canvas and every exporter. Like Revolve, the source path stays editable and the effect draws its 3D faces.
use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::{BlendMode, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Effect, Node, NodeId, NodeKind, OpacityMask, StrokeLayer};
use vectorcraft_geom::{FillRule, PathData, Rect};
use vectorcraft_three_d::{Bevel, BevelShape, Extrude, Facet, Inflate, Mesh, View};

#[cfg(test)]
use vectorcraft_doc::color::Color;

use crate::util::{flag, num, text};
use crate::{GeomContext, apply_geometry_with, is_geometry};

pub(crate) mod classic;
pub use classic::{CLASSIC_EXTRUDE, CLASSIC_ROTATE, ROTATE_SURFACES, SHADING_COLORS, classic_extrude_options, classic_shading};

mod source;
pub(crate) use source::{resolve, resolve_for};

pub(crate) mod shadow;

pub(crate) mod wireframe;
pub use wireframe::three_d_wireframe_in;

pub const EXTRUDE: &str = "threeD.extrude";
pub const INFLATE: &str = "threeD.inflate";
pub const ROTATE: &str = "threeD.rotate";
/// The 3D effects this module draws (Revolve has its own).
pub const SOLIDS: [&str; 5] = [EXTRUDE, INFLATE, ROTATE, CLASSIC_EXTRUDE, CLASSIC_ROTATE];

/// Is `id` one of the 3D and Materials effects (Revolve and Revolve (Classic) included)?
pub fn is_three_d(id: &str) -> bool {
    crate::revolve::is_revolve(id) || SOLIDS.contains(&id)
}

/// Is `id` one of the 3D (Classic) effects: Extrude & Bevel (Classic), Revolve (Classic) or Rotate
/// (Classic)?
pub fn is_classic_three_d(id: &str) -> bool {
    id == CLASSIC_EXTRUDE || id == crate::CLASSIC_REVOLVE || id == CLASSIC_ROTATE
}

/// Does 3D effect `id` turn the art as a flat plane: Rotate or Rotate (Classic)?
fn is_plane(id: &str) -> bool {
    id == ROTATE || id == CLASSIC_ROTATE
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
    if id == CLASSIC_ROTATE {
        classic::validate_rotate(p)?;
    }
    crate::lighting::validate(p)?;
    shadow::validate(p)?;
    if let Some(v) = p.get("bevel")
        && !v.as_str().is_some_and(|s| s == "none" || BevelShape::parse(s).is_some())
    {
        let names: Vec<_> = BevelShape::ALL.iter().map(|b| b.id()).collect();
        return Err(format!("bevel must be none or one of {}", names.join(", ")));
    }
    Ok(())
}

/// The name an error gives 3D effect `id`.
pub(crate) fn label_of(id: &str) -> &'static str {
    crate::effect_info(id).map_or("This effect", |e| e.label.trim_end_matches('…'))
}

/// Validate before a command changes the document; the evaluator bounds loaded files too.
pub fn validate_solid(n: &Node, id: &str, p: &Value) -> Result<(), String> {
    validate_solid_in(None, n, id, p)
}

/// [`validate_solid`] with the symbols of `doc`: a symbol instance, a blend and a clipping group
/// build from the art they show ([`resolve`]).
pub fn validate_solid_in(doc: Option<&Document>, n: &Node, id: &str, p: &Value) -> Result<(), String> {
    validate_solid_for(doc, n, id, p, None)
}

/// [`validate_solid_in`] for the effect on the object (`item` `None`) or on its appearance item
/// `item`, whose shadow is cast by that item's solid alone.
fn validate_solid_for(doc: Option<&Document>, n: &Node, id: &str, p: &Value, item: Option<usize>) -> Result<(), String> {
    validate_solid_params(id, p)?;
    let label = label_of(id);
    let n = &*source::resolve_for(doc, n, label, is_plane(id))?;
    if is_container(n) {
        let shapes = members(n, label)?;
        let b = union_bounds(&shapes).ok_or_else(|| format!("{label} requires a group holding a shape with at least two anchors"))?;
        shapes.iter().try_for_each(|s| facets(id, &s.path, s.rule, b, p).map(|_| ()))?;
        return cast_shadow(id, p, &shapes).map(|_| ());
    }
    let (path, rule) = object_profile(n, label)?;
    let b = path.bounds().ok_or_else(|| format!("{label} requires a shape with at least two anchors"))?;
    facets(id, &path, rule, b, p)?;
    // With its shadow on, the shadow of the object's solids (or of the item's) must fit too.
    let items = match item.and_then(|i| n.appearance.items.get(i)) {
        Some(item) => vec![item.clone()],
        None => solid_paints(n),
    };
    let shapes = [Profile { path, rule, items, opacity: 1.0, scopes: vec![] }];
    cast_shadow(id, p, &shapes).map(|_| ())
}

/// [`profile`], or the error a command reports for object `n`: type with no outlines to build from
/// (empty, or only spaces) is a shape without anchors, an image is traced first, and any other
/// object is the wrong kind.
pub(crate) fn object_profile(n: &Node, label: &str) -> Result<(PathData, FillRule), String> {
    profile(n).ok_or_else(|| match n.kind {
        NodeKind::Text(_) => format!("{label} requires a shape with at least two anchors"),
        NodeKind::Image(_) => source::image_refusal(label),
        _ => format!("{label} currently requires a path, compound path, type, group, blend or symbol instance. Expand other objects first."),
    })
}

/// A group in a 3D effect's group, or a member of it, whose opacity is below 100%, whose blending
/// mode is not Normal or that has an opacity mask. The members of one group share its `key`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Scope {
    pub(crate) key: usize,
    pub(crate) opacity: f32,
    pub(crate) blend: BlendMode,
    /// The object's opacity mask, unless it has none or it is disabled. The mask art is in
    /// document coordinates, as the solids' faces are.
    pub(crate) mask: Option<Arc<OpacityMask>>,
}

impl Scope {
    /// The scope of `n` with key `key`, or `None` when `n` is opaque, blends normally and has no
    /// opacity mask that applies.
    fn of(n: &Node, key: usize) -> Option<Self> {
        let mask = n.extras().mask.as_deref().filter(|m| !m.disabled);
        (n.opacity < 1.0 || n.blend != BlendMode::Normal || mask.is_some()).then(|| Scope {
            key,
            opacity: n.opacity,
            blend: n.blend,
            mask: mask.map(|m| Arc::new(m.clone())),
        })
    }

    /// A group of `children` with id `id` and this scope's opacity, blending mode and opacity mask.
    pub(crate) fn group(&self, id: NodeId, children: Vec<Arc<Node>>) -> Node {
        let mut group = Node::group(id, children);
        group.opacity = self.opacity;
        group.blend = self.blend;
        if let Some(mask) = &self.mask {
            group.extras_mut().mask = Some(Box::new((**mask).clone()));
        }
        group
    }
}

/// Does `scope` draw its members' faces other than plainly: does it have an opacity mask or a
/// blending mode other than Normal?
fn composites(scope: &Scope) -> bool {
    scope.mask.is_some() || scope.blend != BlendMode::Normal
}

/// `surfaces`, each with the scopes of the member it was built from, in groups with the opacity
/// masks and blending modes of those scopes, so each applies to the faces of the members it holds.
/// The members' opacity is already in their paint ([`Profile::opacity`]), so these groups are
/// opaque.
fn masked(id: NodeId, surfaces: Vec<(&[Scope], Arc<Node>)>) -> Vec<Arc<Node>> {
    let masks: Vec<(Vec<Scope>, Arc<Node>)> = surfaces
        .into_iter()
        .map(|(scopes, s)| {
            let masks = scopes.iter().filter(|s| composites(s)).map(|s| Scope { opacity: 1.0, ..s.clone() }).collect();
            (masks, s)
        })
        .collect();
    crate::revolve::scoped(id, masks.iter().map(|(scopes, s)| (scopes.as_slice(), s.clone())).collect(), 0)
}

/// Does any of `shapes` sit in a scope with an opacity mask or a blending mode other than Normal?
pub(crate) fn has_masks(shapes: &[Profile]) -> bool {
    shapes.iter().any(|s| s.scopes.iter().any(composites))
}

/// One shape a solid is built from: its outline and fill rule, the fills and strokes that paint
/// it, and the opacity of the objects it is in multiplied together (1 for the object with the
/// effect itself). `scopes` lists those objects with transparency or an opacity mask of their own
/// ([`Scope`]), from the outermost group down to the member itself (none for the object itself).
/// Group Revolve applies each once, to the surfaces of the members it holds together; the other 3D
/// effects apply each opacity mask that way.
pub(crate) struct Profile {
    pub(crate) path: PathData,
    pub(crate) rule: FillRule,
    pub(crate) items: Vec<AppearanceItem>,
    pub(crate) opacity: f32,
    pub(crate) scopes: Vec<Scope>,
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
/// compound path and type in it (after [`resolve`] gives it the art of its symbol instances, blends
/// and clipping groups), at any depth of its groups, each through its own geometry
/// effects, then those of each group it is in (on that group's members together), and painted
/// with its own fills and strokes. Like the effect on a group in the
/// reference app, they turn about one shared axis (the centre of their bounds together) rather
/// than each about its own. An error names what the group holds that can't become a solid.
pub(crate) fn members(n: &Node, label: &str) -> Result<Vec<Profile>, String> {
    struct Walk<'a> {
        label: &'a str,
        out: Vec<Profile>,
        keys: usize,
    }
    fn walk(n: &Node, opacity: f32, scopes: &[Scope], w: &mut Walk) -> Result<(), String> {
        let label = w.label;
        for c in n.children().into_iter().flatten() {
            if !c.visible || c.is_guide() {
                continue;
            }
            let opacity = opacity * c.opacity;
            let mut scopes = scopes.to_vec();
            if let Some(scope) = Scope::of(c, w.keys + 1) {
                w.keys += 1;
                scopes.push(scope);
            }
            if is_container(c) {
                let first = w.out.len();
                walk(c, opacity, &scopes, w)?;
                // The group's own geometry effects reshape its members together, as it is drawn.
                if let Some(shapes) = w.out.get_mut(first..) {
                    reshape_members(c, &c.appearance.effects, shapes);
                }
                continue;
            }
            let Some((path, rule)) = profile(c) else {
                let what = match &c.kind {
                    NodeKind::Image(_) => return Err(source::image_refusal(label)),
                    NodeKind::Path { clipping: true, .. } => "clipping paths",
                    _ => "other objects",
                };
                return Err(format!(
                    "{label} on a group needs paths, compound paths, type, groups, clipping groups, blends and symbol instances: release or expand its {what} first."
                ));
            };
            if w.out.len() >= MAX_MEMBERS {
                return Err(format!("{label} on a group takes at most {MAX_MEMBERS} objects"));
            }
            let path = geometry(&path, &c.appearance.effects, &GeomContext::of(c));
            if path.bounds().is_some() {
                w.out.push(Profile { path, rule, items: solid_paints(c), opacity, scopes });
            }
        }
        Ok(())
    }
    let mut w = Walk { label, out: vec![], keys: 0 };
    walk(n, 1.0, &[], &mut w)?;
    Ok(w.out)
}

/// Can 3D effect `id` (Revolve included, [`is_three_d`]) with parameters `p` draw `n`? Checked
/// before a command changes the document: Revolve by [`crate::validate_revolve`], the others by
/// [`validate_solid`].
pub fn validate_three_d(n: &Node, id: &str, p: &Value) -> Result<(), String> {
    validate_three_d_in(None, n, id, p)
}

/// [`validate_three_d`] with the symbols of `doc` ([`validate_solid_in`]).
pub fn validate_three_d_in(doc: Option<&Document>, n: &Node, id: &str, p: &Value) -> Result<(), String> {
    if crate::revolve::is_revolve(id) { crate::revolve::validate_in(doc, n, id, p) } else { validate_solid_in(doc, n, id, p) }
}

/// [`validate_three_d`] for an effect at `(item, index)`, index `index` of the effects of
/// appearance item `item` of `n` (the effects' length for a new effect), or on the object itself
/// when `at` is `None`. A Revolve on a fill or stroke is checked in that item's paint, after the
/// item's geometry effects before it, together with the object's other items that carry a Revolve.
/// A group's or layer's fills and strokes take no 3D effect: only the group's own effect draws.
pub fn validate_three_d_at(n: &Node, id: &str, p: &Value, at: Option<(usize, usize)>) -> Result<(), String> {
    validate_three_d_at_in(None, n, id, p, at)
}

/// [`validate_three_d_at`] with the symbols of `doc` ([`validate_solid_in`]). A symbol instance, a
/// blend and a clipping group take a 3D effect as groups do: on the object itself.
pub fn validate_three_d_at_in(doc: Option<&Document>, n: &Node, id: &str, p: &Value, at: Option<(usize, usize)>) -> Result<(), String> {
    match at {
        Some(_) if is_container(n) || source::resolves(n) => {
            Err("A 3D effect on a group or layer goes on the group itself, not on its fill or stroke".into())
        }
        Some((item, index)) if crate::revolve::is_revolve(id) => crate::revolve::validate_item_of(n, id, p, item, index),
        Some((item, _)) => validate_solid_for(doc, n, id, p, Some(item)),
        None => validate_three_d_in(doc, n, id, p),
    }
}

/// Are 3D effect `id`'s parameter values `p` valid? The checks of [`validate_three_d`] that do not
/// build the art, for an edit that hides the effect: hidden, it draws nothing, so its object does
/// not need to fit.
pub fn validate_three_d_params(id: &str, p: &Value) -> Result<(), String> {
    if crate::revolve::is_revolve(id) { crate::revolve::validate_params_of(id, p) } else { validate_solid_params(id, p) }
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

/// `item`'s paint, borrowed (for checks that must not copy it).
fn paint_ref(item: &AppearanceItem) -> &Paint {
    match item {
        AppearanceItem::Fill(f) => &f.paint,
        AppearanceItem::Stroke(s) => &s.paint,
    }
}

/// Paint copies `surface` makes for `facets`: two per facet (fill and seam stroke), six when colored
/// lights give each facet derived paints (`lighting::solid_surface`).
fn facet_copies(facets: &[Facet], paint: &Paint, colored: bool) -> usize {
    let derived = colored && paint.color().is_some() && facets.iter().any(|f| f.rgb_gradient.is_some());
    facets.len().saturating_mul(if derived { 6 } else { 2 })
}

/// The area `item` paints on `path`, and the rule that fills it: the path itself for a fill, the
/// stroke's outline (whose contours nest as its holes do) for a stroke.
fn region(path: &PathData, rule: FillRule, item: &AppearanceItem) -> (PathData, FillRule) {
    match item {
        AppearanceItem::Stroke(s) if s.width > 0.0 && s.width.is_finite() => (crate::stroke::outline_region(path, rule, s), FillRule::EvenOdd),
        _ => (path.clone(), rule),
    }
}

/// One part of [`solid_parts`]: a fill or stroke, its shape's opacity, the region it paints and its
/// fill rule, its shape's scopes and the index of its shape.
type Part<'a> = (&'a AppearanceItem, f32, PathData, FillRule, &'a [Scope], usize);

/// The parts Extrude & Bevel and Inflate build solids from: each shape's top fill and its strokes,
/// a group's members from the back up, each with its shape's opacity, the region it paints and its
/// shape's scopes. The solids share the bounds of all their parts (`fallback` when none has any):
/// one center to turn, twist and taper about, and Inflate's grid reaches over the strokes' outlines.
fn solid_parts(shapes: &[Profile], fallback: Rect) -> (Vec<Part<'_>>, Rect) {
    let parts: Vec<Part> = shapes
        .iter()
        .enumerate()
        .flat_map(|(member, s)| {
            let top_fill = s.items.iter().rposition(|i| matches!(i, AppearanceItem::Fill(_)));
            s.items.iter().enumerate().filter(move |(k, i)| Some(*k) == top_fill || matches!(i, AppearanceItem::Stroke(_))).map(move |(_, i)| {
                let (area, rule) = region(&s.path, s.rule, i);
                (i, s.opacity, area, rule, s.scopes.as_slice(), member)
            })
        })
        .collect();
    let bounds = parts.iter().filter_map(|(_, _, area, _, _, _)| area.bounds()).reduce(|a, c| a.union(c)).unwrap_or(fallback);
    (parts, bounds)
}

/// What a face of one of the solids [`art_for`] draws in one order was built from: the facet's
/// depth, whether it is a side face and whether it is turned away from the viewer, the bounds of
/// its polygon, the index of its shape, and whether its solid is that shape's fill or one of its
/// strokes that covers the fill's whole edge.
struct Placed {
    depth: f64,
    side: bool,
    away: bool,
    bounds: Rect,
    member: usize,
    paints: Paints,
}

/// The part of its shape a face's solid is built from ([`Placed`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Paints {
    Fill,
    /// A stroke whose solid holds the fill's whole edge and hides it: one with a width, without
    /// dashes, a width profile or a brush, painted with a solid color or a gradient without
    /// transparent stops, on a shape whose subpaths are all closed.
    Stroke,
    Other,
}

impl Paints {
    /// What `item` of a shape is, the shape's subpaths all closed when `closed`.
    fn of(item: &AppearanceItem, closed: bool) -> Self {
        match item {
            AppearanceItem::Fill(_) => Paints::Fill,
            AppearanceItem::Stroke(s)
                if closed
                    && s.width > 0.0
                    && s.width.is_finite()
                    && s.dash.is_none()
                    && s.profile.is_none()
                    && s.brush.is_none()
                    && match &s.paint {
                        Paint::Solid { .. } => true,
                        Paint::Gradient(_) => crate::lighting::opaque_paint(&s.paint).1.is_none(),
                        _ => false,
                    } =>
            {
                Paints::Stroke
            }
            AppearanceItem::Stroke(_) => Paints::Other,
        }
    }
}

impl Placed {
    /// The face of `f`, of shape `member`'s part `paints`.
    fn of(f: &Facet, member: usize, paints: Paints) -> Self {
        let mut points = f.contours.iter().flatten();
        let bounds = points.next().map_or(Rect::ZERO, |p| points.fold(Rect::from_points(*p, *p), |r, q| r.union_pt(*q)));
        Placed { depth: f.depth, side: f.side, away: f.away, bounds, member, paints }
    }
}

/// The most bounds comparisons [`draw_order`] makes; the fill walls it has not reached by then keep
/// their depths.
const MAX_OVERLAP_CHECKS: usize = 4_000_000;

/// The order faces `faces` are drawn in, back to front, as indices into `faces`: by depth (faces
/// at the same depth in the order given), except that with `sealed` (solids with caps) a side face
/// of a shape's fill turned toward the viewer is drawn before each face of the same shape's
/// covering strokes ([`Paints::Stroke`]), turned toward the viewer, whose bounds overlap its own.
/// Such a stroke's closed solid holds the fill's walls, so they never show and can be drawn under
/// any other face.
fn draw_order(faces: &[Placed], sealed: bool) -> Vec<usize> {
    // Each face's depth to sort by, and whether it was moved before a stroke face at that depth.
    let mut keys: Vec<(f64, bool)> = faces.iter().map(|f| (f.depth, false)).collect();
    if sealed {
        // The stroke faces turned toward the viewer of each shape, nearest last.
        let mut strokes: Vec<Vec<&Placed>> = vec![];
        for f in faces.iter().filter(|f| f.paints == Paints::Stroke && !f.away) {
            if strokes.len() <= f.member {
                strokes.resize_with(f.member + 1, Vec::new);
            }
            strokes[f.member].push(f);
        }
        for list in &mut strokes {
            list.sort_by(|a, b| a.depth.total_cmp(&b.depth));
        }
        let overlap = |a: &Rect, b: &Rect| a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1;
        let mut checks = 0usize;
        for (f, key) in faces.iter().zip(&mut keys) {
            if f.paints != Paints::Fill || !f.side || f.away {
                continue;
            }
            let Some(list) = strokes.get(f.member).filter(|l| !l.is_empty()) else { continue };
            for s in list.iter().take_while(|s| s.depth <= f.depth) {
                if checks >= MAX_OVERLAP_CHECKS {
                    break;
                }
                checks += 1;
                if overlap(&s.bounds, &f.bounds) {
                    *key = (s.depth, true);
                    break;
                }
            }
        }
    }
    let mut order: Vec<usize> = (0..faces.len()).collect();
    // A stable sort: faces at the same depth keep the order of the appearance, and a fill's side
    // face moved to a stroke face's depth goes before it.
    order.sort_by(|&a, &b| keys[a].0.total_cmp(&keys[b].0).then(keys[b].1.cmp(&keys[a].1)));
    order
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

/// A 3D effect's shadow and its surfaces ([`art_for`]).
type ShadowAndSurfaces = (Vec<Arc<Node>>, Vec<Arc<Node>>);

/// The 3D art of one effect `fx` on `path` (already through the effects before it).
/// Rotate turns every fill and stroke, each in its own paint. Extrude & Bevel and Inflate build a
/// solid from the path for the top visible fill and one from the outline of each visible stroke,
/// each in its own paint. When every one is opaque with normal blending and no colored light makes
/// a surface one composite, their faces are drawn in one back-to-front order (where depths tie, as
/// for the caps turned toward the viewer, in the order of the appearance, and with caps, a shape's
/// fill walls before the faces of its covering strokes that overlap them: [`draw_order`]), the
/// faces of a solid in a gradient with transparent stops in runs that carry its opacity mask
/// ([`in_runs`]);
/// otherwise each solid is a surface of its own, in that order. A member's opacity mask and
/// blending mode, and those of a group it is in, apply to the surfaces of the members it holds
/// ([`masked`]). The art comes in two parts: the effect's shadow ([`cast_shadow`], none with it
/// off), to be drawn behind every surface of the object, and the surfaces.
fn art_for(n: &Node, fx: &Effect, shapes: &[Profile]) -> Option<ShadowAndSurfaces> {
    let art = surfaces_for(n, fx, shapes)?;
    // A shadow past the limits draws the source path, as a solid past them does; commands report
    // the error ([`validate_solid_in`]).
    let shadow = match cast_shadow(&fx.id, &crate::merged_params(&fx.id, &fx.params), shapes).ok()? {
        Some((caster, settings, cast)) => shadow::scoped_art(n.id, &caster, settings, cast, has_masks(shapes)),
        None => vec![],
    };
    Some((shadow, art))
}

/// What the shadow of the solids [`surfaces_for`] builds for effect `id` with parameters `p` on
/// `shapes` is cast from, on one plane: the faces of every solid, the shadow's settings and the
/// faces of each solid of visible art with its member's scopes and ink. `None` with the shadow
/// off, for Extrude & Bevel (Classic) and Rotate (Classic) or with no shapes. A stroke's solid that is not built (past
/// the limits) casts nothing, as it draws nothing; any other solid that is not built is an error.
/// When a member's opacity mask or blending mode applies to its solids ([`masked`]), it applies to
/// the part of the shadow they cast too ([`shadow::scoped_art`]).
fn cast_shadow<'a>(id: &str, p: &Value, shapes: &'a [Profile]) -> Result<Option<Cast<'a>>, String> {
    if id == CLASSIC_EXTRUDE || id == CLASSIC_ROTATE {
        return Ok(None);
    }
    let (Some(settings), Some(b)) = (shadow::options(p), union_bounds(shapes)) else { return Ok(None) };
    let view = view_options(p);
    let mut caster = vectorcraft_three_d::ShadowCaster::new();
    // The faces each solid of visible art added, with its member's scopes and its ink (opacity
    // and blending mode), in order. A fully transparent fill or stroke (or member) casts nothing
    // and does not place the plane.
    let mut cast: Vec<shadow::Cast> = vec![];
    let ink = |item: &AppearanceItem, member: f32| {
        let (_, opacity, blend) = paint_of(item, member);
        shadow::Ink { opacity, blend }
    };
    if id == ROTATE {
        for s in shapes {
            for item in s.items.iter().filter(|i| ink(i, s.opacity).opacity > 0.0) {
                let (area, rule) = region(&s.path, s.rule, item);
                let faces = caster.plane(&area, rule, b, view)?;
                cast.push((s.scopes.as_slice(), ink(item, s.opacity), faces));
            }
        }
    } else {
        let (parts, bounds) = solid_parts(shapes, b);
        let inks: Vec<shadow::Ink> = parts.iter().map(|(item, member, ..)| ink(item, *member)).collect();
        // Inflate's eye is placed for every shape together, as the solids are.
        let added: Vec<Result<std::ops::Range<usize>, String>> = if id == INFLATE {
            let shapes: Vec<(&PathData, FillRule)> = parts.iter().map(|(_, _, area, rule, _, _)| (area, *rule)).collect();
            let casts: Vec<bool> = inks.iter().map(|i| i.opacity > 0.0).collect();
            caster.inflate(&shapes, &casts, bounds, inflate_options(p), view)
        } else {
            let mut added = vec![];
            for ((_, _, area, rule, _, _), ink) in parts.iter().zip(&inks) {
                added.push(if ink.opacity > 0.0 { caster.extrude(area, *rule, bounds, extrude_options(p), view) } else { Ok(0..0) });
            }
            added
        };
        for (((item, _, _, _, scopes, _), ink), faces) in parts.iter().zip(inks).zip(added) {
            if ink.opacity <= 0.0 {
                continue;
            }
            // A stroke whose own solid is past the limits is left out; past the shadow's limit for
            // all the solids together, the shadow is refused.
            let faces = match faces {
                Ok(faces) => faces,
                Err(e) if matches!(item, AppearanceItem::Stroke(_)) && e != vectorcraft_three_d::TOO_COMPLEX_SHADOW => continue,
                Err(e) => return Err(e),
            };
            cast.push((*scopes, ink, faces));
        }
    }
    Ok(Some((caster, settings, cast)))
}

/// The faces a shadow is cast from, its settings, and the faces of each solid of visible art with
/// its member's scopes and ink ([`cast_shadow`]).
type Cast<'a> = (vectorcraft_three_d::ShadowCaster, vectorcraft_three_d::Shadow, Vec<shadow::Cast<'a>>);

/// [`art_for`] without the shadow: the solids' surfaces.
fn surfaces_for(n: &Node, fx: &Effect, shapes: &[Profile]) -> Option<Vec<Arc<Node>>> {
    let b = union_bounds(shapes)?;
    let p = crate::merged_params(&fx.id, &fx.params);
    let classic = fx.id == CLASSIC_EXTRUDE || fx.id == CLASSIC_ROTATE;
    let colored = !classic && view_options(&p).has_colored_lights();
    let masks = has_masks(shapes);
    // The paint copies of every solid together: past the allowance the source path is drawn.
    let mut paint_bytes = 0usize;
    if is_plane(&fx.id) {
        // Every part turns about the centre of the object's own bounds (a group's members about
        // the centre of theirs), so they stay together.
        let mut out = vec![];
        for s in shapes {
            for item in &s.items {
                let (paint, opacity, blend) = paint_of(item, s.opacity);
                let (area, rule) = region(&s.path, s.rule, item);
                let f = facets(ROTATE, &area, rule, b, &p).ok()?;
                if !crate::revolve::paint_copies_add(&mut paint_bytes, facet_copies(&f, &paint, colored), &paint) {
                    return None;
                }
                let art = if classic { classic::surface(n, f, &paint, opacity, blend, &p) } else { surface(n, f, &paint, opacity, blend, colored) };
                out.push((s.scopes.as_slice(), Arc::new(art)));
            }
        }
        return Some(if masks { masked(n.id, out) } else { out.into_iter().map(|(_, s)| s).collect() });
    }
    let (parts, bounds) = solid_parts(shapes, b);
    let built = if fx.id == INFLATE {
        // One eye for every puff, as far away as the highest needs.
        let shapes: Vec<(&PathData, FillRule)> = parts.iter().map(|(_, _, area, rule, _, _)| (area, *rule)).collect();
        vectorcraft_three_d::inflate_together(&shapes, bounds, inflate_options(&p), view_options(&p))
    } else {
        parts.iter().map(|(_, _, area, rule, _, _)| facets(&fx.id, area, *rule, bounds, &p)).collect()
    };
    let mut solids = vec![];
    for ((item, opacity, _, _, scopes, member), f) in parts.iter().zip(built) {
        let item: &AppearanceItem = item;
        match f {
            Ok(f) if !crate::revolve::paint_copies_add(&mut paint_bytes, facet_copies(&f, paint_ref(item), colored), paint_ref(item)) => return None,
            Ok(f) => solids.push((item, *opacity, *scopes, f, *member)),
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
    let opaque = solids.iter().all(|(item, member, _, _, _)| {
        let (_, opacity, blend) = paint_of(item, *member);
        opacity >= 1.0 && blend == BlendMode::Normal
    });
    let surfaces: Vec<(Vec<Placed>, Node, &[Scope])> = solids
        .into_iter()
        .map(|(item, member, scopes, f, shape)| {
            let closed = shapes.get(shape).is_some_and(|s| s.path.subpaths.iter().all(|p| p.closed));
            let paints = Paints::of(item, closed);
            let depths: Vec<Placed> = f.iter().map(|f| Placed::of(f, shape, paints)).collect();
            let (paint, opacity, blend) = paint_of(item, member);
            let art = if classic { classic::surface(n, f, &paint, opacity, blend, &p) } else { surface(n, f, &paint, opacity, blend, colored) };
            (depths, art, scopes)
        })
        .collect();
    if masks {
        return Some(masked(n.id, surfaces.into_iter().map(|(_, s, scopes)| (scopes, Arc::new(s))).collect()));
    }
    // One order takes a plain group of faces from each solid, one face for each facet. With
    // colored lights a surface can instead be one clipped composite of all its facets. A composite
    // has no faces to order, so when any surface is one, every surface of the solid, masked ones
    // included, draws whole in member order.
    let plain = surfaces.iter().all(|(depths, s, _)| matches!(&s.kind, NodeKind::Group { children, clip: false } if children.len() == depths.len()));
    if !opaque || !plain {
        return Some(surfaces.into_iter().map(|(_, s, _)| Arc::new(s)).collect());
    }
    // A surface in a gradient with transparent stops carries an opacity mask: its faces go in runs
    // ([`in_runs`]), and each run carries the mask.
    let masks: Vec<Option<Box<OpacityMask>>> = surfaces.iter().map(|(_, s, _)| s.extras().mask.clone()).collect();
    let (mut placed, mut faces) = (vec![], vec![]);
    for (surface, (depths, s, _)) in surfaces.into_iter().enumerate() {
        if let NodeKind::Group { children, .. } = s.kind {
            placed.extend(depths);
            faces.extend(children.into_iter().map(|face| Some((surface, face))));
        }
    }
    let sealed = match fx.id.as_str() {
        INFLATE => inflate_options(&p).cap,
        CLASSIC_EXTRUDE => classic_extrude_options(&p).cap,
        _ => extrude_options(&p).cap,
    };
    let order = draw_order(&placed, sealed);
    let faces: Vec<(usize, Arc<Node>)> = order.into_iter().filter_map(|k| faces.get_mut(k).and_then(Option::take)).collect();
    let children = if masks.iter().all(Option::is_none) { faces.into_iter().map(|(_, face)| face).collect() } else { in_runs(n.id, faces, &masks) };
    Some(vec![Arc::new(Node::group(n.id, children))])
}

/// `faces` in their back-to-front order, each with the index of its surface, where some surfaces
/// carry an opacity mask (`masks`, by surface). A face of a surface without one goes in the order
/// on its own. The faces of a masked surface share a run, one group with the surface's mask. Such a
/// face joins its surface's last run unless a face drawn after that run overlaps it: only then does
/// the surface split, where one group can't be both behind and in front of another face.
fn in_runs(id: NodeId, faces: Vec<(usize, Arc<Node>)>, masks: &[Option<Box<OpacityMask>>]) -> Vec<Arc<Node>> {
    use crate::revolve::{Coverage, overlaps};
    struct Run {
        surface: Option<usize>,
        bounds: Rect,
        faces: Vec<Arc<Node>>,
        cover: Vec<Coverage>,
    }
    let mut runs: Vec<Run> = vec![];
    let mut last_run = vec![None; masks.len()];
    // Past this many tests a face starts a run of its own, which never draws out of order.
    let mut budget = crate::revolve::MAX_OVERLAP_TESTS;
    for (surface, face) in faces {
        let Some(cover) = Coverage::of(&face) else {
            // A face with no outline and no box overlaps nothing.
            runs.push(Run { surface: None, bounds: Rect::ZERO, faces: vec![face], cover: vec![] });
            continue;
        };
        if masks.get(surface).is_none_or(Option::is_none) {
            runs.push(Run { surface: None, bounds: cover.bounds, faces: vec![face], cover: vec![cover] });
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
        let joins = last_run.get(surface).copied().flatten().filter(|&i: &usize| runs.get(i + 1..).is_some_and(|later| later.iter().all(&mut clear)));
        match joins.and_then(|i| runs.get_mut(i)) {
            Some(run) => {
                run.bounds = run.bounds.union(cover.bounds);
                run.faces.push(face);
                run.cover.push(cover);
            }
            None => {
                if let Some(last) = last_run.get_mut(surface) {
                    *last = Some(runs.len());
                }
                runs.push(Run { surface: Some(surface), bounds: cover.bounds, faces: vec![face], cover: vec![cover] });
            }
        }
    }
    runs.into_iter()
        .filter_map(|mut r| {
            let Some(mask) = r.surface.and_then(|s| masks.get(s)).cloned().flatten() else { return r.faces.pop() };
            let mut group = Node::group(id, r.faces);
            group.extras_mut().mask = Some(mask);
            Some(Arc::new(group))
        })
        .collect()
}

fn geometry(path: &PathData, fx: &[Effect], ctx: &GeomContext) -> PathData {
    apply_geometry_with(fx, path, path.bounds().unwrap_or(Rect::ZERO), ctx)
}

/// `n` drawn with its Extrude & Bevel, Inflate or Rotate effect: a group of shaded faces keeping
/// the object's id and transparency, the source path untouched. `None` without such an effect.
/// On a group or layer the effect builds a solid from each path, compound path and type in it,
/// all turned about one shared axis ([`members`]).
pub fn solid_art(n: &Node) -> Option<Node> {
    solid_art_in(None, n)
}

/// [`solid_art`] with the symbols of `doc`: a symbol instance, a blend and a clipping group, alone
/// or in a group, build from the art they show ([`resolve`]). `None` too when that art can't be
/// resolved.
pub fn solid_art_in(doc: Option<&Document>, n: &Node) -> Option<Node> {
    if !has_solid(n) {
        return None;
    }
    let n = &*resolve(doc, n, "").ok()?;
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
        let (shadow, surfaces) = art_for(n, fx, &[Profile { path, rule, items: paints, opacity: 1.0, scopes: vec![] }])?;
        children.extend(shadow.into_iter().chain(surfaces));
        // Later geometry effects act on the projected art as a whole.
        remaining = past_effect(&n.appearance.effects, index);
    } else {
        let path = geometry(&path, &n.appearance.effects, &ctx);
        remaining = crate::without_geometry(&n.appearance.effects);
        // Every item's shadow goes behind every item's art.
        let mut shadows = vec![];
        for item in &paints {
            if let Some((index, fx)) = solid(item.effects()) {
                let p = geometry(&path, item.effects().get(..index)?, &ctx.item(item));
                let (shadow, surfaces) = art_for(n, fx, &[Profile { path: p, rule, items: vec![item.clone()], opacity: 1.0, scopes: vec![] }])?;
                let later: Vec<Effect> = item.effects().iter().skip(index + 1).filter(|e| !is_three_d(&e.id)).cloned().collect();
                let with_later = |mut art: Arc<Node>| {
                    Arc::make_mut(&mut art).appearance.effects = later.clone();
                    art
                };
                shadows.extend(shadow.into_iter().map(with_later));
                children.extend(surfaces.into_iter().map(with_later));
            } else {
                children.push(Arc::new(Node::path(n.id, path.clone(), Appearance { items: vec![item.clone()], ..Default::default() })));
            }
        }
        children.splice(0..0, shadows);
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
    let (shadow, surfaces) = art_for(n, fx, &shapes)?;
    let children = shadow.into_iter().chain(surfaces).collect();
    let remaining = past_effect(&n.appearance.effects, index);
    let mut art = n.clone();
    art.kind = NodeKind::Group { children, clip: false };
    art.appearance = Appearance { effects: remaining, ..Default::default() }.into();
    Some(art)
}

/// `effects` without the 3D effect at `index` and the geometry effects before it, which shape the
/// art it projects. Each raster effect before it follows the Transform effects among those
/// ([`crate::without_geometry`]), unless a Transform effect after it makes copies or moves at
/// random ([`crate::raster::holds_back`]); the effects after it stay as they are.
pub(crate) fn past_effect(effects: &[Effect], index: usize) -> Vec<Effect> {
    let (before, after) = effects.split_at(index.min(effects.len()));
    let after = after.get(1..).unwrap_or_default();
    let kept: Vec<Effect> = if after.iter().any(crate::raster::holds_back) {
        before.iter().filter(|e| !is_geometry(&e.id)).cloned().collect()
    } else {
        crate::without_geometry(before)
    };
    kept.into_iter().chain(after.iter().cloned()).collect()
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
    if is_plane(&fx.id) {
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
        let shapes: Vec<(&PathData, FillRule)> = parts.iter().map(|(_, _, area, rule, _, _)| (area, *rule)).collect();
        vectorcraft_three_d::inflate_meshes(&shapes, bounds, inflate_options(&p), view)
    });
    let extrude = extrude_options(&p);
    for (item, member, area, rule, _, _) in &parts {
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
    three_d_meshes_in(None, n)
}

/// [`three_d_meshes`] with the symbols of `doc`: a symbol instance, a blend and a clipping group
/// build from the art they show ([`resolve`]).
pub fn three_d_meshes_in(doc: Option<&Document>, n: &Node) -> Result<Vec<SolidMesh>, String> {
    meshes_within(doc, n, &mut 0, MAX_MESH_TRIANGLES)
}

/// The meshes of every object in `nodes` ([`three_d_meshes`]), in order, as one 3D asset exports
/// them: the objects share one budget of [`MAX_MESH_TRIANGLES`].
pub fn three_d_meshes_of<'a>(nodes: impl IntoIterator<Item = &'a Node>) -> Result<Vec<SolidMesh>, String> {
    three_d_meshes_of_in(None, nodes)
}

/// [`three_d_meshes_of`] with the symbols of `doc` ([`three_d_meshes_in`]).
pub fn three_d_meshes_of_in<'a>(doc: Option<&Document>, nodes: impl IntoIterator<Item = &'a Node>) -> Result<Vec<SolidMesh>, String> {
    meshes_of_within(doc, nodes, MAX_MESH_TRIANGLES)
}

/// [`three_d_meshes_of_in`] with a budget of `limit` triangles.
fn meshes_of_within<'a>(doc: Option<&Document>, nodes: impl IntoIterator<Item = &'a Node>, limit: usize) -> Result<Vec<SolidMesh>, String> {
    let mut total = 0usize;
    let mut out = vec![];
    for n in nodes {
        out.extend(meshes_within(doc, n, &mut total, limit)?);
    }
    Ok(out)
}

/// [`three_d_meshes_in`], with the triangles of every solid added to `total` as it is built and an
/// error as soon as `total` passes `limit`.
fn meshes_within(doc: Option<&Document>, n: &Node, total: &mut usize, limit: usize) -> Result<Vec<SolidMesh>, String> {
    if !crate::has_revolve(n) && !has_solid(n) {
        return Err("the object has no 3D effect (Extrude & Bevel, Revolve, Inflate or Rotate)".into());
    }
    let label = n.appearance.effects.iter().find(|e| e.visible && is_three_d(&e.id)).map_or("This effect", |e| label_of(&e.id));
    let n = &*resolve(doc, n, label)?;
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
        let (path, rule) = object_profile(n, label)?;
        let ctx = GeomContext::of(n);
        let paints = solid_paints(n);
        if let Some((index, fx)) = solid(&n.appearance.effects) {
            let path = geometry(&path, n.appearance.effects.get(..index).unwrap_or_default(), &ctx);
            meshes_for(fx, &[Profile { path, rule, items: paints, opacity: 1.0, scopes: vec![] }], total, limit)?
        } else {
            let path = geometry(&path, &n.appearance.effects, &ctx);
            let mut out = vec![];
            for item in &paints {
                if let Some((index, fx)) = solid(item.effects()) {
                    let p = geometry(&path, item.effects().get(..index).unwrap_or_default(), &ctx.item(item));
                    out.extend(meshes_for(fx, &[Profile { path: p, rule, items: vec![item.clone()], opacity: 1.0, scopes: vec![] }], total, limit)?);
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

#[cfg(test)]
mod tests_sources;

#[cfg(test)]
mod tests_shadow;
