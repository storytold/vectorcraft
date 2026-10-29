//! Object → Expand: live shapes, type and effects (Object), strokes (Stroke) and gradient and pattern
//! fills (Fill) into plain art in one undo step; blends, repeats and Intertwine groups become groups of their steps, of their instances and of what they draw. Placed documents (Object) become editable copies of the
//! art they show, their resources joining the document. A linear or radial gradient fill becomes a
//! gradient mesh, or solid strips / concentric ellipses, inside a clip group shaped like the object; a freeform
//! gradient becomes a mesh shaped like the object. A pattern fill becomes the tiles it paints inside a clip
//! group shaped like the object. Create Gradient Mesh shares the sampling.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::cms::Model;
use vectorcraft_color::freeform::spread_scale;
use vectorcraft_color::{BlendMode, Color, Gradient, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::appearance::{AppearanceItem, FillLayer};
use vectorcraft_doc::live::GradientMesh;
use vectorcraft_doc::pattern::{PatternDef, place_art};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, Scaling};
use vectorcraft_geom::{Affine, PathData, Point, Rect, SubPath, Vec2, shapes};

use super::edit::selected_roots;
use super::menucmds::{run_raw, squash};
use super::object::as_clipping_path;
use super::pathops::{assemble, node_path, outline_strokes, replace_leaves, styled_copy, with_transparency};
use super::{CommandSpec, EngineError, Result, Session, bad, bool_or, cmd, f64_or, has_selection, str_param};

const C: &str = "object.expand";
/// Expand Gradient To → Specify … Objects: the default and the most.
pub const DEFAULT_STEPS: u32 = 255;
pub const MAX_STEPS: u32 = 1000;
/// Rows and columns of the mesh a freeform gradient expands to.
const FREEFORM_MESH: u32 = 8;
/// Columns (around the centre) of the mesh a radial gradient expands to.
const RADIAL_MESH_COLS: usize = 8;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.expand",
            "Expand…",
            ["Object"],
            None,
            "{object?: true (type → outlines, live shapes → paths, effects baked, blends → groups of their steps, repeats → groups of their instances, Intertwine groups → groups of their objects and of the parts drawn over the others (clip groups), envelopes → their distorted content, placed documents → editable copies of their art, as placing the file without link gives, its symbols, patterns, swatches and images joining the document), fill?: true (gradient fills → `gradient` art, pattern fills → their tiles in a clip group shaped like the object), stroke?: true (strokes → filled outlines, the stroked paths that baking effects leaves included), gradient?: \"objects\" (default: `steps` solid strips, concentric ellipses for radial gradients) | \"mesh\" (a gradient mesh); both in a clip group shaped like the object (freeform gradients always become a mesh shaped like it), steps?: 1..1000 (255)} one undo step → {ids}",
            has_selection,
            expand
        ),
        cmd!(
            query "object.expand.info",
            "Expand Options",
            [],
            None,
            "{} what each Expand option would change in the selection → {object, fill, stroke, gradient, appearance} (true: something to expand; gradient: a gradient fill that Fill expands, which Expand Gradient To applies to; appearance: appearance attributes Expand Appearance expands first, for which Object › Expand… is dimmed in the menus)",
            has_selection,
            |s, _| {
                let e = Expandable::of(&s.doc()?.doc, &selected_roots(s)?);
                Ok(json!({"object": e.object(), "fill": e.fill, "stroke": e.stroke, "gradient": e.gradient, "appearance": has_appearance(s)}))
            }
        ),
    ]
}

/// Has the selection appearance attributes that Expand Appearance expands (effects, several fills
/// or strokes, a fill or stroke with its own opacity, blend mode or effects, a brush)? Object ›
/// Expand… is dimmed in the menus for them until Expand Appearance has turned them into plain art
/// (#3089); `object.expand` itself still bakes them.
pub fn has_appearance(s: &Session) -> bool {
    let (Ok(st), Ok(roots)) = (s.doc(), selected_roots(s)) else { return false };
    roots.iter().filter_map(|r| st.doc.node(*r)).any(super::effectcmd::expandable)
}

/// Every placed document in `roots` of `d` (at any depth) replaced by an editable copy of the art it
/// shows, keeping its id and place; the resources the art uses join `d` (a name `d` uses for
/// something else gets a free variant).
fn expand_placed_under(d: &mut Document, roots: &[NodeId]) -> Result<()> {
    let mut ids = vec![];
    for n in roots.iter().filter_map(|r| d.node(*r)) {
        n.walk(&mut |c| {
            if matches!(c.kind, NodeKind::PlacedDocument(_)) {
                ids.push(c.id);
            }
        });
    }
    ids.into_iter().try_for_each(|id| super::place::document::expand(d, id, C))
}

/// Expand Gradient To.
#[derive(Clone, Copy, Debug, PartialEq)]
enum GradientTo {
    Mesh,
    Objects(u32),
}

impl GradientTo {
    fn parse(p: &Value) -> Result<Self> {
        let steps = f64_or(p, "steps", DEFAULT_STEPS as f64).round();
        if !(1.0..=MAX_STEPS as f64).contains(&steps) {
            return Err(bad(C, format!("steps must be 1..{MAX_STEPS}")));
        }
        match str_param(p, "gradient").unwrap_or("objects") {
            "objects" => Ok(Self::Objects(steps as u32)),
            "mesh" => Ok(Self::Mesh),
            _ => Err(bad(C, "gradient must be \"objects\" or \"mesh\"")),
        }
    }
}

