use super::predicates::{has_fx, has_object_fx};
use super::raster::doc_raster_ppi;
use super::*;

fn node_bezpath(n: &Node) -> Option<BezPath> {
    match &n.kind {
        NodeKind::Path { path, .. } => Some(path.to_bezpath()),
        NodeKind::Compound { children, .. } => {
            let mut bp = BezPath::new();
            for c in children {
                if let Some(p) = c.path_data() {
                    bp.extend(p.to_bezpath());
                }
            }
            Some(bp)
        }
        _ => None,
    }
}

pub(super) fn apply(effects: &[Effect], bp: &BezPath, ctx: &GeomContext) -> BezPath {
    if !effects::has_geometry(effects) || bp.elements().is_empty() {
        return bp.clone();
    }
    effects::apply_geometry_bez(effects, bp, bp.bounding_box(), ctx)
}

/// The object-level effected geometry of `n` (base path `bp`).
pub(crate) fn effected_path(n: &Node, bp: &BezPath) -> BezPath {
    apply(&n.appearance.effects, bp, &GeomContext::of(n))
}

pub(super) fn item_effects(item: &AppearanceItem) -> &[Effect] {
    match item {
        AppearanceItem::Fill(f) => &f.effects,
        AppearanceItem::Stroke(s) => &s.effects,
    }
}

/// Visual bounds including geometry effects, stroke outsets and shadows/glows (symbol instances
/// measured by their art from `symbols`).
pub(crate) fn visual_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    if let Some(art) = effects::revolve_art(n).or_else(|| effects::solid_art(n)) {
        return cull_bounds(symbols, &art).map(|b| {
            let o = outset(symbols, &n.appearance.effects, b);
            b.inflate(o, o)
        });
    }
    let bp = node_bezpath(n)?;
    let ctx = GeomContext::of(n);
    let g = effected_path(n, &bp);
    let mut r: Option<Rect> = (!g.elements().is_empty()).then(|| g.bounding_box());
    for item in &n.appearance.items {
        let fx = item_effects(item);
        if effects::has_geometry(fx) {
            let ig = apply(fx, &g, &ctx.item(item));
            if !ig.elements().is_empty() {
                let b = ig.bounding_box();
                r = Some(r.map_or(b, |r| r.union(b)));
            }
        }
    }
    let s = n.appearance.outset();
    r.map(|r| {
        let r = r.inflate(s, s);
        // The fills' and strokes' own raster effects, then the object's.
        let r = n.appearance.items.iter().filter(|i| paints(i)).map(|i| spread(symbols, item_effects(i), r)).fold(r, |acc, b| acc.union(b));
        let o = outset(symbols, &n.appearance.effects, r);
        r.inflate(o, o)
    })
}

/// Does `item` paint (as [`Renderer::paint_items`] draws it)?
fn paints(item: &AppearanceItem) -> bool {
    item.visible() && !item.paint().is_none()
}

/// The bounds of what `n` draws, its own and its members' geometry effects applied and the raster
/// effects of its members and of its fills and strokes included (each fill's and stroke's
/// spreading what it paints, [`item_bounds`]), with strokes measured as
/// [`Node::visual_bounds_with`] measures them, without the reach of `n`'s own raster effects
/// (symbol instances measured by their art from `symbols`): what Rasterize makes its image over.
pub(crate) fn drawn_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    let b = art_bounds(symbols, n)?;
    let items = n.appearance.items.iter().filter(|i| paints(i) && !effects::raster_effects(item_effects(i)).is_empty());
    Some(items.map(|i| spread(symbols, item_effects(i), item_bounds(n, i).unwrap_or(b))).fold(b, |acc, r| acc.union(r)))
}

