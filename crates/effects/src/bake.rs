//! Baking live geometry effects for export.
//!
//! SVG, PDF and the clipboard have no notion of Illustrator's live effects, so exporters receive a
//! copy of the document in which every geometry effect — object level, per fill/stroke and
//! Effect → Pathfinder, and those on type, images, symbol instances and live objects (through their
//! outlines, [`reshape`]) — is evaluated exactly like the renderer does, and so are the own fills,
//! strokes and geometry effects of groups and layers ([`evaluate_container`]). Raster effects
//! (shadows, glows, blur) stay on the objects for the exporter to translate (e.g. SVG filters).
//! Colour adjustments recolour the objects ([`adjust_in_document`]; recoloured embedded images
//! become images of their own).
//!
//! Type that Transform effects on it, or on its group or layer, only move and copy stays type in
//! each place when its characters are in solid colors and the copies do not overlap ([`moved_type`]
//! and [`container_keeping_type`] list the other conditions). The renderer still draws its outlines.
//!
//! Object → Expand Appearance goes further ([`expand_leaf`]): every fill and stroke becomes an
//! object of its own, strokes turned into filled art by the caller (their outlines or brush art).

use std::collections::HashMap;
use std::sync::Arc;

use vectorcraft_doc::color::Paint;
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Effect, FillLayer, Knockout, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{Affine, FillRule, PathData, Rect, Vec2};

use crate::distort::{transform_move, transform_offsets};
use crate::group::{has_own_paint, paints};
use crate::reshape::{as_art, geometry_effects, glyph_outlines, outline_nested_text, reshape_leaves};
use crate::{
    GeomContext, adjust_in_document, apply_geometry_with, crop_marks_art, evaluate_container, has_adjustment, has_crop_marks, has_geometry,
    has_item_geometry, has_pathfinder, is_geometry, is_pathfinder, merged_params, needs_outline, outline_text, reshape,
};

fn item_effects(item: &AppearanceItem) -> &[vectorcraft_doc::Effect] {
    match item {
        AppearanceItem::Fill(f) => &f.effects,
        AppearanceItem::Stroke(s) => &s.effects,
    }
}

fn clear_item_effects(item: &mut AppearanceItem) {
    match item {
        AppearanceItem::Fill(f) => f.effects.retain(|e| !is_geometry(&e.id)),
        AppearanceItem::Stroke(s) => s.effects.retain(|e| !is_geometry(&e.id)),
    }
}

/// Does anything in `n`'s subtree need baking?
pub fn needs_bake(n: &Node) -> bool {
    crate::has_revolve(n)
        || crate::has_solid(n)
        || has_adjustment(n)
        || has_crop_marks(n)
        || has_pathfinder(n)
        || has_own_paint(n)
        || has_geometry(&n.appearance.effects)
        || n.projection().is_some()
        || n.appearance.items.iter().any(|i| has_geometry(item_effects(i)))
        || n.children().is_some_and(|ch| ch.iter().any(|c| needs_bake(c)))
}

fn geometry(n: &Node) -> Option<(PathData, FillRule)> {
    match &n.kind {
        NodeKind::Path { path, rule, guide: false, .. } => Some((path.clone(), *rule)),
        NodeKind::Compound { children, rule } => {
            Some((PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect()), *rule))
        }
        _ => None,
    }
}

fn apply(effects: &[vectorcraft_doc::Effect], path: &PathData, ctx: &GeomContext) -> PathData {
    if !has_geometry(effects) || path.is_empty() {
        return path.clone();
    }
    let b = vectorcraft_geom::Shape::bounding_box(&path.to_bezpath());
    apply_geometry_with(effects, path, b, ctx)
}

/// A path node of `kind`'s flavour (compound when the result has several subpaths).
fn path_kind(d: &mut Document, path: PathData, rule: FillRule) -> NodeKind {
    if path.subpaths.len() > 1 {
        let children = path.subpaths.into_iter().map(|sp| Arc::new(Node::path(d.alloc_id(), PathData::single(sp), Appearance::default()))).collect();
        NodeKind::Compound { children, rule }
    } else {
        NodeKind::Path { path, rule, live: None, clipping: false, guide: false }
    }
}

