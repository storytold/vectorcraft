//! Extrude & Bevel, Inflate, Rotate, Extrude & Bevel (Classic) and Rotate (Classic) as live, shaded vector art,
//! shared by the canvas and every exporter. Like Revolve, the source path stays editable and the effect draws its 3D faces.
use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::{BlendMode, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Effect, Node, NodeId, NodeKind, OpacityMask, StrokeLayer};
use vectorcraft_geom::{BezPath, FillRule, PathData, Point, Rect, Vec2};
use vectorcraft_pathops::BoolOp;
use vectorcraft_three_d::{Bevel, BevelProfile, BevelShape, DepthPlane, Extrude, Facet, Inflate, MAX_OBJECT_FACES, Mesh, PlanarFace, View};

#[cfg(test)]
use vectorcraft_doc::color::Color;

use crate::util::{flag, num, text};
use crate::{GeomContext, apply_geometry_with, is_geometry};

pub(crate) mod classic;
pub use classic::{CLASSIC_EXTRUDE, CLASSIC_ROTATE, ROTATE_SURFACES, SHADING_COLORS, classic_extrude_options, classic_shading};

mod source;
pub(crate) use source::{resolve, resolve_for};

mod graphics;
pub use graphics::{
    GRAPHICS, GraphicFrame, MAX_GRAPHICS, MappedGraphic, graphic_symbols, mapped_graphics, missing_symbol, names_graphic_symbol,
    rename_graphic_symbol, rename_graphic_symbol_in, takes_graphics, with_graphic,
};
pub use graphics::{
    MAX_OFFSET as GRAPHIC_MAX_OFFSET, SCALE_RANGE as GRAPHIC_SCALE_RANGE, appearance_graphic_symbols, rename_appearance_graphic_symbol,
};

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
/// settings only `to` takes start at their defaults. Settings `to` does not take are dropped; a
/// custom bevel's name and points ([`BEVEL_NAME`], [`BEVEL_PROFILE`]) go with `bevel` when `to`
/// takes one.
pub fn three_d_params_as(from: &str, params: &Value, to: &str) -> Value {
    let current = crate::merged_params(from, params);
    let mut out = crate::default_params(to).and_then(|v| v.as_object().cloned()).unwrap_or_default();
    for (key, value) in out.iter_mut() {
        if let Some(v) = current.get(key) {
            value.clone_from(v);
        }
    }
    if out.contains_key("bevel") {
        for key in [BEVEL_NAME, BEVEL_PROFILE] {
            if let Some(v) = current.get(key).filter(|v| !v.is_null()) {
                out.insert(key.into(), v.clone());
            }
        }
    }
    if to != crate::REVOLVE {
        for key in ["rotationX", "rotationY", "rotationZ"] {
            if let Some(a) = out.get(key).and_then(Value::as_f64).filter(|a| !(-180.0..=180.0).contains(a)) {
                out.insert(key.into(), serde_json::json!((a + 180.0).rem_euclid(360.0) - 180.0));
            }
        }
    }
    // A plane has only a front, so graphics on the back cap are dropped.
    if to == ROTATE
        && let Some(Value::Array(list)) = out.get_mut(GRAPHICS)
    {
        list.retain(|g| g.get("face").and_then(Value::as_str) != Some(PlanarFace::Back.id()));
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

/// The custom bevel's name parameter: the name the bevels file gave it, kept for the menus.
pub const BEVEL_NAME: &str = "bevelName";
/// The custom bevel's points parameter: `[[across, down], …]`, shares of the bevel's width and
/// height from the cap's edge `[0, 0]` to the side wall `[1, 1]` (a copy of the bevel's path, so
/// the document draws it without the bevels file).
pub const BEVEL_PROFILE: &str = "bevelProfile";

/// The points of `v`, a [`BEVEL_PROFILE`] value; `None` unless it is an array of `[x, y]` number
/// pairs that make a [`BevelProfile`].
fn bevel_profile(v: &Value) -> Option<BevelProfile> {
    let list = v.as_array().filter(|a| a.len() <= BevelProfile::MAX_POINTS)?;
    let pts: Option<Vec<(f64, f64)>> = list
        .iter()
        .map(|pt| match pt.as_array().map(Vec::as_slice) {
            Some([x, y]) => Some((x.as_f64()?, y.as_f64()?)),
            _ => None,
        })
        .collect();
    BevelProfile::new(&pts?)
}

/// The bevel shape of parameters `p`: `bevel` `"none"` (none), a built-in [`BevelShape`] id, or
/// `"custom"` with its points in [`BEVEL_PROFILE`] (none when they do not make a profile).
pub(crate) fn bevel_shape(p: &Value) -> Option<BevelShape> {
    match text(p, "bevel", "none") {
        "custom" => p.get(BEVEL_PROFILE).and_then(bevel_profile).map(BevelShape::Custom),
        id => BevelShape::parse(id),
    }
}

/// Check the custom bevel parameters of `p`: `bevel` `"custom"` needs [`BEVEL_PROFILE`], whose
/// points, when given, must make a profile, and [`BEVEL_NAME`] is a string.
fn validate_bevel(p: &Value) -> Result<(), String> {
    let most = BevelProfile::MAX_POINTS;
    if let Some(v) = p.get(BEVEL_PROFILE)
        && !v.is_null()
        && bevel_profile(v).is_none()
    {
        return Err(format!(
            "{BEVEL_PROFILE} must be 2 to {most} [x, y] points, each share between 0 and 1, from [0, 0] (the cap's edge) to [1, 1] (the side wall)"
        ));
    }
    if text(p, "bevel", "none") == "custom" && p.get(BEVEL_PROFILE).is_none_or(Value::is_null) {
        return Err(format!("bevel custom needs {BEVEL_PROFILE}, the bevel's points"));
    }
    if p.get(BEVEL_NAME).is_some_and(|v| !v.is_string() && !v.is_null()) {
        return Err(format!("{BEVEL_NAME} must be a string"));
    }
    Ok(())
}

/// Extrude & Bevel's parameters. `bevel` is `"none"`, a [`BevelShape`] id, or `"custom"` with
/// its points in [`BEVEL_PROFILE`].
pub fn extrude_options(p: &Value) -> Extrude {
    Extrude {
        depth: num(p, "depth", 50.0),
        twist: num(p, "twist", 0.0),
        taper: num(p, "taper", 100.0) / 100.0,
        cap: flag(p, "cap", true),
        bevel: bevel_shape(p).map(|shape| Bevel {
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
    graphics::validate(id, p)?;
    if let Some(v) = p.get("bevel")
        && !v.as_str().is_some_and(|s| s == "none" || s == "custom" || BevelShape::parse(s).is_some())
    {
        let names: Vec<_> = BevelShape::ALL.iter().map(|b| b.id()).collect();
        return Err(format!("bevel must be none or one of {}, or custom with {BEVEL_PROFILE}", names.join(", ")));
    }
    validate_bevel(p)
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
    validate_solid_for(doc, n, id, p, (None, None))
}

/// [`validate_solid_in`] for the effect at `(item, index)`: index `index` of the effects of the
/// object (`item` `None`) or of its appearance item `item`, whose shadow is cast by that item's
/// solid alone. The index is the effects' length for a new effect. The solids are checked as
/// [`solid_art_in`] draws them: from the shapes after the geometry effects before the 3D effect.
/// On the object (or group) with `index` `None`, that effect is its first visible Extrude & Bevel,
/// Inflate or Rotate, as drawn, or a new one after its other effects when it has none.
fn validate_solid_for(doc: Option<&Document>, n: &Node, id: &str, p: &Value, at: (Option<usize>, Option<usize>)) -> Result<(), String> {
    validate_solid_params(id, p)?;
    if let Some(doc) = doc {
        graphics::validate_symbols(doc, p)?;
    }
    let label = label_of(id);
    let n = &*source::resolve_for(doc, n, label, is_plane(id))?;
    let effects = &n.appearance.effects;
    let index = match at {
        (None, Some(index)) => index,
        _ => solid(effects).map_or(effects.len(), |(index, _)| index),
    };
    let before = effects.get(..index).unwrap_or(effects);
    if is_container(n) {
        let mut shapes = members(n, label)?;
        reshape_members(n, before, &mut shapes);
        let b = union_bounds(&shapes).ok_or_else(|| format!("{label} requires a group holding a shape with at least two anchors"))?;
        shapes.iter().try_for_each(|s| facets(id, &s.path, s.rule, b, p).map(|_| ()))?;
        check_solids(id, p, &shapes)?;
        return cast_shadow(id, p, &shapes).map(|_| ());
    }
    let (path, rule) = object_profile(n, label)?;
    let anchors = || format!("{label} requires a shape with at least two anchors");
    path.bounds().ok_or_else(anchors)?;
    let ctx = GeomContext::of(n);
    // With its shadow on, the shadow of the object's solids (or of the item's) must fit too.
    let (path, items) = match at.0.and_then(|i| Some((n.appearance.items.get(i)?, at.1.unwrap_or(usize::MAX)))) {
        Some((item, index)) => {
            let path = geometry(&path, effects, &ctx);
            (geometry(&path, item.effects().get(..index).unwrap_or(item.effects()), &ctx.item(item)), vec![item.clone()])
        }
        None => (geometry(&path, before, &ctx), solid_paints(n)),
    };
    let b = path.bounds().ok_or_else(anchors)?;
    facets(id, &path, rule, b, p)?;
    let shapes = [Profile { path, rule, items, opacity: 1.0, scopes: vec![] }];
    check_solids(id, p, &shapes)?;
    cast_shadow(id, p, &shapes).map(|_| ())
}

/// Do the solids 3D effect `id` with parameters `p` builds from the fills and strokes of `shapes`
/// fit, each on its own and all of them together ([`built_solids`])? Rotate's planes are not
/// counted.
fn check_solids(id: &str, p: &Value, shapes: &[Profile]) -> Result<(), String> {
    let Some(b) = union_bounds(shapes).filter(|_| !is_plane(id)) else { return Ok(()) };
    let (parts, bounds) = solid_parts(shapes, b);
    built_solids(id, p, &parts, bounds, MAX_OBJECT_FACES).try_for_each(|r| r.map(|_| ()))
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
    graphics::validate(id, p)?;
    if crate::revolve::is_revolve(id) { crate::revolve::validate_in(doc, n, id, p) } else { validate_solid_in(doc, n, id, p) }
}

/// [`validate_three_d`] for an effect at `(item, index)`: index `index` of the effects of
/// appearance item `item` of `n`, or of the object itself when `item` is `None` (the effects'
/// length for a new effect). An Extrude & Bevel, Inflate or Rotate is checked on the shapes after
/// the geometry effects before that index. A Revolve on a fill or stroke is checked in that item's
/// paint, after the item's geometry effects before it, together with the object's other items that
/// carry a Revolve. A group's or layer's fills and strokes take no 3D effect: only the group's own
/// effect draws.
pub fn validate_three_d_at(n: &Node, id: &str, p: &Value, at: (Option<usize>, usize)) -> Result<(), String> {
    validate_three_d_at_in(None, n, id, p, at)
}

/// [`validate_three_d_at`] with the symbols of `doc` ([`validate_solid_in`]). A symbol instance, a
/// blend and a clipping group take a 3D effect as groups do: on the object itself.
pub fn validate_three_d_at_in(doc: Option<&Document>, n: &Node, id: &str, p: &Value, at: (Option<usize>, usize)) -> Result<(), String> {
    graphics::validate(id, p)?;
    match at {
        (Some(_), _) if is_container(n) || source::resolves(n) => {
            Err("A 3D effect on a group or layer goes on the group itself, not on its fill or stroke".into())
        }
        (Some(item), index) if crate::revolve::is_revolve(id) => crate::revolve::validate_item_of(n, id, p, item, index),
        (None, _) if crate::revolve::is_revolve(id) => crate::revolve::validate_in(doc, n, id, p),
        (item, index) => validate_solid_for(doc, n, id, p, (item, Some(index))),
    }
}

/// Are 3D effect `id`'s parameter values `p` valid? The checks of [`validate_three_d`] that do not
/// build the art, for an edit that hides the effect: hidden, it draws nothing, so its object does
/// not need to fit.
pub fn validate_three_d_params(id: &str, p: &Value) -> Result<(), String> {
    graphics::validate(id, p)?;
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

#[cfg(test)]
thread_local! {
    /// How many solids [`built_solids`] has built on this thread, for the tests.
    static SOLIDS_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The solids Extrude & Bevel (Classic included) or Inflate `id` with parameters `p` builds from
/// `parts` ([`solid_parts`]) about `bounds`, one result for each part in order, each built only as
/// the iterator reaches it. A stroke's solid past its own limits is `None` (left out of the art, its
/// faces not counted); a fill's is an error. The faces of the solids kept count together: once they
/// pass `limit` ([`MAX_OBJECT_FACES`] for the art) the result is an error, and
/// no later part is built. An error ends the iterator.
fn built_solids<'a>(
    id: &'a str,
    p: &'a Value,
    parts: &'a [Part<'a>],
    bounds: Rect,
    limit: usize,
) -> impl Iterator<Item = Result<Option<Vec<Facet>>, String>> + 'a {
    // Inflate's eye is placed for every puff together, as far away as the highest needs.
    let mut inflated = (id == INFLATE).then(|| {
        let shapes: Vec<(&PathData, FillRule)> = parts.iter().map(|(_, _, area, rule, _, _)| (area, *rule)).collect();
        vectorcraft_three_d::inflate_each(&shapes, bounds, inflate_options(p), view_options(p))
    });
    let mut kept = 0usize;
    let mut ended = false;
    parts.iter().map_while(move |(item, _, area, rule, _, _)| {
        if ended {
            return None;
        }
        #[cfg(test)]
        SOLIDS_BUILT.with(|n| n.set(n.get() + 1));
        let built = match inflated.as_mut() {
            Some(each) => each.next().unwrap_or_else(|| Ok(vec![])),
            None => facets(id, area, *rule, bounds, p),
        };
        let result = match built {
            Ok(f) => {
                kept = kept.saturating_add(f.len());
                if kept > limit { Err(vectorcraft_three_d::TOO_COMPLEX_SOLID.to_string()) } else { Ok(Some(f)) }
            }
            Err(_) if matches!(item, AppearanceItem::Stroke(_)) => Ok(None),
            Err(e) => Err(e),
        };
        ended = result.is_err();
        Some(result)
    })
}

/// What a face of one of the solids [`art_for`] draws in one order was built from: the facet's
/// depth, whether it is a side face and whether it is turned away from the viewer, the bounds of
/// its polygon, the index of its shape, and whether its solid is that shape's fill or one of its
/// strokes that covers the fill's whole edge. A face that [`convex_face`] takes also keeps its
/// polygon and plane, for [`local_order`]; every other face has an empty `outline` and no `plane`.
struct Placed {
    depth: f64,
    side: bool,
    away: bool,
    bounds: Rect,
    member: usize,
    paints: Paints,
    outline: Vec<Point>,
    plane: Option<DepthPlane>,
}

/// The part of its shape a face's solid is built from ([`Placed`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Paints {
    Fill,
    /// A stroke whose solid holds the fill's whole edge and hides it: one with a width, without
    /// dashes, a width profile or a brush, painted with a solid color or a gradient without
    /// transparent stops, on a shape whose subpaths are all closed, extruded (an inflated fill can
    /// rise in front of its stroke).
    Stroke,
    Other,
}

/// Can a stroke's solid on a shape with outline `path` hold the fill's edge ([`Paints::Stroke`])
/// under the effect `effect`? Only when the subpaths are all closed and the effect extrudes: an
/// inflated fill can rise in front of its stroke.
fn holds_edge(effect: &str, path: &PathData) -> bool {
    effect != INFLATE && path.subpaths.iter().all(|p| p.closed)
}

impl Paints {
    /// What `item` of a shape is, its stroke's solid able to hold the fill's edge when `closed`
    /// ([`holds_edge`]).
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
        let local = convex_face(f).is_some();
        let outline = f.contours.first().filter(|_| local).cloned().unwrap_or_default();
        let plane = f.plane.filter(|_| local);
        Placed { depth: f.depth, side: f.side, away: f.away, bounds, member, paints, outline, plane }
    }
}

/// The most pairs of faces [`local_order`] compares, and the most [`cut_stroke_faces`] compares;
/// the pairs they have not reached by then keep the order of their depths.
const MAX_LOCAL_CHECKS: usize = 4_000_000;

/// The polygon of `f` and the sign of its turns, when `f` is a face turned toward the viewer other
/// than a planar end, with a plane and one convex contour: a face [`local_order`] and
/// [`cut_stroke_faces`] work on.
fn convex_face(f: &Facet) -> Option<(&[Point], f64)> {
    if f.away || f.end.is_some() || f.plane.is_none() || f.contours.len() != 1 {
        return None;
    }
    let c = f.contours.first()?;
    Some((c, convex(c)?))
}

/// The bounds of polygon `p` (`Rect::ZERO` when it has no points).
fn polygon_bounds(p: &[Point]) -> Rect {
    let mut points = p.iter();
    points.next().map_or(Rect::ZERO, |q| points.fold(Rect::from_points(*q, *q), |r, q| r.union_pt(*q)))
}

/// Do rectangles `a` and `b` overlap (more than along an edge)?
fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

/// How many faces [`cut_stroke_faces`] may add to solids painted by `items` that hold `faces` faces
/// and whose paint copies take `paint_bytes` so far: as many as keep the faces within
/// [`MAX_OBJECT_FACES`] and the copies within the allowance, each added face counted as six copies
/// of the costliest stroke's paint.
fn cut_room<'a>(items: impl Iterator<Item = &'a AppearanceItem>, faces: usize, paint_bytes: usize) -> usize {
    let per_face = items
        .filter(|item| matches!(item, AppearanceItem::Stroke(_)))
        .map(|item| {
            let mut copies = 0usize;
            crate::revolve::paint_copies_add(&mut copies, 6, paint_ref(item));
            copies
        })
        .max()
        .unwrap_or(0)
        .max(1);
    let affordable = crate::revolve::MAX_PAINT_COPIES.saturating_sub(paint_bytes) / per_face;
    MAX_OBJECT_FACES.saturating_sub(faces).min(affordable)
}

/// In `solids`, each solid's part ([`Paints`]), its shape's index and its faces, cut each face of a
/// stroke that does not cover the fill's edge ([`Paints::Other`]) that passes through the plane of a
/// face of the same shape's fill, or of a face of another of the shape's strokes, where the two
/// overlap on the drawing ([`cut_through`]). Of two such strokes, the later one's faces are cut
/// against the earlier one's; a covering stroke's faces ([`Paints::Stroke`]) are never cut, and an
/// other stroke's faces are cut against them. At most `room` pieces are added, the object's solids
/// together.
fn cut_stroke_faces(solids: &mut [(Paints, usize, &mut Vec<Facet>)], mut room: usize) {
    let mut checks = 0usize;
    for k in 0..solids.len() {
        let Some((Paints::Other, shape)) = solids.get(k).map(|(p, s, _)| (*p, *s)) else { continue };
        let walls: Vec<usize> = (0..solids.len())
            .filter(|&j| {
                solids.get(j).is_some_and(|(p, s, _)| {
                    *s == shape
                        && match p {
                            Paints::Fill | Paints::Stroke => true,
                            Paints::Other => j < k,
                        }
                })
            })
            .collect();
        for j in walls {
            let (wall, stroke) = if j < k {
                let (before, from) = solids.split_at_mut(k);
                (&**before[j].2, &mut *from[0].2)
            } else {
                let (before, from) = solids.split_at_mut(j);
                (&**from[0].2, &mut *before[k].2)
            };
            cut_through(wall, stroke, &mut room, &mut checks);
        }
    }
}

/// Cut each face of `stroke` that passes through the plane of a face of `solid` (the fill's or
/// another stroke's) where their polygons overlap on the drawing, along the line where their depths
/// meet, so that each piece is wholly in front of or behind that face there ([`local_order`] can
/// then order them). A dash's end, which crosses the edge of the fill it is drawn on, is cut into its
/// part outside the fill, in front of the fill's wall, and its part inside, behind it. A piece keeps
/// the part of the face's curved outline on its side of the cut. The first piece of a face takes its
/// place, the others go at the end. Each cut adds a face out of `room`, and each pair compared counts
/// in `checks` ([`MAX_LOCAL_CHECKS`]); cutting stops when either runs out.
fn cut_through(solid: &[Facet], stroke: &mut Vec<Facet>, room: &mut usize, checks: &mut usize) {
    // The faces from left to right: a face is compared with those that start left of its end.
    let mut walls: Vec<(&Facet, &[Point], f64, Rect)> =
        solid.iter().filter_map(|f| convex_face(f).map(|(c, turn)| (f, c, turn, polygon_bounds(c)))).collect();
    if walls.is_empty() {
        return;
    }
    walls.sort_by(|a, b| a.3.x0.total_cmp(&b.3.x0));
    let mut k = 0;
    while k < stroke.len() {
        let mut extent = stroke.get(k).and_then(convex_face).map(|(c, _)| polygon_bounds(c));
        for &(wall, outline, turn, bounds) in &walls {
            let Some(at) = extent.filter(|e| bounds.x0 < e.x1) else { break };
            if *checks >= MAX_LOCAL_CHECKS || *room == 0 {
                return;
            }
            *checks += 1;
            if !rects_overlap(at, bounds) {
                continue;
            }
            let Some(s) = stroke.get(k) else { break };
            let Some((c, _)) = convex_face(s) else { break };
            if let Some((near, far)) = cut(s, c, wall, outline, turn) {
                *room -= 1;
                extent = convex_face(&near).map(|(c, _)| polygon_bounds(c));
                stroke[k] = near;
                stroke.push(far);
            }
        }
        k += 1;
    }
}

/// Face `s` (polygon `c`) cut where its depth meets that of face `wall` (convex polygon `outline`,
/// turning with sign `turn`): its piece nearer the viewer than the wall's plane and its piece
/// farther. `None` when the two do not overlap on the drawing or `s` is on one side of the wall's
/// plane wherever they do.
fn cut(s: &Facet, c: &[Point], wall: &Facet, outline: &[Point], turn: f64) -> Option<(Facet, Facet)> {
    let ahead = |p: Point| Some(s.depth_at(p)? - wall.depth_at(p)?);
    let overlap = clip_convex(c, outline, turn);
    centroid(&overlap)?;
    let tolerance = 1e-6;
    let (mut nearer, mut farther) = (false, false);
    for p in &overlap {
        let d = ahead(*p)?;
        nearer |= d > tolerance;
        farther |= d < -tolerance;
    }
    if !(nearer && farther) {
        return None;
    }
    let (a, b) = (s.plane?, wall.plane?);
    let values: Vec<f64> = match (a.eye, b.eye) {
        (Some(e), Some(f)) if e == f && a.center == b.center => {
            // With perspective the depths themselves are not linear on the drawing, but where they
            // meet is a line: (dA - aA e)(nzB - aB) = (dB - aB e)(nzA - aA), with aX = (nX · p) / e,
            // whose difference is linear. Its sign is that of the depths' difference over the
            // product of the two denominators, taken at the middle of the face.
            let at = |p: Point| {
                let (x, y) = (p.x - a.center.x, p.y - a.center.y);
                let (aa, ab) = ((a.normal[0] * x + a.normal[1] * y) / e, (b.normal[0] * x + b.normal[1] * y) / e);
                let (da, db) = (a.normal[2] - aa, b.normal[2] - ab);
                ((a.offset - aa * e) * db - (b.offset - ab * e) * da, da * db)
            };
            let sign = at(centroid(c)?).1.signum();
            c.iter().map(|p| at(*p).0 * sign).collect()
        }
        _ => c.iter().map(|p| ahead(*p)).collect::<Option<_>>()?,
    };
    let piece = |sign: f64| {
        let points = half(c, &values, sign);
        let mid = centroid(&points)?;
        let mut f = s.clone();
        f.depth = s.depth_at(mid).unwrap_or(s.depth);
        f.contours = vec![points];
        f.outline = s.outline.as_ref().and_then(|o| clip_to_side(o, c, &values, sign));
        Some(f)
    };
    Some((piece(1.0)?, piece(-1.0)?))
}

/// Curved outline `outline` of a face with polygon `c`, cut along the line where `values` (one per
/// point of `c`, varying linearly along each edge) is 0, keeping the side where `sign` times them is
/// at least 0. `None` when the line does not cross `c` at two points or nothing is left.
fn clip_to_side(outline: &PathData, c: &[Point], values: &[f64], sign: f64) -> Option<PathData> {
    let mut crossings = vec![];
    for (j, (&a, &va)) in c.iter().zip(values).enumerate() {
        let k = (j + 1) % c.len();
        let (Some(&b), Some(&vb)) = (c.get(k), values.get(k)) else { continue };
        if (va >= 0.0) != (vb >= 0.0) {
            crossings.push(a.lerp(b, va / (va - vb)));
        }
    }
    let &[q0, q1] = crossings.as_slice() else { return None };
    let along = q1 - q0;
    let length = along.hypot();
    if length.is_nan() || length <= 1e-9 {
        return None;
    }
    let along = along / length;
    let (inside, _) = c.iter().zip(values).find(|(_, v)| **v * sign > 0.0)?;
    let mut across = Vec2::new(-along.y, along.x);
    if (*inside - q0).dot(across) < 0.0 {
        across = -across;
    }
    let bounds = outline.bounds()?.union(polygon_bounds(c));
    let reach = 2.0 * (bounds.width() + bounds.height() + (q0 - bounds.center()).hypot()) + 1.0;
    let mut side = BezPath::new();
    side.move_to(q0 - along * reach);
    side.line_to(q0 + along * reach);
    side.line_to(q0 + along * reach + across * reach);
    side.line_to(q0 - along * reach + across * reach);
    side.close_path();
    let kept = vectorcraft_pathops::boolean(outline, FillRule::NonZero, &PathData::from_bezpath(&side), FillRule::NonZero, BoolOp::Intersect);
    kept.bounds().is_some().then_some(kept)
}

/// The part of polygon `p` where `sign` times `values` (one per point, varying linearly along each
/// edge) is at least 0.
fn half(p: &[Point], values: &[f64], sign: f64) -> Vec<Point> {
    let mut out = vec![];
    for (j, (&a, &va)) in p.iter().zip(values).enumerate() {
        let k = (j + 1) % p.len();
        let (Some(&b), Some(&vb)) = (p.get(k), values.get(k)) else { continue };
        let (sa, sb) = (va * sign, vb * sign);
        if sa >= 0.0 {
            out.push(a);
        }
        if (sa >= 0.0) != (sb >= 0.0) {
            out.push(a.lerp(b, sa / (sa - sb)));
        }
    }
    out
}

/// `order` (back to front), when it draws a face of one of a shape's strokes and an overlapping face
/// of the same shape's fill or strokes the wrong way round for their depths where they overlap on
/// the drawing, with every pair of overlapping faces that keep a polygon and a plane
/// ([`convex_face`]: turned toward the viewer, not a planar end), of any shape, put in the order of
/// their depths where they overlap: the face nearer the viewer there goes after the other. A fill
/// wall and a face of the same shape's covering stroke ([`Paints::Stroke`]) keep their order. Each
/// face otherwise keeps its place in `order`, so a fill wall in the gap between two dashes stays
/// where its depth puts it. A cycle of such pairs is broken at the face that comes first in `order`.
/// A face drawn last (a planar end turned toward the viewer, at depth `f64::MAX`) keeps its place:
/// no face moves across it. Otherwise `order` as it is.
fn local_order(faces: &[Placed], order: Vec<usize>) -> Vec<usize> {
    use std::cmp::Reverse;
    // The faces of each shape that take part, from left to right for a sweep along x.
    let mut shapes: Vec<Vec<usize>> = vec![];
    for (k, f) in faces.iter().enumerate() {
        if f.outline.len() < 3 || f.plane.is_none() {
            continue;
        }
        if shapes.len() <= f.member {
            shapes.resize_with(f.member + 1, Vec::new);
        }
        shapes[f.member].push(k);
    }
    if !shapes.iter().any(|list| list.iter().any(|&k| faces[k].paints != Paints::Fill)) {
        return order;
    }
    for list in &mut shapes {
        list.sort_by(|&a, &b| faces[a].bounds.x0.total_cmp(&faces[b].bounds.x0));
    }
    // A fill wall and a face of the same shape's covering stroke keep the order [`draw_order`] gave
    // them: the stroke's solid holds the wall, which never shows.
    let covered = |f: &Placed, s: &Placed| f.paints == Paints::Fill && f.side && s.paints == Paints::Stroke && f.member == s.member;
    // Is there a face of a shape's strokes and a face of the same shape's fill or strokes that the
    // order of their depths draws the wrong way round where they overlap?
    let mut position = vec![0usize; faces.len()];
    for (at, &k) in order.iter().enumerate() {
        position[k] = at;
    }
    let mut checks = 0usize;
    let mut misordered = false;
    'shapes: for list in &shapes {
        for (i, &f) in list.iter().enumerate() {
            let a = &faces[f];
            for &s in list.iter().skip(i + 1).take_while(|&&s| faces[s].bounds.x0 < a.bounds.x1) {
                if checks >= MAX_LOCAL_CHECKS {
                    break 'shapes;
                }
                checks += 1;
                let b = &faces[s];
                if (a.paints == Paints::Fill && b.paints == Paints::Fill) || covered(a, b) || covered(b, a) || !rects_overlap(a.bounds, b.bounds) {
                    continue;
                }
                if nearer_where_overlapping(a, b).is_some_and(|a_nearer| a_nearer != (position[f] > position[s])) {
                    misordered = true;
                    break 'shapes;
                }
            }
        }
    }
    if !misordered {
        return order;
    }
    // Moving those faces must not carry them past any other face they overlap, so every pair of
    // faces with a polygon and a plane that overlap goes in the order of their depths there.
    let mut sweep: Vec<usize> = (0..faces.len()).filter(|&k| faces[k].outline.len() >= 3 && faces[k].plane.is_some()).collect();
    sweep.sort_by(|&a, &b| faces[a].bounds.x0.total_cmp(&faces[b].bounds.x0));
    // For each face, the faces drawn after it, and for each, how many go before it.
    let mut after: Vec<Vec<usize>> = vec![vec![]; faces.len()];
    let mut waits = vec![0usize; faces.len()];
    'pairs: for (i, &k) in sweep.iter().enumerate() {
        let a = &faces[k];
        for &j in sweep.iter().skip(i + 1).take_while(|&&j| faces[j].bounds.x0 < a.bounds.x1) {
            if checks >= MAX_LOCAL_CHECKS {
                break 'pairs;
            }
            checks += 1;
            let b = &faces[j];
            if !rects_overlap(a.bounds, b.bounds) {
                continue;
            }
            let a_later = if covered(a, b) || covered(b, a) {
                position[k] > position[j]
            } else {
                let Some(a_nearer) = nearer_where_overlapping(a, b) else { continue };
                a_nearer
            };
            let (back, front) = if a_later { (j, k) } else { (k, j) };
            after[back].push(front);
            waits[front] += 1;
        }
    }
    // Each step draws the face that comes first in `order` of those whose faces before are drawn.
    let mut ready: std::collections::BinaryHeap<Reverse<(usize, usize)>> =
        (0..faces.len()).filter(|&k| waits[k] == 0).map(|k| Reverse((position[k], k))).collect();
    // The faces drawn last, in `order`: each goes only after every face before it in `order`.
    let last: Vec<usize> = order.iter().copied().filter(|&k| faces[k].depth >= f64::MAX).collect();
    let mut next_last = 0usize;
    let mut drawn = vec![false; faces.len()];
    let mut out = Vec::with_capacity(order.len());
    let mut first = 0usize;
    while out.len() < order.len() {
        while last.get(next_last).is_some_and(|&k| drawn[k]) {
            next_last += 1;
        }
        let limit = last.get(next_last).map_or(usize::MAX, |&k| position[k]);
        let k = match ready.peek() {
            Some(&Reverse((at, k))) if at < limit => {
                ready.pop();
                k
            }
            _ => {
                // A cycle, or the faces left to draw before a face drawn last wait for faces after
                // it: the first face in `order` not drawn yet goes next.
                while order.get(first).is_some_and(|&k| drawn[k]) {
                    first += 1;
                }
                let Some(&k) = order.get(first) else { break };
                k
            }
        };
        if drawn[k] {
            continue;
        }
        drawn[k] = true;
        out.push(k);
        for &j in &after[k] {
            waits[j] = waits[j].saturating_sub(1);
            if waits[j] == 0 && !drawn[j] {
                ready.push(Reverse((position[j], j)));
            }
        }
    }
    out
}