/// What the selection holds for each Expand option.
#[derive(Default)]
struct Expandable {
    text: bool,
    live: bool,
    effects: bool,
    fill: bool,
    /// Gradient fills (the ones Expand Gradient To applies to), a part of `fill`.
    gradient: bool,
    stroke: bool,
    /// An envelope (Object expands it into its distorted content).
    envelope: bool,
    /// Live blends (Object expands them into groups of their steps).
    blends: bool,
    /// Placed documents (Object expands them into the art they show).
    placed: bool,
    /// Repeats (Object expands them into groups of their instances).
    repeats: bool,
    /// Intertwine groups (Object expands them into groups of what they draw).
    intertwines: bool,
}

impl Expandable {
    fn of(d: &Document, roots: &[NodeId]) -> Self {
        let mut e = Self::default();
        for n in roots.iter().filter_map(|r| d.node(*r)) {
            n.walk(&mut |c| {
                let shape = matches!(c.kind, NodeKind::Path { .. } | NodeKind::Compound { .. });
                e.text |= matches!(c.kind, NodeKind::Text(_));
                e.live |= matches!(c.kind, NodeKind::Path { live: Some(_), .. });
                e.effects |= !c.appearance.effects.is_empty();
                e.envelope |= matches!(c.kind, NodeKind::Envelope { .. });
                e.blends |= matches!(c.kind, NodeKind::Blend { .. });
                e.placed |= matches!(c.kind, NodeKind::PlacedDocument(_));
                e.repeats |= matches!(c.kind, NodeKind::Repeat(_));
                e.intertwines |= matches!(c.kind, NodeKind::Intertwine { .. });
                e.stroke |= match &c.kind {
                    // Type's outlines take its characters' stroke (see `type.createOutlines`).
                    NodeKind::Text(t) => t.runs.iter().any(|r| r.style.has_stroke()),
                    _ => shape && !c.appearance.stroke_paint().is_none(),
                };
                e.gradient |= match &c.kind {
                    NodeKind::Text(t) => t.runs.iter().any(|r| matches!(r.style.fill, Paint::Gradient(_))),
                    _ => shape && c.appearance.items.iter().any(|i| gradient_fill(i).is_some()),
                };
                e.fill |= e.gradient;
                e.fill |= match &c.kind {
                    NodeKind::Text(t) => {
                        t.runs.iter().any(|r| matches!(&r.style.fill, Paint::Pattern { pattern, .. } if d.pattern(pattern).is_some()))
                    }
                    _ => shape && c.appearance.items.iter().any(|i| pattern_fill(d, i).is_some()),
                };
            });
        }
        e
    }

    fn object(&self) -> bool {
        self.text || self.live || self.effects || self.envelope || self.blends || self.placed || self.repeats || self.intertwines
    }
}

/// The gradient of a visible gradient fill.
fn gradient_fill(i: &AppearanceItem) -> Option<(&FillLayer, &GradientPaint)> {
    match i {
        AppearanceItem::Fill(f) if f.visible => match &f.paint {
            Paint::Gradient(g) => Some((f, g)),
            _ => None,
        },
        _ => None,
    }
}

/// The definition and placement of a visible pattern fill whose pattern `d` defines.
fn pattern_fill<'a, 'd>(d: &'d Document, i: &'a AppearanceItem) -> Option<(&'a FillLayer, &'d PatternDef, Affine)> {
    match i {
        AppearanceItem::Fill(f) if f.visible => match &f.paint {
            Paint::Pattern { pattern, xf } => Some((f, d.pattern(pattern)?, *xf)),
            _ => None,
        },
        _ => None,
    }
}