/// `n` with its geometry effects evaluated (`None` = unchanged).
fn bake_node(d: &mut Document, n: &Node) -> Option<Node> {
    // A cancelled export leaves the rest of the document as it is: the caller drops the result.
    if !needs_bake(n) || vectorcraft_doc::cancel::is_cancelled() {
        return None;
    }
    // Colour adjustments: the object recoloured (inside it too), then baked as it is.
    if has_adjustment(n)
        && let Some(m) = adjust_in_document(d, n)
    {
        return Some(bake_node(d, &m).unwrap_or(m));
    }
    // Crop marks: the object and its marks.
    if let Some(m) = crop_marks_art(n) {
        return Some(bake_pieces(d, m));
    }
    if let Some(m) = crate::revolve_art(n).or_else(|| crate::solid_art(n)) {
        let m = evaluate_container(Some(d), &m).unwrap_or(m);
        return Some(bake_pieces(d, m));
    }
    // Type, images, symbol instances and live objects: reshaped through their outlines, and so are
    // type whose own fills or strokes have geometry effects and images whose own fills or strokes
    // paint a shape ([`has_item_geometry`]). Type that Transform effects only move and copy stays
    // type, moved to each place.
    if needs_outline(n) && (has_geometry(&n.appearance.effects) || n.projection().is_some() || has_item_geometry(n)) {
        if let Some(m) = moved_type_group(n) {
            return Some(bake_pieces(d, m));
        }
        let symbol = match &n.kind {
            NodeKind::SymbolInstance { symbol, .. } => d.symbols.iter().find(|s| s.name == *symbol).map(|s| s.art.clone()),
            _ => None,
        };
        let m = reshape(Some(d), n, symbol.as_deref())?;
        return Some(bake_node(d, &m).unwrap_or(m));
    }
    // A path whose Transform effect copies it with a gradient, or scales its strokes or moves its
    // patterns: each copy an object of its own, so its gradient fits it rather than spanning
    // every copy and its strokes and patterns can differ from the others'.
    if crate::has_transform_art(n)
        && let Some(m) = reshape(Some(d), n, None)
    {
        return Some(bake_pieces(d, m));
    }
    // Groups and layers: Pathfinder, geometry effects and their own fills and strokes (type that
    // Transform effects only move stays type). The new pieces (results, outlines, copies, the
    // fills' and strokes' art) get ids of their own.
    if let Some(m) = container_keeping_type(d, n).or_else(|| evaluate_container(Some(d), n)) {
        return Some(bake_pieces(d, m));
    }
    if let Some(ch) = n.children() {
        if matches!(n.kind, NodeKind::Compound { .. })
            && (has_geometry(&n.appearance.effects) || n.appearance.items.iter().any(|i| has_geometry(item_effects(i))))
        {
            return bake_leaf(d, n);
        }
        let mut m = n.clone();
        let baked: Vec<Arc<Node>> = ch.iter().map(|c| bake_node(d, c).map(Arc::new).unwrap_or_else(|| c.clone())).collect();
        if let Some(ch) = m.children_mut() {
            *ch = baked;
        }
        return Some(m);
    }
    bake_leaf(d, n)
}

/// `m`, art evaluated from an object, with its children baked and then given ids of their own (the
/// pieces copied from the source keep the source's ids until then). The pieces are baked with the
/// ids the renderer draws them with: Random in a Transform effect takes its seed from the id.
fn bake_pieces(d: &mut Document, mut m: Node) -> Node {
    let mut seen = std::collections::HashSet::from([m.id]);
    let children = m.children().cloned().unwrap_or_default();
    let children = children
        .into_iter()
        .map(|c| {
            let c = Arc::unwrap_or_clone(c);
            let mut c = bake_node(d, &c).unwrap_or(c);
            fresh_ids(d, &mut c, &mut seen);
            Arc::new(c)
        })
        .collect();
    if let Some(ch) = m.children_mut() {
        *ch = children;
    }
    m
}