/// Whether face `a` is nearer the viewer than face `b` where their polygons overlap on the drawing,
/// measured at the middle of the overlap. `None` when they do not overlap, when either polygon is
/// not convex, or when they are at the same depth there.
fn nearer_where_overlapping(a: &Placed, b: &Placed) -> Option<bool> {
    let (pa, pb) = (a.plane?, b.plane?);
    let turn = convex(&b.outline)?;
    convex(&a.outline)?;
    let overlap = clip_convex(&a.outline, &b.outline, turn);
    let mid = centroid(&overlap)?;
    let (da, db) = (pa.depth_at(mid)?, pb.depth_at(mid)?);
    let tolerance = 1e-6 * (1.0 + da.abs().max(db.abs()));
    ((da - db).abs() > tolerance).then_some(da > db)
}

/// The sign of the turns of convex polygon `p` (`None` when it is not convex or has no area).
fn convex(p: &[Point]) -> Option<f64> {
    let n = p.len();
    let mut sign = 0.0;
    for i in 0..n {
        let (a, b, c) = (p[i], p[(i + 1) % n], p[(i + 2) % n]);
        let cross = (b - a).cross(c - b);
        if cross.abs() <= 1e-12 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return None;
        }
    }
    (sign != 0.0).then_some(sign)
}