fn expand(s: &mut Session, p: &Value) -> Result<Value> {
    let object = bool_or(p, "object", true);
    let fill = bool_or(p, "fill", true);
    let stroke = bool_or(p, "stroke", true);
    let to = GradientTo::parse(p)?;
    let from = s.doc()?.history.undo.len();
    let rev0 = s.doc()?.revision;
    let mut e = Expandable::of(&s.doc()?.doc, &selected_roots(s)?);
    // Placed documents first: their art (type, strokes, gradients) expands below.
    if object && e.placed {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| expand_placed_under(d, &roots))?;
    }
    // Intertwine groups before what they hold: their objects, and the copies of the parts drawn
    // over the others, expand below.
    if object && e.intertwines {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| super::intertwine::expand_intertwines(d, &roots).map(|_| ()))?;
    }
    // Blends first: what their steps hold (type, effects, strokes, gradients) expands below.
    if object && e.blends {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| super::live::expand_blends(d, &roots).map(|_| ()))?;
    }
    // Repeats too: their instances are copies of art that expands below.
    if object && e.repeats {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| super::patterncmds::expand_repeats(d, &roots).map(|_| ()))?;
    }
    if object && e.effects {
        let _ = run_raw(s, "effect.expandAppearance", &json!({}));
        // What is left to expand is in the baked art: Expand Appearance leaves strokes strokes
        // (#3089), a group's own as stroked paths among its members, which Stroke outlines below.
        e = Expandable::of(&s.doc()?.doc, &selected_roots(s)?);
    }
    if object && e.envelope {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| {
            for r in &roots {
                super::live::expand_envelopes_under(d, *r)?;
            }
            Ok(())
        })?;
    }
    if object && e.live {
        let roots = selected_roots(s)?;
        s.edit("Expand", |d, _| {
            for r in &roots {
                if let Some(n) = d.node_mut(*r) {
                    drop_live(n);
                }
            }
            Ok(())
        })?;
    }
    if object && e.text {
        let _ = run_raw(s, "type.createOutlines", &json!({}));
    }
    if stroke && e.stroke {
        let _ = run_raw(s, "object.path.outlineStroke", &json!({}));
    }
    if fill {
        let _ = expand_gradient_fills(s, to);
        let _ = expand_pattern_fills(s);
    }
    if s.doc()?.revision == rev0 {
        return Err(EngineError::Other("Expand: nothing to expand".into()));
    }
    squash(s, from, "Expand");
    Ok(json!({ "ids": s.doc()?.selection.objects.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn drop_live(n: &mut Node) {
    if let NodeKind::Path { live, .. } = &mut n.kind {
        *live = None;
    }
    if let Some(ch) = n.children_mut() {
        for c in ch.iter_mut() {
            drop_live(Arc::make_mut(c));
        }
    }
}

/// Replace every gradient fill of the selected paths by art (see [`expand_fills`]).
fn expand_gradient_fills(s: &mut Session, to: GradientTo) -> Result<()> {
    let roots = selected_roots(s)?;
    s.edit("Expand", |d, sel| {
        let (mut ids, mut changed) = (vec![], 0);
        for r in &roots {
            let (id, n) = replace_leaves(d, *r, |d, l| expand_fills(d, l, to))?;
            ids.push(id);
            changed += n;
        }
        if changed == 0 {
            return Err(EngineError::Other("Expand: no gradient fills".into()));
        }
        sel.set(ids);
        Ok(())
    })
}

/// Path `l` with each gradient fill turned into art, in appearance order: the other items stay
/// on plain copies of the path. The path's geometry effects ([`reshaped`]) and each fill's own
/// ([`fill_shape`]) shape the outline the art fills first, as on the canvas. `None` when no
/// gradient fill can be expanded.
fn expand_fills(d: &mut Document, l: &Node, to: GradientTo) -> Option<Node> {
    if !matches!(l.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) || !l.appearance.items.iter().any(|i| gradient_fill(i).is_some()) {
        return None;
    }
    let reshaped = reshaped(l);
    let l = reshaped.as_ref().unwrap_or(l);
    let (path, _) = node_path(l)?;
    let bounds = path.bounds()?;
    let mut parts = vec![];
    let mut rest = vec![];
    let mut expanded = false;
    for item in &l.appearance.items {
        // Items that paint nothing go; a gradient that can't be expanded stays.
        let art = gradient_fill(item).and_then(|(f, g)| {
            let (fl, path, bounds) = fill_shape(l, item, &path, bounds)?;
            gradient_art(d, &fl, &path, bounds, f, g, to)
        });
        let Some(art) = art else {
            if item.visible() && !item.paint().is_none() {
                rest.push(item.clone());
            }
            continue;
        };
        if !rest.is_empty() {
            parts.push(styled_copy(d, l, std::mem::take(&mut rest)));
        }
        parts.push(art);
        expanded = true;
    }
    if !expanded {
        return None;
    }
    if !rest.is_empty() {
        parts.push(styled_copy(d, l, rest));
    }
    assemble(d, l, parts)
}

/// Gradient fill `f` (gradient `g`) of path `l` (outline `path`, box `bounds`) as art, with the
/// fill's opacity, blend mode and effects other than geometry effects ([`fill_effects`]).
fn gradient_art(d: &mut Document, l: &Node, path: &PathData, bounds: Rect, f: &FillLayer, g: &GradientPaint, to: GradientTo) -> Option<Node> {
    let content = if g.gradient.kind == GradientKind::Freeform {
        let m = GradientMesh::for_path_with(path, FREEFORM_MESH, FREEFORM_MESH, &*gradient_sampler(g, bounds, d.color_mode.model()))?;
        vec![Node::new(d.alloc_id(), NodeKind::Mesh(m))]
    } else {
        let frame = Frame::new(g, bounds);
        match to {
            GradientTo::Mesh => vec![Node::new(d.alloc_id(), NodeKind::Mesh(frame.mesh(&g.gradient)))],
            GradientTo::Objects(n) => frame
                .pieces(&g.gradient, n as usize)
                .into_iter()
                .map(|(shape, (color, opacity))| {
                    let fill = FillLayer { opacity, overprint: f.overprint, ..FillLayer::new(Paint::solid(color)) };
                    Node::path(d.alloc_id(), shape, Appearance { items: vec![AppearanceItem::Fill(fill)], ..Default::default() })
                })
                .collect(),
        }
    };
    // A freeform mesh takes the object's outline: only holes need the object to clip it.
    let art = if g.gradient.kind == GradientKind::Freeform && path.subpaths.len() == 1 {
        content.into_iter().next()?
    } else {
        let mut clip = styled_copy(d, l, vec![]);
        as_clipping_path(&mut clip).ok()?;
        let children = std::iter::once(clip).chain(content).map(Arc::new).collect();
        Node::new(d.alloc_id(), NodeKind::Group { children, clip: true })
    };
    let mut art = with_transparency(d, art, f.opacity, f.blend);
    art.appearance.effects = fill_effects(f);
    Some(art)
}

/// Path `l` as the canvas paints its fills and strokes: its visible geometry effects applied to its
/// path (a plain path then), and every geometry effect of its own removed. `None` when it has no
/// visible geometry effect.
fn reshaped(l: &Node) -> Option<Node> {
    use vectorcraft_render::effects::{GeomContext, apply_geometry_with, has_geometry, is_geometry};
    if !has_geometry(&l.appearance.effects) {
        return None;
    }
    let (path, _) = node_path(l)?;
    let g = apply_geometry_with(&l.appearance.effects, &path, path.bounds()?, &GeomContext::of(l));
    let mut m = with_path(l, g);
    m.appearance.effects.retain(|e| !is_geometry(&e.id));
    Some(m)
}

/// Path `l` (outline `path`, box `bounds`) as its fill `item` paints it on the canvas, with its
/// outline and box: when the fill has visible geometry effects, `l` with the outline they make of
/// `path` as a plain path, and that outline's box.
fn fill_shape(l: &Node, item: &AppearanceItem, path: &PathData, bounds: Rect) -> Option<(Node, PathData, Rect)> {
    use vectorcraft_render::effects::{GeomContext, apply_geometry_with, has_geometry};
    if !has_geometry(item.effects()) {
        return Some((l.clone(), path.clone(), bounds));
    }
    // In the fill's context, as drawing runs them: a plug-in effect is told which fill it is on.
    let outline = apply_geometry_with(item.effects(), path, bounds, &GeomContext::of(l).item(item));
    let b = outline.bounds()?;
    Some((with_path(l, outline.clone()), outline, b))
}

/// Path or compound path `l` with `path` as its path, a plain path.
fn with_path(l: &Node, path: PathData) -> Node {
    let mut m = l.clone();
    m.kind = match &l.kind {
        NodeKind::Path { rule, clipping, guide, .. } => NodeKind::Path { path, rule: *rule, live: None, clipping: *clipping, guide: *guide },
        NodeKind::Compound { rule, .. } => NodeKind::Path { path, rule: *rule, live: None, clipping: false, guide: false },
        k => k.clone(),
    };
    m
}

/// The effects of fill `f` that stay on its expanded art: all but its geometry effects, which
/// shape the outline it paints ([`fill_shape`]).
fn fill_effects(f: &FillLayer) -> Vec<vectorcraft_doc::Effect> {
    f.effects.iter().filter(|e| !vectorcraft_render::effects::is_geometry(&e.id)).cloned().collect()
}

/// Replace every pattern fill of the selected paths by the tiles it paints (see [`expand_patterns`]).
fn expand_pattern_fills(s: &mut Session) -> Result<()> {
    let roots = selected_roots(s)?;
    s.edit("Expand", |d, sel| {
        let (mut ids, mut changed) = (vec![], 0);
        for r in &roots {
            let (id, n) = replace_leaves(d, *r, expand_patterns)?;
            ids.push(id);
            changed += n;
        }
        if changed == 0 {
            return Err(EngineError::Other("Expand: nothing to expand".into()));
        }
        sel.set(ids);
        Ok(())
    })
}

/// Expand the pattern fills under `root` that paint a transparent image into the tiles they paint
/// ([`expand_patterns_where`]), so each tile image becomes an object of its own; the other
/// pattern fills stay. A tile image counts as
/// transparent when its key is in `see_through` (it has see-through pixels), or when it, a group
/// in the tile art, the fill, the shape or a group the shape is in has opacity or a blending
/// mode. Returns how many shapes changed.
pub(crate) fn expand_transparent_image_patterns(d: &mut Document, root: NodeId, see_through: &BTreeSet<String>) -> Result<usize> {
    let (_, n) = replace_leaves(d, root, |d, l| {
        // Most leaves have no pattern fill: skip the ancestry walk for them.
        if !l.appearance.items.iter().any(|i| pattern_fill(d, i).is_some()) {
            return None;
        }
        let shape = d.ancestry(l.id).unwrap_or_default().iter().filter_map(|a| d.node(*a)).any(blended);
        let transparent = |d: &Document, i: &AppearanceItem| {
            pattern_fill(d, i).is_some_and(|(f, def, _)| {
                let on = shape || f.opacity < 1.0 || f.blend != BlendMode::Normal;
                def.art.iter().any(|a| transparent_image(a, see_through, on))
            })
        };
        expand_patterns_where(d, l, transparent)
    })?;
    Ok(n)
}

/// Whether `n` has opacity or a blending mode of its own.
fn blended(n: &Node) -> bool {
    n.opacity < 1.0 || n.blend != BlendMode::Normal
}

/// Whether `n` is or holds a transparent image: one with see-through pixels (key in `keys`), or
/// with opacity or a blending mode of its own or of a group in `n`, or anywhere when `on`.
fn transparent_image(n: &Node, keys: &BTreeSet<String>, on: bool) -> bool {
    let on = on || blended(n);
    match &n.kind {
        NodeKind::Image(im) => on || keys.contains(&im.key),
        _ => n.children().is_some_and(|ch| ch.iter().any(|c| transparent_image(c, keys, on))),
    }
}

/// Path `l` with each pattern fill turned into the tiles it paints over the box of the outline it
/// fills, in a clip group shaped like that outline with the fill's opacity, blend mode and effects
/// other than geometry effects, in appearance order: the other items stay on plain copies of the
/// path. The path's geometry effects ([`reshaped`]) and each fill's own ([`fill_shape`]) shape
/// that outline first, as on the canvas. The tiles are drawn as the canvas draws them
/// ([`pattern_tiles`]). A pattern fill that lays no tile there stays. `None` when no pattern fill
/// lays a tile.
fn expand_patterns(d: &mut Document, l: &Node) -> Option<Node> {
    expand_patterns_where(d, l, |_, _| true)
}

/// [`expand_patterns`] for only the pattern fills `which` picks: the others stay like the other
/// items.
fn expand_patterns_where(d: &mut Document, l: &Node, which: impl Fn(&Document, &AppearanceItem) -> bool) -> Option<Node> {
    if !matches!(l.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })
        || !l.appearance.items.iter().any(|i| pattern_fill(d, i).is_some() && which(d, i))
    {
        return None;
    }
    let reshaped = reshaped(l);
    let l = reshaped.as_ref().unwrap_or(l);
    let (path, _) = node_path(l)?;
    let bounds = path.bounds()?;
    let mut parts = vec![];
    let mut rest = vec![];
    let mut expanded = false;
    for item in &l.appearance.items {
        let fill = pattern_fill(d, item).filter(|_| which(d, item)).map(|(f, def, xf)| (f, def.clone(), xf));
        let tiles = fill.and_then(|(f, def, xf)| {
            let (fl, _, bounds) = fill_shape(l, item, &path, bounds)?;
            let t = pattern_tiles(d, &def, xf, bounds);
            (!t.is_empty()).then_some((f, fl, t))
        });
        let Some(art) = tiles.and_then(|(f, fl, t)| tile_art(d, &fl, f, &t)) else {
            if item.visible() && !item.paint().is_none() {
                rest.push(item.clone());
            }
            continue;
        };
        if !rest.is_empty() {
            parts.push(styled_copy(d, l, std::mem::take(&mut rest)));
        }
        parts.push(art);
        expanded = true;
    }
    if !expanded {
        return None;
    }
    if !rest.is_empty() {
        parts.push(styled_copy(d, l, rest));
    }
    assemble(d, l, parts)
}

/// The tiles pattern `def` placed by `xf` lays over `bounds` in document `d`, as the canvas draws
/// them: their stroke weights and effects scale with the pattern, and when it also turns, flips,
/// stretches or shears them, the strokes (brushed ones included) and geometry effects of their
/// paths and opacity masks become filled art and their live shapes become plain paths first. The
/// same happens when it scales art with geometry effects (Roughen's detail, for one, is per inch
/// and does not scale). No tiles when it turns, flips, stretches or shears art with raster or 3D
/// effects or with a geometry effect on a group or type, or scales art with a geometry effect on a
/// group or type, which the canvas draws turned or scaled with the tile and live effects would
/// not, and when it does more than move art that holds a symbol instance, whose strokes the canvas
/// draws scaled with the tile and an instance does not scale. Random Transform effects and brushes
/// that vary at random in the art are baked first ([`seeds_baked`], [`random_brushes_baked`]).
fn pattern_tiles(d: &mut Document, def: &PatternDef, xf: Affine, bounds: Rect) -> Vec<Node> {
    if !moves_only(xf) && def.art.iter().any(|a| has_instances(a)) {
        return vec![];
    }
    let Some(def) = seeds_baked(def) else {
        return vec![];
    };
    let brushes = vectorcraft_brush::library(d);
    let def = random_brushes_baked(d, &brushes, def);
    let def = &*def;
    let scaling = Scaling { strokes: true, effects: Some(vectorcraft_render::effects::scale_effect), patterns: true, ..Scaling::default() };
    if scales_evenly(xf) && (moves_only(xf) || !def.art.iter().any(|a| has_geometry_effects(a))) {
        return def.instances_in_with(xf, bounds, scaling);
    }
    if def.art.iter().any(|a| has_unbakeable_effects(a)) {
        return vec![];
    }
    let placed = Scaling { patterns: true, ..Scaling::default() };
    let mut tiles = def.instances_in_with(Affine::IDENTITY, xf.inverse().transform_rect_bbox(bounds), placed);
    for t in &mut tiles {
        *t = prepared(d, &brushes, t);
        place_art(t, xf, scaling);
    }
    tiles
}

/// Pattern `def` with each path in its art (opacity masks included) that has a random Transform
/// effect baked into filled art ([`bake_appearance`](vectorcraft_render::effects::bake_appearance))
/// while it keeps its own id: the effect takes its seed from the id, and the canvas draws the art
/// with those ids for every tile, while the tiles get ids of their own. `None` when something
/// other than a path has such an effect.
fn seeds_baked(def: &PatternDef) -> Option<std::borrow::Cow<'_, PatternDef>> {
    if !def.art.iter().any(|a| has_seeded(a)) {
        return Some(std::borrow::Cow::Borrowed(def));
    }
    let mut def = def.clone();
    def.art = def.art.iter().map(|a| bake_seeded(a).map(Arc::new)).collect::<Option<_>>()?;
    Some(std::borrow::Cow::Owned(def))
}

/// Pattern `def` with each path in its art (opacity masks included) whose stroke has a brush that
/// varies at random ([`draws_random_art`]) [`prepared`] where the pattern defines it: the brush
/// takes its seed from the path's points, and the canvas draws the art there for every tile.
fn random_brushes_baked<'a>(
    d: &mut Document,
    brushes: &[vectorcraft_brush::Brush],
    def: std::borrow::Cow<'a, PatternDef>,
) -> std::borrow::Cow<'a, PatternDef> {
    if !def.art.iter().any(|a| has_random_brush(brushes, a)) {
        return def;
    }
    let mut def = def.into_owned();
    def.art = def.art.iter().map(|a| Arc::new(bake_random_brushes(d, brushes, a))).collect();
    std::borrow::Cow::Owned(def)
}