/// Give `n` and its descendants a new id wherever one was already `seen` (the pieces evaluated art
/// copies from its source keep ids of their own).
pub fn fresh_ids(d: &mut Document, n: &mut Node, seen: &mut std::collections::HashSet<vectorcraft_doc::NodeId>) {
    if !seen.insert(n.id) {
        n.id = d.alloc_id();
        seen.insert(n.id);
    }
    for c in n.children_mut().into_iter().flatten() {
        fresh_ids(d, Arc::make_mut(c), seen);
    }
}

/// Most copies of one type object that stay type, the original included ([`moved_type`]).
const MAX_TYPE_COPIES: usize = 10_000;
/// Most bytes that the copies of one type object that stay type take together in the document's
/// JSON form, the original included ([`moved_type`]): its text, frame or path, wrap shapes and the
/// rest of the object, once per copy.
const MAX_TYPE_COPY_BYTES: usize = 20_000_000;

/// The params of geometry effects `fx` when there are some and they are all Transform effects that
/// only move ([`transform_move`]), in order; `None` otherwise.
fn transform_moves(fx: &[&Effect]) -> Option<Vec<serde_json::Value>> {
    if fx.is_empty() {
        return None;
    }
    fx.iter().map(|e| (e.id == "distort.transform").then(|| merged_params(&e.id, &e.params)).filter(|p| transform_move(p).is_some())).collect()
}

/// Type `n` in each place that Transform effects that only move ([`transform_moves`]) copy its
/// glyph outlines to: `n` moved there, the original's place first. `None` for other objects; for
/// type that is hidden, in perspective, has effects, transparency, an opacity mask or fills and
/// strokes of its own (beyond its characters'), has characters painted with a gradient or pattern,
/// underlined or struck through, has character strokes and is scaled or sheared, holds inline
/// graphics or has no glyphs; when the effects would stop copies of its outlines early
/// ([`transform_offsets`]); when the copies would pass [`MAX_TYPE_COPIES`] or
/// [`MAX_TYPE_COPY_BYTES`]; and when the copies overlap: what one copy paints (its
/// character strokes included) reaches into another's box.
fn moved_type(n: &Node, moves: &[serde_json::Value]) -> Option<Vec<Node>> {
    let NodeKind::Text(t) = &n.kind else { return None };
    let flat = |p: &Paint| matches!(p, Paint::None | Paint::Solid { .. });
    let [a, b, c, d, _, _] = t.xf.as_coeffs();
    let rigid = (a * a + b * b - 1.0).abs() < 1e-9 && (c * c + d * d - 1.0).abs() < 1e-9 && (a * c + b * d).abs() < 1e-9;
    let stroked = t.runs.iter().any(|r| r.style.has_stroke());
    if !n.visible
        || (stroked && !rigid)
        || n.projection().is_some()
        || !n.has_default_transparency()
        || !n.appearance.effects.is_empty()
        || n.appearance.items.iter().any(|i| paints(i) || !item_effects(i).is_empty())
        || t.runs.iter().any(|r| r.inline.is_some() || r.style.underline || r.style.strikethrough || !flat(&r.style.fill) || !flat(&r.style.stroke))
    {
        return None;
    }
    let (_, all) = glyph_outlines(t);
    let outline = PathData::from_bezpath(&all).transformed(t.xf);
    let ink = outline.bounds()?;
    let offsets = transform_offsets(moves, &outline)?;
    if offsets.len() > MAX_TYPE_COPIES {
        return None;
    }
    let size = serde_json::to_vec(n).ok()?.len();
    if size.saturating_mul(offsets.len()) > MAX_TYPE_COPY_BYTES {
        return None;
    }
    let strokes = t.runs.iter().map(|r| r.style.appearance().outset()).fold(0.0, f64::max);
    if !apart(&offsets, ink.inflate(strokes, strokes)) {
        return None;
    }
    Some(
        offsets
            .into_iter()
            .map(|o| {
                let mut c = n.clone();
                c.transform(Affine::translate(o), false);
                c
            })
            .collect(),
    )
}