/// The part of polygon `subject` inside convex polygon `clip`, whose turns have sign `turn`
/// (Sutherland–Hodgman).
fn clip_convex(subject: &[Point], clip: &[Point], turn: f64) -> Vec<Point> {
    let mut out = subject.to_vec();
    for i in 0..clip.len() {
        if out.is_empty() {
            break;
        }
        let (e0, e1) = (clip[i], clip[(i + 1) % clip.len()]);
        let side = |p: Point| (e1 - e0).cross(p - e0) * turn;
        let input = std::mem::take(&mut out);
        for j in 0..input.len() {
            let (p, q) = (input[j], input[(j + 1) % input.len()]);
            let (sp, sq) = (side(p), side(q));
            if sp >= 0.0 {
                out.push(p);
            }
            if (sp >= 0.0) != (sq >= 0.0) {
                let t = sp / (sp - sq);
                out.push(p.lerp(q, t));
            }
        }
    }
    out
}

/// The centroid of polygon `p`, `None` when it has (almost) no area. It is measured from the
/// polygon's first point, so a small polygon far from the origin keeps its area.
fn centroid(p: &[Point]) -> Option<Point> {
    let origin = *p.first()?;
    let n = p.len();
    let (mut area, mut cx, mut cy) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (a, b) = (p[i] - origin, p[(i + 1) % n] - origin);
        let cross = a.x * b.y - b.x * a.y;
        area += cross;
        cx += (a.x + b.x) * cross;
        cy += (a.y + b.y) * cross;
    }
    (area.abs() > 1e-9).then(|| origin + Vec2::new(cx / (3.0 * area), cy / (3.0 * area)))
}