/// Does brush `name` of `brushes` draw different art for different points: a calligraphic or
/// scatter brush with Random variation, or a bristle brush?
fn draws_random_art(brushes: &[vectorcraft_brush::Brush], name: &str) -> bool {
    use vectorcraft_brush::{BrushKind, Variation};
    brushes.iter().find(|b| b.name == name).is_some_and(|b| match &b.kind {
        BrushKind::Calligraphic(c) => c.modes().contains(&Variation::Random),
        BrushKind::Scatter(s) => s.modes().contains(&Variation::Random),
        BrushKind::Bristle(_) => true,
        BrushKind::Art(_) | BrushKind::Pattern(_) => false,
    })
}

/// Is `n` a path with a visible stroke whose brush varies at random ([`draws_random_art`])?
fn random_brushed(brushes: &[vectorcraft_brush::Brush], n: &Node) -> bool {
    let random = |st: &vectorcraft_doc::appearance::StrokeLayer| st.visible && st.brush.as_deref().is_some_and(|b| draws_random_art(brushes, b));
    matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })
        && n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(st) if random(st)))
}

/// Does `n`, something in it or one of their opacity masks have a stroke whose brush varies at
/// random ([`random_brushed`])?
fn has_random_brush(brushes: &[vectorcraft_brush::Brush], n: &Node) -> bool {
    random_brushed(brushes, n)
        || n.extras().mask.as_ref().is_some_and(|m| has_random_brush(brushes, &m.art))
        || (!matches!(n.kind, NodeKind::Compound { .. }) && n.children().is_some_and(|c| c.iter().any(|c| has_random_brush(brushes, c))))
}