/// The bounds of what `item`, a fill or stroke of the path or compound path `n`, paints: the path
/// with `n`'s and `item`'s geometry effects, and for a stroke its width. `None` for other objects,
/// 3D art, a brushed stroke and an item that draws no path.
fn item_bounds(n: &Node, item: &AppearanceItem) -> Option<Rect> {
    let stroke = matches!(item, AppearanceItem::Stroke(_));
    if !has_fx(n) || effects::has_revolve(n) || effects::has_solid(n) || (stroke && vectorcraft_brush::has_brush(n)) {
        return None;
    }
    let g = effected_path(n, &node_bezpath(n)?);
    let fx = item_effects(item);
    if effects::has_geometry(fx) {
        let ig = apply(fx, &g, &GeomContext::of(n).item(item));
        let o = if let AppearanceItem::Stroke(s) = item { s.reach() } else { 0.0 };
        return (!ig.elements().is_empty()).then(|| ig.bounding_box().inflate(o, o));
    }
    let one = vectorcraft_doc::Appearance { items: vec![item.clone()], ..Default::default() };
    (!g.elements().is_empty()).then(|| one.stroked_bounds(g.bounding_box(), || g.clone()))
}

/// `b` grown by how far raster effects `fx` paint past it, each spreading what the ones before it
/// paint (Rasterize at the document's raster effects resolution where it takes it).
fn spread(symbols: &SymbolBoxes, fx: &[Effect], b: Rect) -> Rect {
    let ppi = doc_raster_ppi(symbols.document());
    effects::raster_effects(fx).iter().fold(b, |r, e| e.at_document_ppi(ppi).spread(r, 72.0 / ppi))
}

/// [`drawn_bounds`] before the raster effects of `n`'s fills and strokes: its art, and the art of
/// its brushed strokes (a symbol's brushed art as far as [`crate::brush_fx::pad`] reaches).
fn art_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    let b = shape_bounds(symbols, n);
    let brushed = vectorcraft_brush::has_brush(n)
        .then(|| vectorcraft_brush::expand(symbols.document(), n))
        .flatten()
        .and_then(|art| art.visual_bounds_with(symbols));
    let b = vectorcraft_geom::union_opt(b, brushed)?;
    let pad = if matches!(n.kind, NodeKind::SymbolInstance { .. }) { crate::brush_fx::pad(symbols, n) } else { 0.0 };
    Some(b.inflate(pad, pad))
}