/// The most bounds comparisons [`draw_order`] makes; the fill walls it has not reached by then keep
/// their depths.
const MAX_OVERLAP_CHECKS: usize = 4_000_000;

/// The order faces `faces` are drawn in, back to front, as indices into `faces`: by depth (faces
/// at the same depth in the order given), except that with `sealed` (solids with caps) a side face
/// of a shape's fill turned toward the viewer is drawn before each face of the same shape's
/// covering strokes ([`Paints::Stroke`]), turned toward the viewer, whose bounds overlap its own.
/// Such a stroke's closed solid holds the fill's walls, so they never show and can be drawn under
/// any other face. A shape's fill faces and the faces of its other strokes then go in the order of
/// their depths where they overlap ([`local_order`]).
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
    local_order(faces, order)
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
/// fill walls before the faces of its covering strokes that overlap them, and its fill's faces and
/// its other strokes' faces, cut where they pass through each other ([`cut_stroke_faces`]), in the
/// order of their depths where they overlap: [`draw_order`]), the
/// faces of a solid in a gradient with transparent stops in runs that carry its opacity mask
/// ([`in_runs`]);
/// otherwise each solid is a surface of its own, in that order. A member's opacity mask and
/// blending mode, and those of a group it is in, apply to the surfaces of the members it holds
/// ([`masked`]). The art comes in two parts: the effect's shadow ([`cast_shadow`], none with it
/// off), to be drawn behind every surface of the object, and the surfaces.
fn art_for(doc: Option<&Document>, n: &Node, fx: &Effect, shapes: &[Profile]) -> Option<ShadowAndSurfaces> {
    let art = surfaces_for(doc, n, fx, shapes)?;
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

/// The graphics of a 3D effect with parameters `p`, drawn on its faces ([`graphics::face_art`]):
/// for each planar face with a graphic, the art and the face. `caps` are the faces the viewer sees
/// of the ends of the object's solids, each with its end; `bounds` are the bounds the solids were
/// built with; `opacity` is the opacity of the fill or stroke those solids are painted with, which
/// the graphics take too. `elements` is the effect's count of drawn path elements, shared by every
/// call for one effect ([`graphics::face_art`]).
fn graphic_art(
    doc: Option<&Document>,
    id: NodeId,
    (effect, p): (&str, &Value),
    bounds: Rect,
    caps: &[&Facet],
    opacity: f32,
    elements: &mut usize,
) -> Vec<(PlanarFace, Arc<Node>)> {
    let list = graphics::mapped_graphics(p);
    if list.is_empty() || !takes_graphics(effect) {
        return vec![];
    }
    let view = view_options(p);
    PlanarFace::ALL
        .into_iter()
        .filter_map(|face| {
            let on: Vec<&Facet> = caps.iter().copied().filter(|f| f.end == Some(face)).collect();
            let map = if effect == ROTATE {
                vectorcraft_three_d::plane_face_map(bounds, view)
            } else {
                vectorcraft_three_d::extrude_face_map(bounds, extrude_options(p), view, face)
            }?;
            graphics::face_art(doc, id, &list, (face, &map), bounds.center(), &on, elements).map(|mut art| {
                art.opacity *= opacity;
                (face, Arc::new(art))
            })
        })
        .collect()
}

/// A solid (a fill's or a stroke's) with ends the viewer sees: the index of its surface, its
/// member, the ends, the opacity and blending mode of its fill or stroke, and whether its surface
/// carries an opacity mask (a gradient's transparency under a colored light, [`surface`]).
struct Filled {
    at: usize,
    member: usize,
    caps: Vec<Facet>,
    opacity: f32,
    blend: BlendMode,
    masked: bool,
}

/// Does a surface in `paint` carry an opacity mask ([`surface`])?
fn masks_surface(paint: &Paint, colored: bool) -> bool {
    colored && crate::lighting::opaque_paint(paint).1.is_some()
}

/// The graphics of the solids `filled` ([`graphic_art`]), each with the index of the surface it is
/// drawn right after (the last of those solids holding its face) and its face. Solids in a run
/// with the same opacity and blending mode share their graphics. With `per_member`
/// (a member's opacity mask or blending mode applies to its surfaces) each member's solids get
/// graphics of their own, clipped to that member's caps, so they stay in that member's scopes.
/// All the groups share one limit on the path elements drawn: once it is reached, later groups
/// draw no graphics.
fn placed_graphics(
    doc: Option<&Document>,
    id: NodeId,
    effect: (&str, &Value),
    bounds: Rect,
    filled: &[Filled],
    per_member: bool,
) -> Vec<(usize, PlanarFace, Arc<Node>)> {
    let mut groups: Vec<Vec<&Filled>> = vec![];
    for f in filled {
        // Solids whose fills or strokes differ in opacity or blending mode get graphics of their
        // own, each drawn as its fill or stroke is. A solid whose surface carries an opacity mask
        // gets its own too, which takes that mask ([`with_graphics`]). A solid joins only the
        // group of the solid before it, so a graphic is never drawn past a solid painted between.
        let joins = |first: &Filled| {
            (!per_member || first.member == f.member) && first.opacity == f.opacity && first.blend == f.blend && !first.masked && !f.masked
        };
        match groups.last_mut().filter(|g| g.first().is_some_and(|first| joins(first))) {
            Some(g) => g.push(f),
            None => groups.push(vec![f]),
        }
    }
    let mut out = vec![];
    let mut elements = 0usize;
    for group in groups {
        let caps: Vec<&Facet> = group.iter().flat_map(|f| &f.caps).collect();
        let opacity = group.first().map_or(1.0, |f| f.opacity);
        let blend = group.first().map_or(BlendMode::Normal, |f| f.blend);
        for (face, mut art) in graphic_art(doc, id, effect, bounds, &caps, opacity, &mut elements) {
            if let Some(f) = group.iter().rev().find(|f| f.caps.iter().any(|c| c.end == Some(face))) {
                Arc::make_mut(&mut art).blend = blend;
                out.push((f.at, face, art));
            }
        }
    }
    out
}

/// One solid's surface as [`surfaces_for`] builds it: its faces' depths, its art, its member's
/// scopes, and for each face the end of a solid it is, when the viewer sees it.
type BuiltSurface<'s> = (Vec<Placed>, Node, &'s [Scope], Vec<Option<PlanarFace>>);