/// `n` with the paths in it whose brush varies at random [`prepared`] ([`random_brushes_baked`]).
fn bake_random_brushes(d: &mut Document, brushes: &[vectorcraft_brush::Brush], n: &Node) -> Node {
    if !has_random_brush(brushes, n) {
        return n.clone();
    }
    if random_brushed(brushes, n) {
        return prepared(d, brushes, n);
    }
    let mut n = n.clone();
    if !matches!(n.kind, NodeKind::Compound { .. }) {
        for c in n.children_mut().into_iter().flatten() {
            *c = Arc::new(bake_random_brushes(d, brushes, c));
        }
    }
    if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut()) {
        m.art = Arc::new(bake_random_brushes(d, brushes, &m.art));
    }
    n
}

/// Does `n`, something in it or one of their opacity masks have a visible geometry effect?
fn has_geometry_effects(n: &Node) -> bool {
    let geometry = |e: &vectorcraft_doc::Effect| e.visible && vectorcraft_render::effects::is_geometry(&e.id);
    n.appearance.effects.iter().any(geometry)
        || n.appearance.items.iter().any(|i| i.effects().iter().any(geometry))
        || n.extras().mask.as_ref().is_some_and(|m| has_geometry_effects(&m.art))
        || n.children().is_some_and(|c| c.iter().any(|c| has_geometry_effects(c)))
}