/// [`art_bounds`] without brush art.
fn shape_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    // The visible members of `g` (a Shaper group's first member is its source art, which paints
    // nothing).
    let union = |g: &Node| {
        let members = g.children()?.get(usize::from(g.extras().shaper.is_some())..).unwrap_or_default();
        members.iter().filter(|c| c.visible).fold(None, |acc, c| vectorcraft_geom::union_opt(acc, painted_reach(symbols, c)))
    };
    match &n.kind {
        _ if effects::has_container_appearance(n) => {
            let art = effects::evaluate_container(Some(symbols.document()), n);
            let art = art.as_ref().unwrap_or(n);
            if art.clips() { art.visual_bounds_with(symbols) } else { union(art) }
        }
        NodeKind::Layer { clip: false, .. } | NodeKind::Group { clip: false, .. } => union(n),
        _ if has_fx(n) => {
            // A 3D effect's art.
            if let Some(art) = effects::revolve_art(n).or_else(|| effects::solid_art(n)) {
                return drawn_bounds(symbols, &art);
            }
            let g = effected_path(n, &node_bezpath(n)?);
            // The path as the fills and strokes without geometry effects of their own paint it
            // (as every fill and stroke would when none paints).
            let mut plain = n.appearance.clone();
            plain.items.retain(|i| !(paints(i) && effects::has_geometry(item_effects(i))));
            let on_path = !n.appearance.items.iter().any(paints) || plain.items.iter().any(paints);
            let mut r = (on_path && !g.elements().is_empty()).then(|| plain.stroked_bounds(g.bounding_box(), || g.clone()));
            let ctx = GeomContext::of(n);
            for item in n.appearance.items.iter().filter(|i| paints(i) && effects::has_geometry(item_effects(i))) {
                let ig = apply(item_effects(item), &g, &ctx.item(item));
                let o = if let AppearanceItem::Stroke(s) = item { s.reach() } else { 0.0 };
                r = vectorcraft_geom::union_opt(r, (!ig.elements().is_empty()).then(|| ig.bounding_box().inflate(o, o)));
            }
            r
        }
        // As [`cull_bounds_in`] measures them, without the object's own raster effects: the
        // reshaped art (type's outlines, and an image and the paths of its own fills and strokes,
        // measured one by one with the effects of those fills and strokes), grown by how far the
        // effects in a symbol's art and a live object's content paint past it.
        _ if has_object_fx(n) => {
            let symbol = match &n.kind {
                NodeKind::SymbolInstance { symbol, .. } => symbols.document().symbols.iter().find(|s| s.name == *symbol).map(|s| s.art.clone()),
                _ => None,
            };
            let reshaped = effects::reshape(Some(symbols.document()), n, symbol.as_deref());
            let b = match &reshaped {
                Some(r) if matches!(n.kind, NodeKind::Text(_)) || (matches!(n.kind, NodeKind::Image(_)) && !r.clips()) => {
                    r.children().into_iter().flatten().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, painted_reach(symbols, c)))
                }
                Some(r) => r.visual_bounds_with(symbols),
                None => None,
            }
            .or_else(|| n.visual_bounds_with(symbols))?;
            let a = match &reshaped {
                Some(r) => reshaped_art_effects_reach(symbols, n, r),
                None => art_effects_reach(symbols, n),
            };
            let a = a.max(live_content_reach(symbols, n));
            Some(b.inflate(a, a))
        }
        _ => {
            let b = n.visual_bounds_with(symbols)?;
            let a = art_effects_reach(symbols, n).max(live_content_reach(symbols, n));
            Some(b.inflate(a, a))
        }
    }
}

/// [`drawn_bounds`] of `n` grown by how far its own raster effects paint past them. With Crop
/// Marks, the [`drawn_bounds`] of its art: the marks and the object, which paints its own raster
/// effects inside the art.
pub(crate) fn painted_reach(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    if let Some(art) = effects::crop_marks_art(n) {
        return drawn_bounds(symbols, &art);
    }
    drawn_bounds(symbols, n).map(|b| spread(symbols, &n.appearance.effects, b))
}

/// The outline the renderer clips `n`'s Feather, Inner Glow and Rasterize with Create Clipping
/// Mask to: a path's or compound path's geometry with its object-level geometry effects applied,
/// the outline of the art [`Renderer::fx_art`] makes for an object with object effects, and with
/// Crop Marks the object's own. For other objects it is [`effects::clip_outline`].
pub(crate) fn drawn_outline(doc: &Document, n: &Node) -> Option<(BezPath, FillRule)> {
    if let Some(art) = effects::crop_marks_art(n) {
        return drawn_outline(doc, art.children()?.first()?);
    }
    if has_object_fx(n) {
        let art = Renderer::new().fx_art(doc, &Arc::new(n.clone()), false);
        return effects::clip_outline(&art);
    }
    match &n.kind {
        NodeKind::Path { rule, .. } | NodeKind::Compound { rule, .. } if effects::has_geometry(&n.appearance.effects) => {
            Some((effected_path(n, &node_bezpath(n)?), *rule))
        }
        _ => effects::clip_outline(n),
    }
}

/// How far raster effects `fx` paint beyond `b`, each spreading what the ones before it paint
/// ([`spread`]).
fn outset(symbols: &SymbolBoxes, fx: &[Effect], b: Rect) -> f64 {
    let s = spread(symbols, fx, b);
    [b.x0 - s.x0, b.y0 - s.y0, s.x1 - b.x1, s.y1 - b.y1].into_iter().fold(0.0, f64::max)
}