/// Do the copies of box `b` moved by each of `offsets` stay clear of each other? Copies may touch.
/// False as well when an offset is 10^15 or more times the box's width or height.
fn apart(offsets: &[Vec2], b: Rect) -> bool {
    let (w, h) = (b.width(), b.height());
    if offsets.len() < 2 {
        return true;
    }
    if !(w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite()) {
        return false;
    }
    // Copies closer than `w` across and `h` down lie in the same or a neighboring cell.
    let cell = |o: Vec2| {
        let (x, y) = ((o.x / w).floor(), (o.y / h).floor());
        (x.abs() < 1e15 && y.abs() < 1e15).then_some((x as i64, y as i64))
    };
    let mut cells: HashMap<(i64, i64), Vec2> = HashMap::new();
    for &o in offsets {
        let Some((x, y)) = cell(o) else { return false };
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(p) = cells.get(&(x + dx, y + dy))
                    && (p.x - o.x).abs() < w
                    && (p.y - o.y).abs() < h
                {
                    return false;
                }
            }
        }
        cells.insert((x, y), o);
    }
    true
}

/// Type `n` whose own geometry effects only move it ([`transform_moves`]) as a group of the moved
/// type ([`moved_type`]). The group keeps `n`'s id, name, visibility, transparency, opacity mask and
/// remaining (raster) effects, and the type in it has none of them. `None` otherwise.
fn moved_type_group(n: &Node) -> Option<Node> {
    if !matches!(n.kind, NodeKind::Text(_)) {
        return None;
    }
    let moves = transform_moves(&geometry_effects(n))?;
    let mut plain = n.clone();
    plain.visible = true;
    plain.opacity = 1.0;
    plain.blend = Default::default();
    plain.isolate = false;
    plain.knockout_shape = false;
    plain.extras_mut().mask = None;
    plain.appearance.effects.clear();
    let copies = moved_type(&plain, &moves)?;
    let mut g = Node::group(n.id, copies.into_iter().map(Arc::new).collect());
    g.name = n.name.clone();
    g.visible = n.visible;
    g.opacity = n.opacity;
    g.blend = n.blend;
    g.isolate = n.isolate;
    // One element: its pieces never knock each other out.
    g.knockout = Knockout::Off;
    g.knockout_shape = n.knockout_shape;
    g.extras_mut().mask = n.extras().mask.clone();
    g.appearance.effects = n.appearance.effects.iter().filter(|e| !is_geometry(&e.id)).cloned().collect();
    Some(g)
}