/// Does `n`, something in it or one of their opacity masks hold a symbol instance?
fn has_instances(n: &Node) -> bool {
    matches!(n.kind, NodeKind::SymbolInstance { .. })
        || n.extras().mask.as_ref().is_some_and(|m| has_instances(&m.art))
        || n.children().is_some_and(|c| c.iter().any(|c| has_instances(c)))
}

/// Does `a` only move (no scale, turn, flip, stretch or shear)?
fn moves_only(a: Affine) -> bool {
    let [m0, m1, m2, m3, _, _] = a.as_coeffs();
    let eps = 1e-9;
    (m0 - 1.0).abs() <= eps && m1.abs() <= eps && m2.abs() <= eps && (m3 - 1.0).abs() <= eps
}

/// Does `n` have a visible Transform effect with Random on, its own or a fill's or stroke's?
fn seeded_here(n: &Node) -> bool {
    let seeded =
        |e: &vectorcraft_doc::Effect| e.visible && e.id == "distort.transform" && e.params.get("random").and_then(Value::as_bool) == Some(true);
    n.appearance.effects.iter().any(seeded) || n.appearance.items.iter().any(|i| i.effects().iter().any(seeded))
}

/// Does `n`, something in it or one of their opacity masks have a random Transform effect
/// ([`seeded_here`])?
pub(crate) fn has_seeded(n: &Node) -> bool {
    seeded_here(n)
        || n.extras().mask.as_ref().is_some_and(|m| has_seeded(&m.art))
        || (!matches!(n.kind, NodeKind::Compound { .. }) && n.children().is_some_and(|c| c.iter().any(|c| has_seeded(c))))
}

/// `n` with the paths in it that have a random Transform effect baked ([`seeds_baked`]). `None`
/// when something else has one.
fn bake_seeded(n: &Node) -> Option<Node> {
    if !has_seeded(n) {
        return Some(n.clone());
    }
    let mut n = if seeded_here(n) {
        if !matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) {
            return None;
        }
        vectorcraft_render::effects::bake_appearance(n)?
    } else {
        n.clone()
    };
    if !matches!(n.kind, NodeKind::Compound { .. }) {
        for c in n.children_mut().into_iter().flatten() {
            *c = Arc::new(bake_seeded(c)?);
        }
    }
    if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut()) {
        m.art = Arc::new(bake_seeded(&m.art)?);
    }
    Some(n)
}

/// Does `n`, something in it or one of their opacity masks have a visible effect that [`prepared`]
/// cannot bake: a raster or 3D effect, or a geometry effect on something other than a path?
fn has_unbakeable_effects(n: &Node) -> bool {
    use vectorcraft_render::effects::{is_geometry, is_raster, is_three_d};
    let path = matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. });
    let unbakeable = |e: &vectorcraft_doc::Effect| e.visible && (is_raster(&e.id) || is_three_d(&e.id) || (!path && is_geometry(&e.id)));
    n.appearance.effects.iter().any(unbakeable)
        || n.appearance.items.iter().any(|i| i.effects().iter().any(unbakeable))
        || n.extras().mask.as_ref().is_some_and(|m| has_unbakeable_effects(&m.art))
        || n.children().is_some_and(|c| c.iter().any(|c| has_unbakeable_effects(c)))
}

/// Tile art `n` ready for a turn, flip, stretch or shear: the strokes and geometry effects of its
/// paths baked ([`baked`], [`outline_brushed`]) and its live shapes turned into plain paths (a
/// stretch would rebuild their corners round), in its opacity masks too.
fn prepared(d: &mut Document, brushes: &[vectorcraft_brush::Brush], n: &Node) -> Node {
    let mut t = baked(n);
    outline_brushed(d, brushes, &mut t);
    drop_live(&mut t);
    prepare_masks(d, brushes, &mut t);
    t
}

