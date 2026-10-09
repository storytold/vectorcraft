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
//! Object → Expand Appearance goes further ([`expand_leaf`]): every fill and stroke becomes an
//! object of its own, strokes turned into filled art by the caller (their outlines or brush art).

use std::sync::Arc;

use vectorcraft_doc::{Appearance, AppearanceItem, Document, FillLayer, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{FillRule, PathData};

use crate::group::{has_own_paint, paints};
use crate::{
    GeomContext, adjust_in_document, apply_geometry_with, crop_marks_art, evaluate_container, has_adjustment, has_crop_marks, has_geometry,
    has_pathfinder, is_geometry, needs_outline, reshape,
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
    matches!(n.kind, NodeKind::CompoundShape { .. })
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
    if !needs_bake(n) {
        return None;
    }
    // Colour adjustments: the object recoloured (inside it too), then baked as it is.
    if has_adjustment(n)
        && let Some(m) = adjust_in_document(d, n)
    {
        return Some(bake_node(d, &m).unwrap_or(m));
    }
    // Compound shapes: their evaluated path, with their transparency and opacity mask (its
    // subpaths become a compound path with ids of their own).
    if matches!(n.kind, NodeKind::CompoundShape { .. })
        && let Some(m) = crate::evaluate_compound_shape(n)
    {
        let mut m = crate::carry_transparency(n, m);
        if let NodeKind::Path { path, rule, .. } = &m.kind {
            m.kind = path_kind(d, path.clone(), *rule);
        }
        return Some(bake_node(d, &m).unwrap_or(m));
    }
    // Crop marks: the object and its marks.
    if let Some(m) = crop_marks_art(n) {
        return Some(bake_pieces(d, m));
    }
    // Type, images, symbol instances and live objects: reshaped through their outlines.
    if needs_outline(n) && (has_geometry(&n.appearance.effects) || n.projection().is_some()) {
        let symbol = match &n.kind {
            NodeKind::SymbolInstance { symbol, .. } => d.symbols.iter().find(|s| s.name == *symbol).map(|s| s.art.clone()),
            _ => None,
        };
        let m = reshape(n, symbol.as_deref())?;
        return Some(bake_node(d, &m).unwrap_or(m));
    }
    // Groups and layers: Pathfinder, geometry effects and their own fills and strokes. The new
    // pieces (results, outlines, the fills' and strokes' art) get ids of their own.
    if let Some(m) = evaluate_container(n) {
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

/// `m`, art evaluated from an object, with its children baked and given ids of their own (the
/// pieces copied from the source keep the source's ids until then).
fn bake_pieces(d: &mut Document, mut m: Node) -> Node {
    let mut seen = std::collections::HashSet::from([m.id]);
    let children = m.children().cloned().unwrap_or_default();
    let children = children
        .into_iter()
        .map(|c| {
            let mut c = Arc::unwrap_or_clone(c);
            fresh_ids(d, &mut c, &mut seen);
            Arc::new(bake_node(d, &c).unwrap_or(c))
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

/// A new path of `n`'s (its geometry `path`, filled under `rule`) painted by `item` alone, with
/// none of `n`'s transparency.
fn piece(d: &mut Document, n: &Node, path: PathData, rule: FillRule, item: AppearanceItem) -> Node {
    let id = d.alloc_id();
    let kind = path_kind(d, path, rule);
    Node {
        id,
        name: None,
        appearance: Appearance { items: vec![item], ..Default::default() },
        opacity: 1.0,
        blend: Default::default(),
        isolate: false,
        knockout: Default::default(),
        knockout_shape: false,
        mask: None,
        trace: None,
        wrap: None,
        graph: None,
        // The group keeps the URL (one link around the pieces).
        attrs: None,
        kind,
        ..n.clone()
    }
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
/// stroke's paint, opacity and blend mode (a brushed stroke stays a stroke), in paint order, as
/// the members of a group keeping `n`'s id, transparency, opacity mask and raster effects. `None`
/// when it has neither a stroke to outline nor a geometry effect, so bending first changes nothing.
pub fn bake_appearance(n: &Node) -> Option<Node> {
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
                let fill = FillLayer { opacity: st.opacity, blend: st.blend, ..FillLayer::new(st.paint.clone()) };
                piece(crate::stroke::outline_region(&ig, rule, st), FillRule::NonZero, AppearanceItem::Fill(fill))
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
mod tests {
    use super::*;
    use serde_json::json;
    use vectorcraft_doc::{Effect, NodeId};
    use vectorcraft_geom::{Rect, shapes};

    fn fx(id: &str, params: serde_json::Value) -> Effect {
        Effect { id: id.into(), params, visible: true }
    }

    fn doc_with(n: Node) -> Document {
        let mut d = Document::new(400.0, 400.0);
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        d
    }

    #[test]
    fn plain_documents_are_not_copied() {
        let mut d = Document::new(100.0, 100.0);
        let id = d.alloc_id();
        let d = doc_with(Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art()));
        assert!(bake_document(&d).is_none());
    }

    #[test]
    fn object_effects_become_geometry_and_raster_effects_stay() {
        let mut n = Node::path(NodeId(50), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default_art());
        n.appearance.effects.push(fx("path.offsetPath", json!({"offset": 10.0})));
        n.appearance.effects.push(fx("stylize.dropShadow", json!({})));
        let d = bake_document(&doc_with(n)).unwrap();
        let m = d.node(NodeId(50)).unwrap();
        let b = m.geometric_bounds().unwrap();
        assert!((b.width() - 120.0).abs() < 0.5, "{b:?}");
        assert_eq!(m.appearance.effects.len(), 1);
        assert_eq!(m.appearance.effects[0].id, "stylize.dropShadow");
    }

    #[test]
    fn item_effects_split_into_one_path_per_item() {
        let mut n = Node::path(NodeId(50), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default_art());
        if let AppearanceItem::Stroke(s) = &mut n.appearance.items[1] {
            s.effects.push(fx("path.offsetPath", json!({"offset": 5.0})));
        }
        let d = bake_document(&doc_with(n)).unwrap();
        let m = d.node(NodeId(50)).unwrap();
        let ch = m.children().unwrap();
        assert_eq!(ch.len(), 2);
        assert!((ch[0].geometric_bounds().unwrap().width() - 100.0).abs() < 1e-6);
        assert!((ch[1].geometric_bounds().unwrap().width() - 110.0).abs() < 0.5);
        assert!(ch[0].id != ch[1].id && ch[0].id != NodeId(50));
    }
}