/// Bounds used for culling: like `Node::visual_bounds_with`, but aware of live effects anywhere in
/// the subtree (a shadow can reach the view while its object is outside it). Symbol instances are
/// measured by their art from `symbols`.
pub(crate) fn cull_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    cull_bounds_in(symbols, n, true)
}

/// [`cull_bounds`] of `n`. With `after_reshape` true, the effects in the art of a symbol instance
/// with geometry effects of its own are measured on the art as those effects reshape it
/// ([`reshaped_art_effects_reach`]). With `after_reshape` false, they are measured on the art
/// before those effects run ([`art_effects_reach`]), which `symbols` keeps for each symbol and
/// linear part.
fn cull_bounds_in(symbols: &SymbolBoxes, n: &Node, after_reshape: bool) -> Option<Rect> {
    let inner = |c: &Node| cull_bounds_in(symbols, c, after_reshape);
    match &n.kind {
        // The object and its crop marks.
        _ if effects::has_crop_marks(n) => effects::crop_marks_art(n).and_then(|art| inner(&art)),
        // The evaluated art (its fills, strokes and geometry effects) and the raster effects' reach.
        _ if effects::has_container_appearance(n) => {
            let art = effects::evaluate_container(Some(symbols.document()), n);
            let art = art.as_ref().unwrap_or(n);
            let b = if art.clips() {
                art.visual_bounds_with(symbols)
            } else {
                art.children()?.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, inner(c)))
            }?;
            let o = outset(symbols, &n.appearance.effects, b);
            Some(b.inflate(o, o))
        }
        NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => {
            children.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, inner(c)))
        }
        _ if has_fx(n) => visual_bounds(symbols, n),
        // The reshaped art (a symbol instance's reshaped art is made from its symbol's art; type's
        // outlines, and an image and the paths of its own fills and strokes, measured one by one
        // with the geometry effects of those fills and strokes), grown by how far the effects in a
        // symbol instance's art paint past that art.
        _ if has_object_fx(n) => {
            let symbol = match &n.kind {
                NodeKind::SymbolInstance { symbol, .. } => symbols.document().symbols.iter().find(|s| s.name == *symbol).map(|s| s.art.clone()),
                _ => None,
            };
            let reshaped = effects::reshape(Some(symbols.document()), n, symbol.as_deref());
            let b = match &reshaped {
                Some(r) if matches!(n.kind, NodeKind::Text(_)) || (matches!(n.kind, NodeKind::Image(_)) && !r.clips()) => {
                    r.children().into_iter().flatten().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, inner(c)))
                }
                Some(r) => r.visual_bounds_with(symbols),
                None => None,
            }
            .or_else(|| n.visual_bounds_with(symbols))?;
            let a = match &reshaped {
                Some(r) if after_reshape => reshaped_art_effects_reach(symbols, n, r),
                _ => art_effects_reach(symbols, n),
            };
            let a = a.max(live_content_reach(symbols, n));
            let b = b.inflate(a, a);
            let o = outset(symbols, &n.appearance.effects, b);
            Some(b.inflate(o, o))
        }
        NodeKind::SymbolInstance { .. } => {
            let b = n.visual_bounds_with(symbols)?;
            let a = art_effects_reach(symbols, n);
            Some(b.inflate(a, a))
        }
        _ => {
            let b = n.visual_bounds_with(symbols)?;
            let b = match &n.kind {
                NodeKind::Text(t) => vectorcraft_geom::union_opt(Some(b), glyphs_box(t, n.appearance.outset())).unwrap_or(b),
                _ => b,
            };
            let a = live_content_reach(symbols, n);
            Some(b.inflate(a, a))
        }
    }
}