/// The opacity masks of `n` and of everything in it [`prepared`].
fn prepare_masks(d: &mut Document, brushes: &[vectorcraft_brush::Brush], n: &mut Node) {
    if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut()) {
        m.art = Arc::new(prepared(d, brushes, &m.art));
    }
    for c in n.children_mut().into_iter().flatten() {
        prepare_masks(d, brushes, Arc::make_mut(c));
    }
}

/// `n` with each path in it that has a brushed stroke turned into filled art and brush art
/// ([`outline_strokes`]).
fn outline_brushed(d: &mut Document, brushes: &[vectorcraft_brush::Brush], n: &mut Node) {
    if matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) {
        let brushed = n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(st) if st.brush.is_some()));
        if brushed && let Some(o) = outline_strokes(d, brushes, n) {
            *n = o;
        }
        return;
    }
    for c in n.children_mut().into_iter().flatten() {
        outline_brushed(d, brushes, Arc::make_mut(c));
    }
}

/// Is `a` a move and a positive uniform scale (which brush nibs, strokes and effects follow by
/// scaling alone)?
fn scales_evenly(a: Affine) -> bool {
    let [m0, m1, m2, m3, _, _] = a.as_coeffs();
    let eps = 1e-9 * m0.abs().max(m3.abs());
    m0 > 0.0 && m1.abs() <= eps && m2.abs() <= eps && (m0 - m3).abs() <= eps
}

/// `n` with the strokes and geometry effects of each path in it baked into filled art
/// ([`bake_appearance`](vectorcraft_render::effects::bake_appearance)).
fn baked(n: &Node) -> Node {
    if let Some(b) = vectorcraft_render::effects::bake_appearance(n) {
        return b;
    }
    let mut n = n.clone();
    for c in n.children_mut().into_iter().flatten() {
        *c = Arc::new(baked(c));
    }
    n
}

/// Pattern tiles `tiles` of fill `f` of path `l` as a clip group shaped like `l` that isolates
/// their blending (as the canvas draws a tile on its own), with the fill's opacity, blend mode and
/// effects other than geometry effects ([`fill_effects`]).
fn tile_art(d: &mut Document, l: &Node, f: &FillLayer, tiles: &[Node]) -> Option<Node> {
    let mut clip = styled_copy(d, l, vec![]);
    as_clipping_path(&mut clip).ok()?;
    let tiles: Vec<Node> = tiles.iter().map(|t| d.reid(t)).collect();
    let children = std::iter::once(clip).chain(tiles).map(Arc::new).collect();
    let mut art = Node::new(d.alloc_id(), NodeKind::Group { children, clip: true });
    art.isolate = true;
    let mut art = with_transparency(d, art, f.opacity, f.blend);
    art.appearance.effects = fill_effects(f);
    Some(art)
}

/// The colour and opacity gradient fill `g` paints at a document point of an object whose box is
/// `bounds`, as the canvas draws it (the end colours continue past the ends). Colours stay in the
/// color model their stops share; display RGB where models differ. Freeform points mix in the
/// model they share ([`vectorcraft_color::Freeform::model_field`]); where their models differ, the
/// RGB color they mix to is stored in `doc_model` (the document's model) the way `paint.setFill`
/// stores an RGB color ([`super::paint::stored_color`]).
pub(crate) fn gradient_sampler(g: &GradientPaint, bounds: Rect, doc_model: Model) -> Box<dyn Fn(Point) -> (Color, f32) + '_> {
    if g.gradient.kind == GradientKind::Freeform {
        let b = bounds.abs();
        let field = g.freeform_on(b).model_field(spread_scale(b));
        let store_in = (field.model().is_none() && doc_model != Model::Rgb).then_some(doc_model);
        return Box::new(move |p| {
            let (c, a) = field.sample(p);
            (super::paint::stored_color(store_in, c), a)
        });
    }
    let geom = g.resolve(bounds);
    Box::new(move |p| g.gradient.sample_keep_model(geom.param_at(g.gradient.kind, p) as f32))
}

/// Where the colour changes along a gradient between parameters `t0` and `t1`: both ends, every
/// stop between them (in its own colour, so coincident stops keep a sharp edge) and every midpoint
/// off the centre, as (parameter, colour, opacity) in order, with more between them where they
/// are over `max_gap` apart. The colour is linear in display RGB between neighbours, as the
/// canvas draws it.
fn breaks(g: &Gradient, t0: f64, t1: f64, max_gap: f64) -> Vec<(f64, (Color, f32))> {
    let at = |t: f64| (t, g.sample_keep_model(t as f32));
    let mut v = vec![at(t0)];
    // A Perceptual gradient curves between its stops: the mesh takes a column at each of the stops
    // its expansion adds, so the linear mix between columns follows that curve.
    if !g.method.is_classic() {
        v.extend(g.expanded().map(|(o, ..)| f64::from(o)).filter(|o| *o > t0 && *o < t1).map(at));
    }
    for (i, s) in g.stops.iter().enumerate() {
        let o = s.offset as f64;
        if o > t0 && o < t1 {
            v.push((o, (s.color, s.opacity)));
        }
        if let Some(n) = g.stops.get(i + 1)
            && (s.midpoint - 0.5).abs() > 1e-3
        {
            let m = (s.offset + (n.offset - s.offset) * s.midpoint) as f64;
            if m > t0 && m < t1 {
                v.push(at(m));
            }
        }
    }
    v.push(at(t1));
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::with_capacity(v.len());
    for w in v.windows(2) {
        out.push(w[0]);
        let (a, b) = (w[0].0, w[1].0);
        let n = ((b - a) / max_gap).ceil() as usize;
        out.extend((1..n).map(|k| at(a + (b - a) * k as f64 / n as f64)));
    }
    out.extend(v.last().copied());
    out
}