/// [`with_graphics`] of built surfaces, each drawn whole.
fn whole_surfaces<'s>(surfaces: Vec<BuiltSurface<'s>>, arts: &[(usize, PlanarFace, Arc<Node>)]) -> Vec<(&'s [Scope], Arc<Node>)> {
    with_graphics(surfaces.into_iter().map(|(_, s, scopes, _)| (scopes, Arc::new(s))).collect(), 0, arts)
}

/// `surfaces` (those from index `first`) drawn whole, each with its scopes, and the graphics `arts`
/// ([`placed_graphics`]) of those surfaces each right after its surface, with that surface's scopes
/// and its opacity mask (a gradient's transparency), so a graphic is hidden where its cap is.
fn with_graphics<'s>(
    surfaces: Vec<(&'s [Scope], Arc<Node>)>,
    first: usize,
    arts: &[(usize, PlanarFace, Arc<Node>)],
) -> Vec<(&'s [Scope], Arc<Node>)> {
    let mut out = Vec::with_capacity(surfaces.len() + arts.len());
    for (k, (scopes, s)) in surfaces.into_iter().enumerate() {
        let mask = s.extras().mask.clone();
        out.push((scopes, s));
        out.extend(arts.iter().filter(|(at, ..)| *at == first + k).map(|(.., art)| {
            let mut art = art.clone();
            if let Some(mask) = &mask {
                Arc::make_mut(&mut art).extras_mut().mask = Some(mask.clone());
            }
            (scopes, art)
        }));
    }
    out
}