/// The box of type `t`'s glyphs as laid out (its layout bounds, else laid out now), grown by its
/// characters' strokes, miter spikes included, and by `outset`: on a path they stand above it and
/// run past its ends, in an area they can hang past its frame, where its own box
/// ([`Node::visual_bounds_with`]) doesn't.
fn glyphs_box(t: &vectorcraft_doc::TextObject, outset: f64) -> Option<Rect> {
    let b = t.cached_bounds.unwrap_or_else(|| vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).bounds);
    let o = outset.max(t.char_stroke_reach());
    (b.width() > 0.0 || b.height() > 0.0).then(|| t.xf.transform_rect_bbox(b).inflate(o, o))
}

/// How far the content of live object `n` (a blend, envelope or repeat) can paint past its visual
/// bounds: for each object in it, how far the effects in a symbol instance's art paint past that
/// art ([`art_effects_reach`]) and how far brush art can paint ([`crate::brush_fx::pad`]), on the
/// farthest side. 0 for any other kind of object.
fn live_content_reach(symbols: &SymbolBoxes, n: &Node) -> f64 {
    if !vectorcraft_doc::live::is_live(n) {
        return 0.0;
    }
    let mut reach = 0.0f64;
    n.walk(&mut |c| reach = reach.max(art_effects_reach(symbols, c) + crate::brush_fx::pad(symbols, c)));
    reach
}

/// How far the effects in the art of symbol instance `n` (and in the art of the symbols the
/// instances in it show) paint past that art's visual bounds, on its farthest side, measured on the
/// art as the instance places it before its effects run: 0 when the art has no visible effects, and
/// for any other kind of object. The reach is the same wherever the art is moved to, so the art is
/// placed by the linear part of the instance's transform alone, and `symbols` keeps the reach for
/// each symbol and linear part, up to a limit ([`SymbolBoxes::effects_reach`]).
fn art_effects_reach(symbols: &SymbolBoxes, n: &Node) -> f64 {
    let NodeKind::SymbolInstance { symbol, xf } = &n.kind else { return 0.0 };
    if !symbols.has_effects(symbol) {
        return 0.0;
    }
    let measure = |[m0, m1, m2, m3]: [f64; 4]| {
        let past = |art: &Arc<Node>| {
            let mut placed = (**art).clone();
            placed.transform(Affine::new([m0, m1, m2, m3, 0.0, 0.0]), false);
            match (cull_bounds(symbols, &placed), placed.visual_bounds_with(symbols)) {
                (Some(c), Some(v)) => [v.x0 - c.x0, v.y0 - c.y0, c.x1 - v.x1, c.y1 - v.y1].into_iter().fold(0.0, f64::max),
                _ => 0.0,
            }
        };
        symbols.measure_art(symbol, past).unwrap_or(0.0)
    };
    let [m0, m1, m2, m3, _, _] = xf.as_coeffs();
    symbols.effects_reach(symbol, [m0, m1, m2, m3], &measure)
}

/// How far the effects in the art of symbol instance `n` paint past `r`, that art as `n`'s own
/// geometry effects reshape it ([`effects::reshape`]), on its farthest side: 0 when the art has no
/// visible effects, and for any other kind of object. The art in `r` is measured without the raster
/// effects of `n` that `r` carries, by [`cull_bounds_in`] with `after_reshape` false: the effects in
/// the art of an instance in it are measured before that instance's own geometry effects run.
fn reshaped_art_effects_reach(symbols: &SymbolBoxes, n: &Node, r: &Node) -> f64 {
    let NodeKind::SymbolInstance { symbol, .. } = &n.kind else { return 0.0 };
    if !symbols.has_effects(symbol) {
        return 0.0;
    }
    let art = r.children().into_iter().flatten().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, cull_bounds_in(symbols, c, false)));
    match (art, r.visual_bounds_with(symbols)) {
        (Some(c), Some(v)) => [v.x0 - c.x0, v.y0 - c.y0, c.x1 - v.x1, c.y1 - v.y1].into_iter().fold(0.0, f64::max),
        _ => 0.0,
    }
}