/// `n`, a group or layer without Pathfinder effects or fills and strokes that paint, whose geometry
/// effects only move its members ([`transform_moves`]), as plain art in which the type it holds
/// stays type: each type object moved, or replaced by a group of its copies ([`moved_type`]).
/// Type in a clipping path, other type that [`moved_type`] doesn't copy, and type inside live
/// objects or inside groups with Pathfinder or geometry effects or fills and strokes that paint, is
/// outlined as [`evaluate_container`] outlines it, and the other members become what it makes of
/// them. `None` when no type stays type.
fn container_keeping_type(d: &Document, n: &Node) -> Option<Node> {
    fn keep(n: &mut Node, moves: &[serde_json::Value], clipping: bool, kept: &mut bool) {
        let clips = n.clips();
        let Some(ch) = n.children_mut() else { return };
        for (i, c) in ch.iter_mut().enumerate() {
            let clipping = clipping || (clips && i == 0);
            if !matches!(c.kind, NodeKind::Text(_)) {
                if has_pathfinder(c) || has_own_paint(c) || !geometry_effects(c).is_empty() {
                    outline_nested_text(Arc::make_mut(c), clipping);
                } else if c.children().is_some() {
                    keep(Arc::make_mut(c), moves, clipping, kept);
                }
                continue;
            }
            // Type that isn't copied is outlined as `outline_nested_text` outlines it.
            let copies = if clipping { None } else { moved_type(c, moves) };
            let art = match copies {
                Some(mut copies) => {
                    *kept = true;
                    match copies.pop() {
                        Some(only) if copies.is_empty() => Some(only),
                        last => Some(Node::group(c.id, copies.into_iter().chain(last).map(Arc::new).collect())),
                    }
                }
                None if clipping => outline_text(c).map(|o| Node { visible: c.visible, ..o }),
                None => as_art(None, c, None),
            };
            if let Some(o) = art {
                *c = Arc::new(o);
            }
        }
    }
    if !matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }) || has_pathfinder(n) || has_own_paint(n) {
        return None;
    }
    let fx = geometry_effects(n);
    let moves = transform_moves(&fx)?;
    let mut text = false;
    n.walk(&mut |c| text |= matches!(c.kind, NodeKind::Text(_)));
    if !text {
        return None;
    }
    let mut m = crate::reshape::evaluated(Some(d), n);
    let mut kept = false;
    keep(&mut m, &moves, false, &mut kept);
    if !kept {
        return None;
    }
    reshape_leaves(&mut m, &fx);
    m.appearance.effects.retain(|e| !is_pathfinder(&e.id) && !is_geometry(&e.id));
    m.appearance.items.clear();
    m.appearance.contents_index = None;
    Some(m)
}

/// A new path of `n`'s (its geometry `path`, filled under `rule`) painted by `item` alone, with
/// none of `n`'s transparency.
fn piece(d: &mut Document, n: &Node, path: PathData, rule: FillRule, item: AppearanceItem) -> Node {
    let id = d.alloc_id();
    let kind = path_kind(d, path, rule);
    let mut out = n.clone();
    out.id = id;
    out.name = None;
    out.appearance = Appearance { items: vec![item], ..Default::default() }.into();
    out.opacity = 1.0;
    out.blend = Default::default();
    out.isolate = false;
    out.knockout = Default::default();
    out.knockout_shape = false;
    out.kind = kind;
    let mut extras = out.extras_mut();
    extras.mask = None;
    extras.trace = None;
    extras.wrap = None;
    extras.graph = None;
    // The group keeps the URL (one link around the pieces).
    extras.attrs = None;
    drop(extras);
    out
}

/// `n`'s geometry with its object-level geometry effects applied, its fill rule, the context of
/// its effects, and `n` without those effects.
fn leaf_base(n: &Node) -> Option<(PathData, FillRule, GeomContext<'_>, Node)> {
    let (base, rule) = geometry(n)?;
    let ctx = GeomContext::of(n);
    let g = apply(&n.appearance.effects, &base, &ctx);
    let mut m = n.clone();
    m.appearance.effects.retain(|e| !is_geometry(&e.id));
    Some((g, rule, ctx, m))
}

fn bake_leaf(d: &mut Document, n: &Node) -> Option<Node> {
    let (g, rule, ctx, mut m) = leaf_base(n)?;
    if !n.appearance.items.iter().any(|i| has_geometry(item_effects(i))) {
        m.kind = path_kind(d, g, rule);
        return Some(m);
    }
    // Per-item geometry: one path per fill/stroke, grouped under the object's transparency and
    // raster effects.
    let children = n
        .appearance
        .items
        .iter()
        .map(|item| {
            let ig = apply(item_effects(item), &g, &ctx.item(item));
            let mut it = item.clone();
            clear_item_effects(&mut it);
            Arc::new(piece(d, n, ig, rule, it))
        })
        .collect();
    m.appearance.items.clear();
    m.kind = NodeKind::Group { children, clip: false };
    Some(m)
}