/// The ends of `f` the viewer sees, for a fill's solid or a stroke's: where its graphics go.
fn seen_ends(f: &[Facet]) -> impl Iterator<Item = &Facet> {
    f.iter().filter(|f| f.end.is_some() && !f.away)
}

/// [`art_for`] without the shadow: the solids' surfaces, with the graphics mapped onto their planar
/// faces drawn right after the last of the solids' surfaces that holds that face.
fn surfaces_for(doc: Option<&Document>, n: &Node, fx: &Effect, shapes: &[Profile]) -> Option<Vec<Arc<Node>>> {
    let b = union_bounds(shapes)?;
    let p = crate::merged_params(&fx.id, &fx.params);
    let classic = fx.id == CLASSIC_EXTRUDE || fx.id == CLASSIC_ROTATE;
    let colored = !classic && view_options(&p).has_colored_lights();
    let masks = has_masks(shapes);
    // The paint copies of every solid together: past the allowance the source path is drawn.
    let mut paint_bytes = 0usize;
    // The caps are copied only when the effect has graphics to draw on them.
    let graphics_on = !classic && takes_graphics(&fx.id) && !mapped_graphics(&p).is_empty();
    if is_plane(&fx.id) {
        // Every part turns about the centre of the object's own bounds (a group's members about
        // the centre of theirs), so they stay together.
        let mut out = vec![];
        let mut filled = vec![];
        for (member, s) in shapes.iter().enumerate() {
            for item in &s.items {
                let (paint, opacity, blend) = paint_of(item, s.opacity);
                let (area, rule) = region(&s.path, s.rule, item);
                let f = facets(ROTATE, &area, rule, b, &p).ok()?;
                if !crate::revolve::paint_copies_add(&mut paint_bytes, facet_copies(&f, &paint, colored), &paint) {
                    return None;
                }
                if graphics_on {
                    let caps: Vec<Facet> = seen_ends(&f).cloned().collect();
                    if !caps.is_empty() {
                        filled.push(Filled { at: out.len(), member, caps, opacity, blend, masked: masks_surface(&paint, colored) });
                    }
                }
                let art = if classic { classic::surface(n, f, &paint, opacity, blend, &p) } else { surface(n, f, &paint, opacity, blend, colored) };
                out.push((s.scopes.as_slice(), Arc::new(art)));
            }
        }
        let arts = placed_graphics(doc, n.id, (ROTATE, &p), b, &filled, masks);
        let out = with_graphics(out, 0, &arts);
        return Some(if masks { masked(n.id, out) } else { out.into_iter().map(|(_, s)| s).collect() });
    }
    let (parts, bounds) = solid_parts(shapes, b);
    let mut solids = vec![];
    for ((item, opacity, _, _, scopes, member), f) in parts.iter().zip(built_solids(&fx.id, &p, &parts, bounds, MAX_OBJECT_FACES)) {
        let item: &AppearanceItem = item;
        match f {
            Ok(Some(f)) if !crate::revolve::paint_copies_add(&mut paint_bytes, facet_copies(&f, paint_ref(item), colored), paint_ref(item)) => {
                return None;
            }
            Ok(Some(f)) => solids.push((item, *opacity, *scopes, f, *member)),
            // A stroke's outline past the limits is left out; the fill's solid still draws.
            Ok(None) => {}
            // A loaded file or a shape edited past the limits (its own, or those of the object's
            // solids together) keeps drawing its source path instead of failing the frame;
            // commands validate the same evaluator and report its errors.
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
    let closed = |shape: usize| shapes.get(shape).is_some_and(|s| holds_edge(&fx.id, &s.path));
    // A solid whose surface is one composite of all its facets ([`surface`]: a solid color under
    // colored lights with smooth shading) has no faces to order.
    let composite = |item: &AppearanceItem, f: &[Facet]| colored && paint_ref(item).color().is_some() && f.iter().any(|f| f.rgb_gradient.is_some());
    // The faces are drawn in one order (below) unless a composite draws the surfaces whole. Under
    // a member's opacity mask or blending, each member's faces go in one order of their own when its
    // fills and strokes are opaque, blend normally, carry no opacity mask and are not composites.
    let one_order = opaque && !solids.iter().any(|(item, _, _, f, _)| composite(item, f));
    let shape_order: Vec<bool> = (0..shapes.len())
        .map(|shape| {
            solids.iter().filter(|s| s.4 == shape).all(|(item, member, _, f, _)| {
                let (paint, opacity, blend) = paint_of(item, *member);
                opacity >= 1.0 && blend == BlendMode::Normal && !masks_surface(&paint, colored) && !composite(item, f)
            })
        })
        .collect();
    let ordered = |shape: usize| if masks { shape_order.get(shape).copied().unwrap_or(false) } else { one_order };
    if solids.iter().any(|s| ordered(s.4)) {
        // A stroke face that passes through a fill wall or another stroke's face is cut where it
        // does. The pieces count against the limits as the faces did: cutting stops before the
        // faces pass MAX_OBJECT_FACES or their paint copies pass the allowance, so an effect that
        // draws uncut still draws.
        let counts: Vec<usize> = solids.iter().map(|(.., f, _)| f.len()).collect();
        let room = cut_room(solids.iter().map(|(item, ..)| *item), counts.iter().sum(), paint_bytes);
        let mut cuts: Vec<(Paints, usize, &mut Vec<Facet>)> = solids
            .iter_mut()
            .filter(|(.., shape)| ordered(*shape))
            .map(|(item, _, _, f, shape)| (Paints::of(item, closed(*shape)), *shape, f))
            .collect();
        cut_stroke_faces(&mut cuts, room);
        for ((item, _, _, f, _), before) in solids.iter().zip(counts) {
            let added = f.get(before..).unwrap_or_default();
            if !crate::revolve::paint_copies_add(&mut paint_bytes, facet_copies(added, paint_ref(item), colored), paint_ref(item)) {
                return None;
            }
        }
    }
    let arts = if !graphics_on {
        vec![]
    } else {
        let filled: Vec<Filled> = solids
            .iter()
            .enumerate()
            .filter_map(|(at, (item, member, _, f, shape))| {
                let caps: Vec<Facet> = seen_ends(f).cloned().collect();
                let (paint, opacity, blend) = paint_of(item, *member);
                let masked = masks_surface(&paint, colored);
                (!caps.is_empty()).then_some(Filled { at, member: *shape, caps, opacity, blend, masked })
            })
            .collect();
        placed_graphics(doc, n.id, (&fx.id, &p), bounds, &filled, masks)
    };
    let members: Vec<usize> = solids.iter().map(|s| s.4).collect();
    let surfaces: Vec<BuiltSurface> = solids
        .into_iter()
        .map(|(item, member, scopes, f, shape)| {
            let paints = Paints::of(item, closed(shape));
            let depths: Vec<Placed> = f.iter().map(|f| Placed::of(f, shape, paints)).collect();
            let ends = f.iter().map(|f| f.end.filter(|_| !f.away)).collect();
            let (paint, opacity, blend) = paint_of(item, member);
            let art = if classic { classic::surface(n, f, &paint, opacity, blend, &p) } else { surface(n, f, &paint, opacity, blend, colored) };
            (depths, art, scopes, ends)
        })
        .collect();
    let sealed = match fx.id.as_str() {
        INFLATE => inflate_options(&p).cap,
        CLASSIC_EXTRUDE => classic_extrude_options(&p).cap,
        _ => extrude_options(&p).cap,
    };
    // One order takes a plain group of faces from each solid, one face for each facet. With
    // colored lights a surface can instead be one clipped composite of all its facets. A composite
    // has no faces to order, so it draws whole.
    let plain = |(depths, s, _, _): &BuiltSurface| matches!(&s.kind, NodeKind::Group { children, clip: false } if children.len() == depths.len());
    if masks {
        // Each member's surfaces, in the scopes of its opacity mask or blending: its faces in one
        // order when it is opaque and plain, otherwise each surface whole.
        let mut out = vec![];
        let mut surfaces = surfaces.into_iter().zip(members).enumerate().peekable();
        while let Some((first, (s, member))) = surfaces.next() {
            let mut run = vec![s];
            while let Some((_, (s, _))) = surfaces.next_if(|(_, (_, m))| *m == member) {
                run.push(s);
            }
            let flat = run.iter().all(|s| plain(s) && s.1.opacity >= 1.0 && s.1.blend == BlendMode::Normal && s.1.extras().mask.is_none());
            if ordered(member) && flat {
                let scopes = run.first().map_or(&[][..], |s| s.2);
                out.push((scopes, Arc::new(Node::group(n.id, in_one_order(n.id, run, first, &arts, sealed)))));
            } else {
                out.extend(with_graphics(run.into_iter().map(|(_, s, scopes, _)| (scopes, Arc::new(s))).collect(), first, &arts));
            }
        }
        return Some(masked(n.id, out));
    }
    if !opaque || !surfaces.iter().all(plain) {
        return Some(whole_surfaces(surfaces, &arts).into_iter().map(|(_, s)| s).collect());
    }
    Some(vec![Arc::new(Node::group(n.id, in_one_order(n.id, surfaces, 0, &arts, sealed)))])
}

/// The faces of `surfaces` (plain groups of their faces, [`surfaces_for`]'s from index `first`) in
/// one order ([`draw_order`], `sealed` for solids with caps), each of the graphics `arts` of those
/// surfaces right after the last face it is drawn on, of the surfaces that share it. A surface in a gradient with transparent stops
/// carries an opacity mask: its faces go in runs ([`in_runs`]), each carrying the mask, and each of
/// its graphics carries the mask too ([`with_graphics`]), so a graphic is hidden where its cap is.
fn in_one_order(id: NodeId, surfaces: Vec<BuiltSurface<'_>>, first: usize, arts: &[(usize, PlanarFace, Arc<Node>)], sealed: bool) -> Vec<Arc<Node>> {
    let masks: Vec<Option<Box<OpacityMask>>> = surfaces.iter().map(|(_, s, _, _)| s.extras().mask.clone()).collect();
    let arts: Vec<&(usize, PlanarFace, Arc<Node>)> = arts.iter().filter(|(at, ..)| (first..first + surfaces.len()).contains(at)).collect();
    let (mut placed, mut faces, mut ends) = (vec![], vec![], vec![]);
    for (surface, (depths, s, _, e)) in surfaces.into_iter().enumerate() {
        if let NodeKind::Group { children, .. } = s.kind {
            placed.extend(depths);
            faces.extend(children.into_iter().map(|face| Some((surface, face))));
            ends.extend(e.into_iter().map(|end| end.map(|end| (surface, end))));
        }
    }
    let order = draw_order(&placed, sealed);
    // Each graphic goes right after the last of the faces it is drawn on, among the surfaces that
    // share it ([`placed_graphics`]): a masked surface's own, or else the run of surfaces without a
    // mask that ends at its surface.
    let last: Vec<Option<usize>> = arts
        .iter()
        .map(|(at, face, _)| {
            let at = at - first;
            let from = if masks.get(at).is_some_and(Option::is_some) {
                at
            } else {
                masks.get(..at).and_then(|before| before.iter().rposition(Option::is_some)).map_or(0, |k| k + 1)
            };
            order.iter().rposition(|k| ends.get(*k).copied().flatten().is_some_and(|(surface, end)| end == *face && (from..=at).contains(&surface)))
        })
        .collect();
    let mut drawn: Vec<(usize, Arc<Node>)> = Vec::with_capacity(order.len() + arts.len());
    for (at, k) in order.into_iter().enumerate() {
        drawn.extend(faces.get_mut(k).and_then(Option::take));
        drawn.extend(last.iter().zip(&arts).filter(|(l, _)| **l == Some(at)).map(|(_, (surface, .., art))| {
            let mut art = art.clone();
            if let Some(mask) = masks.get(surface - first).cloned().flatten() {
                Arc::make_mut(&mut art).extras_mut().mask = Some(mask);
            }
            // An index past the surfaces: the graphic goes in a run of its own, with its own mask.
            (masks.len(), art)
        }));
    }
    if masks.iter().any(Option::is_some) { in_runs(id, drawn, &masks) } else { drawn.into_iter().map(|(_, face)| face).collect() }
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
        return group_solid_art(doc, n);
    }
    let (path, rule) = profile(n)?;
    let ctx = GeomContext::of(n);
    let paints = solid_paints(n);
    let mut children = vec![];
    let remaining: Vec<Effect>;
    if let Some((index, fx)) = solid(&n.appearance.effects) {
        let path = geometry(&path, n.appearance.effects.get(..index)?, &ctx);
        let (shadow, surfaces) = art_for(doc, n, fx, &[Profile { path, rule, items: paints, opacity: 1.0, scopes: vec![] }])?;
        children.extend(shadow.into_iter().chain(surfaces));
        // Later geometry effects act on the projected art as a whole.
        remaining = past_effect(&n.appearance.effects, index);
    } else {
        let path = geometry(&path, &n.appearance.effects, &ctx);
        remaining = crate::without_geometry(&n.appearance.effects);
        // The object's Transform effects, applied to the path above, transform the items' raster
        // effects too.
        let mut paints = paints;
        crate::raster::follow_paints(&mut paints, &n.appearance.effects);
        // Every item's shadow goes behind every item's art.
        let mut shadows = vec![];
        for item in &paints {
            if let Some((index, fx)) = solid(item.effects()) {
                let p = geometry(&path, item.effects().get(..index)?, &ctx.item(item));
                let (shadow, surfaces) = art_for(doc, n, fx, &[Profile { path: p, rule, items: vec![item.clone()], opacity: 1.0, scopes: vec![] }])?;
                let later: Vec<Effect> = item.effects().iter().skip(index + 1).filter(|e| !is_three_d(&e.id)).cloned().collect();
                let with_later = |mut art: Arc<Node>| {
                    Arc::make_mut(&mut art).appearance.effects = later.clone();
                    art
                };
                shadows.extend(shadow.into_iter().map(with_later));
                children.extend(surfaces.into_iter().map(with_later));
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

/// [`solid_art`] of group or layer `n`: its object-level 3D effect on its members together. The
/// geometry effects before it reshape the members, those after it bend the result as a whole; the
/// group's own fills and strokes are not drawn. `None` when the effect is only on one of the
/// group's own fills or strokes, or when its members can't become solids.
fn group_solid_art(doc: Option<&Document>, n: &Node) -> Option<Node> {
    let (index, fx) = solid(&n.appearance.effects)?;
    let mut shapes = members(n, "").ok()?;
    reshape_members(n, n.appearance.effects.get(..index)?, &mut shapes);
    let (shadow, surfaces) = art_for(doc, n, fx, &shapes)?;
    let children = shadow.into_iter().chain(surfaces).collect();
    let remaining = past_effect(&n.appearance.effects, index);
    let mut art = n.clone();
    art.kind = NodeKind::Group { children, clip: false };
    art.appearance = Appearance { effects: remaining, ..Default::default() }.into();
    Some(art)
}

/// Where the graphics of the 3D effect of `n` (on the object itself with `item` `None`, else on its
/// fill or stroke `item`) are drawn: the bounds its solids are built with and the maps of their
/// planar faces ([`graphics::GraphicFrame`]). `None` when `n` has no visible Extrude & Bevel or
/// Rotate there, or its art can't be built.
pub fn graphic_frame(doc: Option<&Document>, n: &Node, item: Option<usize>) -> Option<graphics::GraphicFrame> {
    frame_of(doc, n, item, &solid)
}

/// [`graphic_frame`] of the 3D effect at `index` in that list, which must be a visible Extrude &
/// Bevel or Rotate.
pub fn graphic_frame_at(doc: Option<&Document>, n: &Node, item: Option<usize>, index: usize) -> Option<graphics::GraphicFrame> {
    frame_of(doc, n, item, &|fx: &[Effect]| fx.get(index).filter(|e| e.visible && SOLIDS.contains(&e.id.as_str())).map(|e| (index, e)))
}

/// Does a visible geometry effect come after the 3D effect at `index` in the effects of `n` (on the
/// object itself with `item` `None`, else on its fill or stroke `item`)? Such an effect moves or
/// bends the drawn art, graphics included, while [`graphic_frame_at`] places the graphics on the
/// art before it.
pub fn graphics_reshaped_after(n: &Node, item: Option<usize>, index: usize) -> bool {
    n.appearance.effects_at(item).and_then(|fx| fx.get(index + 1..)).is_some_and(crate::has_geometry)
}

/// Finds a 3D effect in a list of effects, with its index there.
type Pick<'p> = dyn for<'e> Fn(&'e [Effect]) -> Option<(usize, &'e Effect)> + 'p;

/// [`graphic_frame`] of the 3D effect `pick` finds in the effects of `n` or its `item`.
fn frame_of(doc: Option<&Document>, n: &Node, item: Option<usize>, pick: &Pick<'_>) -> Option<graphics::GraphicFrame> {
    let n = &*resolve(doc, n, "").ok()?;
    let (fx, shapes) = if is_container(n) {
        let (index, fx) = pick(&n.appearance.effects).filter(|_| item.is_none())?;
        let mut shapes = members(n, "").ok()?;
        reshape_members(n, n.appearance.effects.get(..index)?, &mut shapes);
        (fx, shapes)
    } else {
        let (path, rule) = profile(n)?;
        let ctx = GeomContext::of(n);
        match item {
            None => {
                let (index, fx) = pick(&n.appearance.effects)?;
                let path = geometry(&path, n.appearance.effects.get(..index)?, &ctx);
                (fx, vec![Profile { path, rule, items: solid_paints(n), opacity: 1.0, scopes: vec![] }])
            }
            Some(k) => {
                let it = n.appearance.items.get(k)?;
                let (index, fx) = pick(it.effects())?;
                let path = geometry(&geometry(&path, &n.appearance.effects, &ctx), it.effects().get(..index)?, &ctx.item(it));
                (fx, vec![Profile { path, rule, items: vec![it.clone()], opacity: 1.0, scopes: vec![] }])
            }
        }
    };
    if !takes_graphics(&fx.id) {
        return None;
    }
    let p = crate::merged_params(&fx.id, &fx.params);
    let b = union_bounds(&shapes)?;
    let view = view_options(&p);
    if fx.id == ROTATE {
        let plane = vectorcraft_three_d::plane_face_map(b, view);
        return Some(graphics::GraphicFrame::new(b, plane, None));
    }
    let (_, bounds) = solid_parts(&shapes, b);
    let options = extrude_options(&p);
    let map = |face| vectorcraft_three_d::extrude_face_map(bounds, options, view, face);
    Some(graphics::GraphicFrame::new(bounds, map(PlanarFace::Front), map(PlanarFace::Back)))
}

/// The bounds of the art symbol `name` of `doc` draws as a graphic, in the symbol's coordinates.
pub fn graphic_bounds(doc: &Document, name: &str) -> Option<Rect> {
    graphics::art_bounds(doc, name)
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
    // The solids the art draws, past the limits of the object's solids together as it is
    // ([`built_solids`]), each built and its faces counted as the loop reaches it, then dropped.
    let drawn = built_solids(&fx.id, &p, &parts, bounds, MAX_OBJECT_FACES).map(|r| r.map(|f| f.is_some()));
    // Inflate lays out every puff at once (they share one eye); each Inflate and Extrude & Bevel
    // solid and its mesh are built as the loop reaches them.
    let mut inflated = (fx.id == INFLATE).then(|| {
        let shapes: Vec<(&PathData, FillRule)> = parts.iter().map(|(_, _, area, rule, _, _)| (area, *rule)).collect();
        vectorcraft_three_d::inflate_meshes(&shapes, bounds, inflate_options(&p), view)
    });
    let extrude = if fx.id == CLASSIC_EXTRUDE { classic_extrude_options(&p) } else { extrude_options(&p) };
    for ((item, member, area, rule, _, _), drawn) in parts.iter().zip(drawn) {
        let item: &AppearanceItem = item;
        if !drawn? {
            // A stroke the art leaves out is left out of the meshes too; Inflate's next puff
            // still belongs to the next part.
            if let Some(built) = inflated.as_mut() {
                built.next();
            }
            continue;
        }
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