/// A linear or radial gradient's geometry over an object's box: its start, the vector to its
/// end, the vector across it (radial: the ellipse's other axis), and the parameter ranges that
/// cover the box (linear: along the vector `t0..t1` and across it `s0..s1` in points; radial:
/// `0..t1`, each level's ellipse centred on the way from the focal point to the start).
struct Frame {
    radial: bool,
    start: Point,
    focal: Point,
    along: Vec2,
    across: Vec2,
    t0: f64,
    t1: f64,
    s0: f64,
    s1: f64,
}

impl Frame {
    fn new(g: &GradientPaint, bounds: Rect) -> Self {
        let geom = g.resolve(bounds);
        let radial = g.gradient.kind == GradientKind::Radial;
        // A vector of no length paints as one pointing right (as the canvas does).
        let along = if geom.start.distance(geom.end) < 1e-9 { Vec2::new(1.0, 0.0) } else { geom.end - geom.start };
        let normal = Vec2::new(-along.y, along.x) / along.hypot();
        let across = if radial { normal * along.hypot() * geom.aspect } else { normal };
        let corners = [bounds.origin(), Point::new(bounds.x1, bounds.y0), Point::new(bounds.x1, bounds.y1), Point::new(bounds.x0, bounds.y1)];
        let range = |f: &dyn Fn(Point) -> f64| corners.iter().map(|c| f(*c)).fold((f64::MAX, f64::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
        let (mut t0, mut t1) = range(&|c| (c - geom.start).dot(along) / along.hypot2());
        let (s0, s1) = range(&|c| (c - geom.start).dot(normal));
        if radial {
            let geom = vectorcraft_color::GradientGeom { end: geom.start + along, ..geom };
            (t0, t1) = (0.0, range(&|c| geom.param_at(GradientKind::Radial, c)).1);
        }
        // Keep a sliver of span so the art maps back invertibly.
        t1 = t1.max(t0 + 1e-6);
        Self { radial, start: geom.start, focal: geom.focal_point(), along, across, t0, t1, s0, s1 }
    }

    /// Linear: the point at parameter `t` along and `s` across. Radial: the point at parameter `t`
    /// and angle `s` (radians).
    fn at(&self, t: f64, s: f64) -> Point {
        if self.radial {
            self.focal + (self.start - self.focal + self.along * s.cos() + self.across * s.sin()) * t
        } else {
            self.start + self.along * t + self.across * s
        }
    }

    /// A gradient mesh covering the box: columns (linear) or rings (radial) where the colour
    /// changes, so it paints the gradient exactly.
    fn mesh(&self, g: &Gradient) -> GradientMesh {
        // Rings as close as the columns around them, so the mesh draws smoothly.
        let gap = if self.radial { (self.t1 - self.t0) / RADIAL_MESH_COLS as f64 } else { f64::INFINITY };
        let br = breaks(g, self.t0, self.t1, gap);
        let ts: Vec<f64> = br.iter().map(|(t, _)| (t - self.t0) / (self.t1 - self.t0)).collect();
        let t = |x: f64| self.t0 + x * (self.t1 - self.t0);
        if self.radial {
            let angles: Vec<f64> = (0..=RADIAL_MESH_COLS).map(|i| i as f64 / RADIAL_MESH_COLS as f64).collect();
            GradientMesh::from_grid(&angles, &ts, &|u, v| self.at(t(v), u * std::f64::consts::TAU), &|r, _, _| br[r].1)
        } else {
            GradientMesh::from_grid(&ts, &[0.0, 1.0], &|u, v| self.at(t(u), self.s0 + v * (self.s1 - self.s0)), &|_, c, _| br[c].1)
        }
    }

    /// `n` solid shapes covering the box in paint order, each coloured as the gradient in the
    /// middle of its band: for a linear gradient, rectangles from each band's start to the far
    /// end, for a radial one, nested ellipses (largest first). Each paints over the ones
    /// below except its band, so no seam shows between bands. With translucent stops each shape
    /// is just its band, so nothing doubles up.
    fn pieces(&self, g: &Gradient, n: usize) -> Vec<(PathData, (Color, f32))> {
        let step = (self.t1 - self.t0) / n as f64;
        let color = |k: usize| g.sample_keep_model((self.t0 + (k as f64 + 0.5) * step) as f32);
        let opaque = g.stops.iter().all(|s| s.opacity >= 1.0);
        if self.radial {
            let ellipse = |t: f64| {
                let c = self.focal + (self.start - self.focal) * t;
                let m = Affine::new([self.along.x * t, self.along.y * t, self.across.x * t, self.across.y * t, c.x, c.y]);
                shapes::ellipse(Rect::new(-1.0, -1.0, 1.0, 1.0)).transformed(m)
            };
            return (0..n)
                .rev()
                .map(|k| {
                    let mut shape = ellipse((k + 1) as f64 * step);
                    if !opaque && k > 0 {
                        let mut hole = ellipse(k as f64 * step).subpaths.remove(0);
                        hole.reverse();
                        shape.subpaths.push(hole);
                    }
                    (shape, color(k))
                })
                .collect();
        }
        (0..n)
            .map(|k| {
                let a = self.t0 + k as f64 * step;
                let b = if opaque { self.t1 } else { a + step };
                let pts = [self.at(a, self.s0), self.at(b, self.s0), self.at(b, self.s1), self.at(a, self.s1)];
                (PathData::single(SubPath::polyline(&pts, true)), color(k))
            })
            .collect()
    }
}