/// Turns one stroke of a path (the path after its geometry effects, its fill rule, the stroke
/// without geometry effects) into filled art for [`expand_leaf`]: its outline filled with its
/// paint, or its brush art. `None` when it paints nothing.
pub type StrokeArt<'a> = &'a mut dyn FnMut(&mut Document, &PathData, FillRule, &StrokeLayer) -> Option<Node>;

/// Object → Expand Appearance of a path or compound path `n`: its geometry effects applied, every
/// visible fill becomes a copy of the path painted by that fill alone and every visible stroke the
/// filled art `stroke_art` makes of it (each with its fill's or stroke's opacity and blend mode as
/// its own), in paint order, as the members of a group that keeps
/// `n`'s id, name, transparency, opacity mask and remaining (raster) effects. A single piece
/// without transparency or effects of its own takes `n`'s place itself. Hidden fills, strokes and
/// effects are dropped. `None` for other kinds of objects.
pub fn expand_leaf(d: &mut Document, n: &Node, stroke_art: StrokeArt) -> Option<Node> {
    if let Some(m) = crate::revolve::expand_revolve_art(n).or_else(|| crate::solid_art(n)) {
        let m = evaluate_container(Some(d), &m).unwrap_or(m);
        return Some(bake_pieces(d, m));
    }
    let (g, rule, ctx, mut m) = leaf_base(n)?;
    m.appearance.effects.retain(|e| e.visible);
    let mut pieces = vec![];
    for item in n.appearance.items.iter().filter(|i| paints(i)) {
        let ig = apply(item.effects(), &g, &ctx.item(item));
        let mut it = item.clone();
        clear_item_effects(&mut it);
        it.effects_mut().retain(|e| e.visible);
        let p = match &it {
            AppearanceItem::Stroke(st) => stroke_art(d, &ig, rule, st),
            AppearanceItem::Fill(_) => Some(piece(d, n, ig, rule, it)),
        };
        pieces.extend(p.map(hoist_transparency));
    }
    m.appearance.items.clear();
    m.appearance.contents_index = None;
    match pieces.as_slice() {
        [one] if m.appearance.effects.is_empty() && one.has_default_transparency() && one.appearance.effects.is_empty() => {
            m.kind = one.kind.clone();
            m.appearance = one.appearance.clone();
        }
        _ => m.kind = NodeKind::Group { children: pieces.into_iter().map(Arc::new).collect(), clip: false },
    }
    Some(m)
}

/// `p` with the opacity and blend mode of its only fill or stroke as its own (the same look, and a
/// basic appearance).
fn hoist_transparency(mut p: Node) -> Node {
    if p.has_default_transparency()
        && let [item] = p.appearance.items.as_mut_slice()
    {
        let (opacity, blend) = match item {
            AppearanceItem::Fill(f) => (&mut f.opacity, &mut f.blend),
            AppearanceItem::Stroke(s) => (&mut s.opacity, &mut s.blend),
        };
        p.opacity = std::mem::replace(opacity, 1.0);
        p.blend = std::mem::take(blend);
    }
    p
}

/// [`expand_leaf`] every path and compound path in `n` (art made for Expand Appearance, such as a
/// group's own fills and strokes or outlined type), in place.
pub fn expand_art(d: &mut Document, n: &mut Node, stroke_art: StrokeArt) {
    if matches!(n.kind, NodeKind::Path { clipping: false, guide: false, .. } | NodeKind::Compound { .. }) {
        if let Some(m) = expand_leaf(d, n, stroke_art) {
            *n = m;
        }
        return;
    }
    for c in n.children_mut().into_iter().flatten() {
        expand_art(d, Arc::make_mut(c), stroke_art);
    }
}

/// Envelope Options → Distort Appearance: path or compound path `n` as plain filled art that an
/// envelope bends as a whole: its geometry effects applied, every visible fill a copy of the path
/// painted by that fill and every visible stroke without a brush its filled outline in the
/// stroke's paint, opacity, blend mode and raster effects (a gradient along or across the stroke:
/// its gradient meshes clipped to the outline; a brushed stroke stays a stroke), in paint order, as
/// the members of a group keeping `n`'s id, transparency, opacity mask and raster effects. `None`
/// when it has neither a stroke to outline nor a geometry effect, so bending first changes nothing.
pub fn bake_appearance(n: &Node) -> Option<Node> {
    if let Some(m) = crate::revolve_art(n).or_else(|| crate::solid_art(n)) {
        return Some(m);
    }
    if !matches!(n.kind, NodeKind::Path { clipping: false, guide: false, .. } | NodeKind::Compound { .. }) {
        return None;
    }
    let outlined = |i: &AppearanceItem| paints(i) && matches!(i, AppearanceItem::Stroke(s) if s.brush.is_none());
    let geometric = has_geometry(&n.appearance.effects) || n.appearance.items.iter().any(|i| has_geometry(item_effects(i)));
    if !geometric && !n.appearance.items.iter().any(outlined) {
        return None;
    }
    let (g, rule, ctx, mut m) = leaf_base(n)?;
    let piece = |path: PathData, rule: FillRule, item: AppearanceItem| {
        let mut p = Node::path(n.id, path, Appearance { items: vec![item], ..Default::default() });
        if let NodeKind::Path { rule: r, .. } = &mut p.kind {
            *r = rule;
        }
        Arc::new(p)
    };
    let mut pieces = vec![];
    for item in n.appearance.items.iter().filter(|i| paints(i)) {
        let ig = apply(item_effects(item), &g, &ctx.item(item));
        let mut it = item.clone();
        clear_item_effects(&mut it);
        pieces.push(match &it {
            AppearanceItem::Stroke(st) if st.brush.is_none() => {
                let outline = crate::stroke::outline_region(&ig, rule, st);
                match st.path_gradient() {
                    // A gradient along or across the stroke: its gradient meshes clipped to the outline.
                    Some(g) => {
                        let mut clip = Node::path(n.id, outline, Appearance::default());
                        if let NodeKind::Path { clipping, .. } = &mut clip.kind {
                            *clipping = true;
                        }
                        let meshes = crate::stroke::gradient_meshes(&ig.to_bezpath(), rule, st, &g.gradient);
                        let meshes = meshes.into_iter().map(|m| Node::new(n.id, NodeKind::Mesh(m)));
                        let children = std::iter::once(clip).chain(meshes).map(Arc::new).collect();
                        let mut group = Node::new(n.id, NodeKind::Group { children, clip: true });
                        (group.opacity, group.blend) = (st.opacity, st.blend);
                        group.appearance.effects = st.effects.clone();
                        Arc::new(group)
                    }
                    None => {
                        let effects = st.effects.clone();
                        let fill = FillLayer { opacity: st.opacity, blend: st.blend, effects, ..FillLayer::new(st.paint.stroke_as_fill()) };
                        piece(outline, FillRule::NonZero, AppearanceItem::Fill(fill))
                    }
                }
            }
            _ => piece(ig, rule, it),
        });
    }
    m.appearance.items.clear();
    m.appearance.contents_index = None;
    m.kind = NodeKind::Group { children: pieces, clip: false };
    Some(m)
}

/// A copy of `doc` ready to export: every live geometry effect baked into plain paths, and the
/// resources of its placed documents' art added ([`Document::with_placed_art`]); `None` when
/// there is nothing to do (export it as is).
pub fn bake_document(doc: &Document) -> Option<Document> {
    let placed = doc.with_placed_art();
    if !placed.layers.iter().any(|l| needs_bake(l)) {
        return match placed {
            std::borrow::Cow::Owned(d) => Some(d),
            std::borrow::Cow::Borrowed(_) => None,
        };
    }
    let mut d = placed.into_owned();
    let layers = d.layers.clone();
    d.layers = layers.iter().map(|l| bake_node(&mut d, l).map(Arc::new).unwrap_or_else(|| l.clone())).collect();
    Some(d)
}

#[cfg(test)]
mod tests;
