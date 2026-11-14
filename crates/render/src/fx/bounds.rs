use super::predicates::{has_fx, has_object_fx, is_container};
use super::raster::doc_raster_ppi;
use super::{
    Affine, AppearanceItem, Arc, BezPath, Document, Effect, FillRule, GeomContext, Node, NodeKind, PathEl, Rect, Renderer, Shape, SymbolBoxes,
    effects,
};
use vectorcraft_doc::symbol_overrides::dynamic_art;

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

/// The object-level effected geometry of `n` (base path `bp`), with the swatches and patterns that
/// live-effect plug-ins painting their pieces name looked up in `doc`.
pub(crate) fn effected_path(doc: &Document, n: &Node, bp: &BezPath) -> BezPath {
    apply(&n.appearance.effects, bp, &GeomContext { doc: Some(doc), ..GeomContext::of(n) })
}

/// [`apply`], and how gradients follow the effects ([`effects::Carry`]).
pub(super) fn apply_carrying(effects: &[Effect], bp: &BezPath, ctx: &GeomContext) -> (BezPath, effects::Carry) {
    if !effects::has_geometry(effects) || bp.elements().is_empty() {
        return (bp.clone(), effects::Carry::default());
    }
    effects::apply_geometry_bez_carrying(effects, bp, bp.bounding_box(), ctx)
}

/// [`effected_path`], and how `n`'s gradients follow its effects.
pub(crate) fn effected_path_carrying(doc: &Document, n: &Node, bp: &BezPath) -> (BezPath, effects::Carry) {
    apply_carrying(&n.appearance.effects, bp, &GeomContext { doc: Some(doc), ..GeomContext::of(n) })
}

pub(super) fn item_effects(item: &AppearanceItem) -> &[Effect] {
    match item {
        AppearanceItem::Fill(f) => &f.effects,
        AppearanceItem::Stroke(s) => &s.effects,
    }
}

/// The effects of `item`, a fill or stroke of `n`, that its raster effects are drawn from: each
/// one following `n`'s Transform effects as well ([`effects::item_effects_in`]).
pub(super) fn item_raster<'a>(n: &Node, item: &'a AppearanceItem) -> std::borrow::Cow<'a, [Effect]> {
    effects::item_effects_in(&n.appearance.effects, item_effects(item))
}

/// Visual bounds including geometry effects, stroke outsets and shadows/glows (symbol instances
/// measured by their art from `symbols`).
pub(crate) fn visual_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    if let Some(art) = painted_art(symbols, n) {
        return art.visual_bounds_with(symbols).map(|r| with_raster_reach(symbols, n, r));
    }
    let bp = node_bezpath(n)?;
    let ctx = GeomContext { doc: Some(symbols.document()), ..GeomContext::of(n) };
    let (g, carry) = effected_path_carrying(symbols.document(), n, &bp);
    // The raster effects reach as far as they are drawn: following only the Transform effects
    // that moved the art ([`effects::Carry::hide_skipped`]).
    let mut ran = std::borrow::Cow::Borrowed(n);
    if carry.skipped() {
        carry.hide_skipped(&mut ran.to_mut().appearance.effects);
    }
    let mut r: Option<Rect> = (!g.elements().is_empty()).then(|| g.bounding_box());
    for (j, item) in n.appearance.items.iter().enumerate() {
        let fx = item_effects(item);
        if effects::has_geometry(fx) {
            let (ig, c) = apply_carrying(fx, &g, &ctx.item(item));
            if c.skipped()
                && let Some(it) = ran.to_mut().appearance.items.get_mut(j)
            {
                c.hide_skipped(it.effects_mut());
            }
            if !ig.elements().is_empty() {
                let b = ig.bounding_box();
                r = Some(r.map_or(b, |r| r.union(b)));
            }
        }
    }
    let s = n.appearance.outset();
    r.map(|r| with_raster_reach(symbols, &ran, r.inflate(s, s)))
}

/// `n` with the Transform effects its geometry passes leave out (past the budget) hidden, in its
/// own list and in its fills' and strokes' ([`effects::Carry::hide_skipped`]): the effects its
/// raster effects are drawn from. Borrowed when it has no Transform effect or no raster effect, is
/// not a path or compound path, or its art is made by [`effects::reshape`] or a 3D effect.
fn ran<'a>(doc: &Document, n: &'a Node) -> std::borrow::Cow<'a, Node> {
    let transforms = |fx: &[Effect]| fx.iter().any(|e| e.visible && e.id == "distort.transform");
    let mut out = std::borrow::Cow::Borrowed(n);
    if (!transforms(&n.appearance.effects) && !n.appearance.items.iter().any(|i| transforms(item_effects(i))))
        || (effects::raster_effects(&n.appearance.effects).is_empty()
            && n.appearance.items.iter().all(|i| effects::raster_effects(item_effects(i)).is_empty()))
        || effects::has_painted_effects(n)
        || effects::has_revolve(n)
        || effects::has_solid(n)
    {
        return out;
    }
    let Some(bp) = node_bezpath(n) else { return out };
    let ctx = GeomContext { doc: Some(doc), ..GeomContext::of(n) };
    let (g, carry) = apply_carrying(&n.appearance.effects, &bp, &ctx);
    if carry.skipped() {
        carry.hide_skipped(&mut out.to_mut().appearance.effects);
    }
    for (j, item) in n.appearance.items.iter().enumerate() {
        let fx = item_effects(item);
        if !transforms(fx) || g.elements().is_empty() {
            continue;
        }
        let (_, c) = apply_carrying(fx, &g, &ctx.item(item));
        if c.skipped()
            && let Some(it) = out.to_mut().appearance.items.get_mut(j)
        {
            c.hide_skipped(it.effects_mut());
        }
    }
    out
}

/// `r`, the art of `n` with its strokes, grown by how far the raster effects of `n`'s fills and
/// strokes and then `n`'s own paint past it.
fn with_raster_reach(symbols: &SymbolBoxes, n: &Node, r: Rect) -> Rect {
    let r = n.appearance.items.iter().filter(|i| paints(i)).map(|i| spread(symbols, &item_raster(n, i), r)).fold(r, |acc, b| acc.union(b));
    let o = outset(symbols, &n.appearance.effects, r);
    r.inflate(o, o)
}

/// The art of `n` with each piece a live-effect plug-in returns and the paints of that piece
/// ([`effects::has_painted_effects`]); `None` for any other object.
fn painted_art(symbols: &SymbolBoxes, n: &Node) -> Option<Node> {
    effects::has_painted_effects(n).then(|| effects::reshape(Some(symbols.document()), n, None)).flatten()
}

/// Calls `f` on each object in `art`, the art [`painted_art`] makes, that has a visible brushed
/// stroke, looking into the visible members of its groups. Hidden objects are left out.
fn each_brushed_piece(art: &Node, f: &mut impl FnMut(&Node)) {
    if !art.visible {
        return;
    }
    match &art.kind {
        NodeKind::Group { children, .. } => children.iter().for_each(|c| each_brushed_piece(c, f)),
        _ if vectorcraft_brush::has_brush(art) => f(art),
        _ => {}
    }
}

/// [`cull_bounds`] of path or compound path `n` whose live-effect plug-ins paint their pieces
/// ([`painted_art`]), with room for brush art past each piece's brushed strokes
/// ([`crate::brush_fx::pad`] of that piece, from the piece's own stroke widths) and the reach of
/// `n`'s raster effects past them. `None` when the renderer measures `n` otherwise: it has no such
/// pieces, or has Crop Marks, Vector Halftone or a 3D effect.
pub(crate) fn painted_cull_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Option<Rect>> {
    if !effects::has_painted_effects(n)
        || effects::has_crop_marks(n)
        || effects::addons::halftone::has_halftone(n)
        || effects::has_revolve(n)
        || effects::has_solid(n)
    {
        return None;
    }
    let art = painted_art(symbols, n)?;
    let mut b = art.visual_bounds_with(symbols);
    each_brushed_piece(&art, &mut |p| {
        let pad = crate::brush_fx::pad(symbols, p);
        b = vectorcraft_geom::union_opt(b, p.visual_bounds_with(symbols).map(|r| r.inflate(pad, pad)));
    });
    Some(b.map(|b| with_raster_reach(symbols, n, b)))
}

/// The widest stroke of the pieces with a visible brushed stroke that live-effect plug-ins paint
/// ([`painted_art`]) for the paths in the art of the symbol named `symbol`, and in the art of the
/// symbols the instances in it show, each symbol measured once for `symbols`
/// ([`SymbolBoxes::measured_width`]). `None` when no such piece has one; symbols whose art has no
/// visible effect ([`SymbolBoxes::has_effects`]) are not looked at.
pub(crate) fn painted_brush_width(symbols: &SymbolBoxes, symbol: &str) -> Option<f64> {
    symbols.measured_width("painted brush", symbol, || {
        if !symbols.has_effects(symbol) {
            return None;
        }
        let mut width: Option<f64> = None;
        symbols.measure_art(symbol, |art| {
            art.walk(&mut |n| {
                let nested = match &n.kind {
                    NodeKind::SymbolInstance { symbol, .. } => painted_brush_width(symbols, symbol),
                    _ => None,
                };
                width = width.into_iter().chain(nested).reduce(f64::max);
                if let Some(pieces) = painted_art(symbols, n) {
                    each_brushed_piece(&pieces, &mut |p| {
                        let widths = p.appearance.items.iter().filter_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s.width) } else { None });
                        width = Some(widths.fold(width.unwrap_or(0.0), f64::max));
                    });
                }
            })
        });
        width
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
    drawn_bounds_ran(symbols, n, None)
}

/// [`drawn_bounds`], with `ran`, [`ran`] of `n`, when the caller has it.
fn drawn_bounds_ran(symbols: &SymbolBoxes, n: &Node, ran: Option<&Node>) -> Option<Rect> {
    // A 3D effect's art holds its fills' and strokes' raster effects, measured with it.
    if let Some(b) = three_d_bounds(symbols, n) {
        return b;
    }
    let b = art_bounds(symbols, n)?;
    let mut items =
        n.appearance.items.iter().enumerate().filter(|(_, i)| paints(i) && !effects::raster_effects(item_effects(i)).is_empty()).peekable();
    if items.peek().is_none() {
        return Some(b);
    }
    let own;
    let ran = match ran {
        Some(r) => r,
        None => {
            own = self::ran(symbols.document(), n);
            &*own
        }
    };
    Some(
        items
            .map(|(j, i)| {
                let r = ran.appearance.items.get(j).unwrap_or(i);
                spread(symbols, &item_raster(ran, r), item_bounds(symbols.document(), n, i).unwrap_or(b))
            })
            .fold(b, |acc, r| acc.union(r)),
    )
}

/// The bounds of what `item`, a fill or stroke of the path or compound path `n`, paints: the path
/// with `n`'s and `item`'s geometry effects, and for a stroke its width. `None` for other objects,
/// 3D art, a path whose live-effect plug-ins paint their pieces, a brushed stroke and an item that
/// draws no path.
fn item_bounds(doc: &Document, n: &Node, item: &AppearanceItem) -> Option<Rect> {
    let stroke = matches!(item, AppearanceItem::Stroke(_));
    if !has_fx(n)
        || effects::has_revolve(n)
        || effects::has_solid(n)
        || effects::has_painted_effects(n)
        || (stroke && vectorcraft_brush::has_brush(n))
    {
        return None;
    }
    let g = effected_path(doc, n, &node_bezpath(n)?);
    let fx = item_effects(item);
    if effects::has_geometry(fx) {
        let ig = apply(fx, &g, &GeomContext { doc: Some(doc), ..GeomContext::of(n) }.item(item));
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
/// its brushed strokes (a symbol's brushed art as far as [`crate::brush_fx::pad`] reaches). A path
/// whose live-effect plug-ins paint their pieces is measured by those pieces, each with the art
/// its brushed strokes draw along it.
fn art_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    if let Some(art) = painted_art(symbols, n) {
        let mut b = art.visual_bounds_with(symbols);
        each_brushed_piece(&art, &mut |p| {
            let brushed = vectorcraft_brush::expand(symbols.document(), p).and_then(|a| a.visual_bounds_with(symbols));
            b = vectorcraft_geom::union_opt(b, brushed);
        });
        return b;
    }
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
            let g = effected_path(symbols.document(), n, &node_bezpath(n)?);
            // The path as the fills and strokes without geometry effects of their own paint it
            // (as every fill and stroke would when none paints).
            let mut plain = n.appearance.clone();
            plain.items.retain(|i| !(paints(i) && effects::has_geometry(item_effects(i))));
            let on_path = !n.appearance.items.iter().any(paints) || plain.items.iter().any(paints);
            let mut r = (on_path && !g.elements().is_empty()).then(|| plain.stroked_bounds(g.bounding_box(), || g.clone()));
            let ctx = GeomContext { doc: Some(symbols.document()), ..GeomContext::of(n) };
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
                NodeKind::SymbolInstance { symbol, .. } => {
                    symbols.document().symbols.iter().find(|s| s.name == *symbol).map(|s| dynamic_art(&s.art, n))
                }
                _ => None,
            };
            let reshaped = effects::reshape(Some(symbols.document()), n, symbol.as_ref());
            let b = match &reshaped {
                Some(r) if has_instance_row_effects(n) => instance_piece_bounds(symbols, r),
                Some(r) if matches!(n.kind, NodeKind::Text(_)) || (matches!(n.kind, NodeKind::Image(_)) && !r.clips()) => {
                    r.children().into_iter().flatten().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, painted_reach(symbols, c)))
                }
                Some(r) => r.visual_bounds_with(symbols),
                None => None,
            }
            .or_else(|| own_box(symbols, n))?;
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

/// Is `n` a symbol instance whose active fill or stroke has geometry or raster effects that the
/// fills and strokes of its art take ([`effects::has_item_geometry`],
/// [`effects::has_instance_row_raster`])?
fn has_instance_row_effects(n: &Node) -> bool {
    matches!(n.kind, NodeKind::SymbolInstance { .. }) && (effects::has_item_geometry(n) || effects::has_instance_row_raster(n))
}

/// The bounds of `art`, the art [`effects::reshape`] made of a symbol instance whose active fill or
/// stroke has geometry or raster effects ([`has_instance_row_effects`]): its paths measured with the
/// effects of their fills and strokes, shadows and glows included ([`visual_bounds`]), the symbol instances in it that
/// took those effects by their own reshaped art in turn, its groups and layers with their own fills
/// and strokes by the art those paint ([`effects::evaluate_container`]), its clip groups by their
/// clipping paths and other objects as [`Node::visual_bounds_with`] measures them. Hidden objects are left out.
/// An instance more than [`vectorcraft_doc::depth::MAX_DEPTH`] instances deep, or whose art would
/// take the objects measured so far past [`effects::SYMBOL_ART_MAX_NODES`], is measured as
/// [`Node::visual_bounds_with`] measures it.
fn instance_piece_bounds(symbols: &SymbolBoxes, art: &Node) -> Option<Rect> {
    let mut budget = effects::SYMBOL_ART_MAX_NODES;
    pieces_within(symbols, art, 1, &mut budget)
}

/// [`instance_piece_bounds`] of `art`, `depth` instances deep, with `budget` objects left to measure.
fn pieces_within(symbols: &SymbolBoxes, art: &Node, depth: usize, budget: &mut u64) -> Option<Rect> {
    if !art.visible {
        return None;
    }
    match &art.kind {
        NodeKind::Group { clip: false, .. } | NodeKind::Layer { clip: false, .. } => {
            // A group's or layer's own fills and strokes as the art among its members that they paint.
            let evaluated = effects::has_container_appearance(art).then(|| effects::evaluate_container(Some(symbols.document()), art)).flatten();
            let g = evaluated.as_ref().unwrap_or(art);
            if g.clips() {
                return g.visual_bounds_with(symbols);
            }
            g.children().into_iter().flatten().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, pieces_within(symbols, c, depth, budget)))
        }
        NodeKind::Path { .. } | NodeKind::Compound { .. } if has_fx(art) => visual_bounds(symbols, art),
        NodeKind::SymbolInstance { symbol, .. } if depth < vectorcraft_doc::depth::MAX_DEPTH && has_instance_row_effects(art) => {
            let doc = symbols.document();
            let reshaped = doc.symbols.iter().find(|s| s.name == *symbol).and_then(|s| {
                let count = s.art.count() as u64;
                if count > *budget {
                    return None;
                }
                *budget -= count;
                effects::reshape(Some(doc), art, Some(&dynamic_art(&s.art, art)))
            });
            match reshaped {
                Some(r) => pieces_within(symbols, &r, depth + 1, budget),
                None => art.visual_bounds_with(symbols),
            }
        }
        _ => art.visual_bounds_with(symbols),
    }
}

/// [`drawn_bounds`] of `n` grown by how far its own raster effects paint past them. With Crop
/// Marks, the [`drawn_bounds`] of its art: the marks and the object, which paints its own raster
/// effects inside the art.
pub(crate) fn painted_reach(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    if let Some(art) = effects::crop_marks_art(n) {
        return drawn_bounds(symbols, &art);
    }
    let ran = ran(symbols.document(), n);
    let b = drawn_bounds_ran(symbols, n, Some(&ran))?;
    Some(spread(symbols, &ran.appearance.effects, b))
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
            Some((effected_path(doc, n, &node_bezpath(n)?), *rule))
        }
        _ => effects::clip_outline(n),
    }
}

/// Adds the outlines of `n` to `out`, one per object and without uniting them: the visible members of
/// a group, layer or intertwine (a Shaper group's without its source art) and the evaluated art of a
/// blend, envelope, repeat, Pathfinder result or object with object effects. A gradient mesh adds its
/// mesh lines and a symbol instance its bounds, as the selection highlight shows them; any other
/// object, a clip group included, adds [`drawn_outline`]. Each outline's points and each evaluated
/// object are taken from `left`; `None` when they do not fit in it.
pub(crate) fn drawn_outlines(doc: &Document, symbols: &SymbolBoxes, n: &Node, left: &mut usize, out: &mut Vec<BezPath>) -> Option<()> {
    if let Some(art) = effects::crop_marks_art(n) {
        return match art.children().and_then(|c| c.first()) {
            Some(object) => drawn_outlines(doc, symbols, object, left, out),
            None => Some(()),
        };
    }
    if has_object_fx(n) {
        let makes_art = effects::has_revolve(n) || effects::has_solid(n) || effects::addons::halftone::has_halftone(n);
        // Effects that leave a mesh's shape alone evaluate it to its many colored pieces.
        if let NodeKind::Mesh(m) = &n.kind
            && !makes_art
            && !effects::has_geometry(&n.appearance.effects)
        {
            return push_outline(Some(m.lines().to_bezpath()), left, out);
        }
        let mut object = n.clone();
        // A group's own fills and strokes repeat its members' geometry as more art.
        if is_container(n) && !makes_art && !effects::has_item_geometry(n) {
            object.appearance.items.clear();
        }
        let art = Renderer::new().fx_art(doc, &Arc::new(object), false);
        if art.children().is_some() && !has_object_fx(&art) {
            return evaluated_outlines(doc, symbols, &art, left, out);
        }
        return push_outline(effects::clip_outline(&art).map(|(path, _)| path), left, out);
    }
    if effects::has_pathfinder(n)
        && let Some(art) = effects::evaluate_container(Some(doc), n).filter(|art| !effects::has_pathfinder(art))
    {
        return evaluated_outlines(doc, symbols, &art, left, out);
    }
    match &n.kind {
        NodeKind::Group { children, clip: false } | NodeKind::Layer { children, clip: false, .. } | NodeKind::Intertwine { children } => {
            for c in children.iter().skip(usize::from(n.extras().shaper.is_some())).filter(|c| c.visible) {
                drawn_outlines(doc, symbols, c, left, out)?;
            }
            Some(())
        }
        NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Repeat(_) => {
            evaluated_outlines(doc, symbols, &effects::expanded_live_group(Some(doc), n), left, out)
        }
        NodeKind::Mesh(m) => push_outline(Some(m.lines().to_bezpath()), left, out),
        NodeKind::SymbolInstance { .. } => {
            push_outline(n.geometric_bounds_with(symbols).map(|b| vectorcraft_geom::shapes::rectangle(b).to_bezpath()), left, out)
        }
        _ => push_outline(drawn_outline(doc, n).map(|(path, _)| path), left, out),
    }
}

/// [`drawn_outlines`] of `art`, the evaluated form of a live object or an object with effects, with
/// each of its objects taken from `left`.
fn evaluated_outlines(doc: &Document, symbols: &SymbolBoxes, art: &Node, left: &mut usize, out: &mut Vec<BezPath>) -> Option<()> {
    *left = left.checked_sub(art.children().map_or(0, Vec::len))?;
    drawn_outlines(doc, symbols, art, left, out)
}

/// Adds `path` to `out` with its points taken from `left`; `None` when they do not fit in it.
fn push_outline(path: Option<BezPath>, left: &mut usize, out: &mut Vec<BezPath>) -> Option<()> {
    if let Some(path) = path {
        *left = left.checked_sub(path.elements().iter().filter(|el| !matches!(el, PathEl::ClosePath)).count())?;
        out.push(path);
    }
    Some(())
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
    cull_bounds_in(symbols, n, true, false)
}

/// [`cull_bounds`] with room for the brush art of the members of the groups and layers in `n`, each
/// member measured as [`crate::brush_fx::cull_bounds`] measures it. The room for `n`'s own brushed
/// strokes is left to the caller.
pub(crate) fn brushed_cull_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    cull_bounds_in(symbols, n, true, true)
}

/// [`cull_bounds_in`] of `c`, a member of a group or layer; with `brush`, grown by the room its
/// brushed strokes leave for brush art, as [`crate::brush_fx::cull_bounds`] measures it.
fn member_cull_bounds(symbols: &SymbolBoxes, c: &Node, after_reshape: bool, brush: bool) -> Option<Rect> {
    if !brush {
        return cull_bounds_in(symbols, c, after_reshape, false);
    }
    if let Some(b) = painted_cull_bounds(symbols, c) {
        return b;
    }
    let b = cull_bounds_in(symbols, c, after_reshape, true)?;
    let pad = crate::brush_fx::pad(symbols, c);
    Some(b.inflate(pad, pad))
}

/// [`cull_bounds`] of path or compound path `n` made from `visual`, its visual bounds as
/// [`Node::visual_bounds_with`] measures them, when [`cull_bounds`] measures `n` by those (it has
/// no crop marks, Vector Halftone, container appearance or effects); else `None`.
pub(crate) fn path_cull_bounds(symbols: &SymbolBoxes, n: &Node, visual: impl FnOnce() -> Option<Rect>) -> Option<Option<Rect>> {
    let plain = matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })
        && !effects::has_crop_marks(n)
        && !effects::addons::halftone::has_halftone(n)
        && !effects::has_container_appearance(n)
        && !has_fx(n)
        && !has_object_fx(n);
    plain.then(|| {
        let b = visual()?;
        let a = live_content_reach(symbols, n);
        Some(b.inflate(a, a))
    })
}

/// [`cull_bounds`] of `n`. With `after_reshape` true, the effects in the art of a symbol instance
/// with geometry effects of its own are measured on the art as those effects reshape it
/// ([`reshaped_art_effects_reach`]). With `after_reshape` false, they are measured on the art
/// before those effects run ([`art_effects_reach`]), which `symbols` keeps for each symbol and
/// linear part. With `brush` true, the members of a group or layer are measured with the room
/// their brushed strokes leave for brush art ([`member_cull_bounds`]).
fn cull_bounds_in(symbols: &SymbolBoxes, n: &Node, after_reshape: bool, brush: bool) -> Option<Rect> {
    let inner = |c: &Node| cull_bounds_in(symbols, c, after_reshape, false);
    let member = |c: &Node| member_cull_bounds(symbols, c, after_reshape, brush);
    // The art that a group's or layer's crop marks or Vector Halftone draw holds its members. The
    // caller leaves the room for an object's own brushed strokes.
    let held = |art: &Node| cull_bounds_in(symbols, art, after_reshape, brush && is_container(n));
    match &n.kind {
        // The object and its crop marks.
        _ if effects::has_crop_marks(n) => effects::crop_marks_art(n).and_then(|art| held(&art)),
        // The object Vector Halftone samples, and as far as its dots reach past it.
        _ if effects::addons::halftone::has_halftone(n) => {
            let reach = effects::addons::halftone::reach(n);
            held(&effects::addons::halftone::split(n).0).map(|b| b.inflate(reach, reach))
        }
        // A 3D effect's art, as the renderer draws it, and the raster effects' reach: the solid of
        // type or of a group's members reaches past the letters or members the arms below measure.
        _ if (effects::has_revolve(n) || effects::has_solid(n))
            && let Some(b) = three_d_bounds(symbols, n) =>
        {
            b.map(|b| with_own_raster_reach(symbols, n, b))
        }
        // The evaluated art (its fills, strokes and geometry effects) and the raster effects' reach.
        _ if effects::has_container_appearance(n) => {
            let art = effects::evaluate_container(Some(symbols.document()), n);
            let art = art.as_ref().unwrap_or(n);
            let b = if art.clips() {
                art.visual_bounds_with(symbols)
            } else {
                art.children()?.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, member(c)))
            }?;
            Some(with_own_raster_reach(symbols, n, b))
        }
        NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => {
            children.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, member(c)))
        }
        _ if has_fx(n) => visual_bounds(symbols, n),
        // The reshaped art (a symbol instance's reshaped art is made from its symbol's art; type's
        // outlines, and an image and the paths of its own fills and strokes, measured one by one
        // with the geometry effects of those fills and strokes), grown by how far the effects in a
        // symbol instance's art paint past that art.
        _ if has_object_fx(n) => {
            let symbol = match &n.kind {
                NodeKind::SymbolInstance { symbol, .. } => {
                    symbols.document().symbols.iter().find(|s| s.name == *symbol).map(|s| dynamic_art(&s.art, n))
                }
                _ => None,
            };
            let reshaped = effects::reshape(Some(symbols.document()), n, symbol.as_ref());
            let b = match &reshaped {
                Some(r) if has_instance_row_effects(n) => instance_piece_bounds(symbols, r),
                Some(r) if matches!(n.kind, NodeKind::Text(_)) || (matches!(n.kind, NodeKind::Image(_)) && !r.clips()) => {
                    r.children().into_iter().flatten().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, inner(c)))
                }
                Some(r) => r.visual_bounds_with(symbols),
                None => None,
            }
            .or_else(|| own_box(symbols, n))?;
            let a = match &reshaped {
                Some(r) if after_reshape => reshaped_art_effects_reach(symbols, n, r),
                _ => art_effects_reach(symbols, n),
            };
            let a = a.max(live_content_reach(symbols, n));
            let b = b.inflate(a, a);
            Some(with_own_raster_reach(symbols, n, b))
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
                // The geometry effects in a clipping path move or grow the region it clips to
                // ([`effects::clip_outline`]) past its plain shape, and its stroke paints along it.
                NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => match children.first() {
                    Some(c) if effects::needs_bake(c) => {
                        let s = c.appearance.outset();
                        let region = effects::clip_outline(c).map(|(bp, _)| bp.bounding_box().inflate(s, s));
                        let painted = if has_fx(c) { visual_bounds(symbols, c) } else { None };
                        vectorcraft_geom::union_opt(vectorcraft_geom::union_opt(Some(b), region), painted).unwrap_or(b)
                    }
                    _ => b,
                },
                _ => b,
            };
            let a = live_content_reach(symbols, n);
            Some(b.inflate(a, a))
        }
    }
}

/// The bounds of the art a 3D effect of `n` draws (Revolve, Extrude & Bevel, Inflate, Rotate),
/// built now, without `n`'s own raster effects ([`three_d_art_bounds`]): `None` when it draws no
/// such art, `Some(None)` when that art paints nothing. The renderer measures the art it keeps
/// instead ([`crate::Renderer::bounds_of`]).
fn three_d_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Option<Rect>> {
    if !effects::has_revolve(n) && !effects::has_solid(n) {
        return None;
    }
    let doc = Some(symbols.document());
    let art = effects::revolve_art_in(doc, n).or_else(|| effects::solid_art_in(doc, n))?;
    Some(three_d_art_bounds(symbols, &art))
}

/// The bounds of `art`, the art a 3D effect draws for an object, as the renderer draws it: with
/// its geometry effects, without the raster effects the object paints over it (which
/// [`with_own_raster_reach`] adds), and each object in it with the raster effects of its own fills
/// and strokes and the art of its brushed strokes (a stroke without a 3D effect of its own keeps
/// its brush): the art itself, which Rasterize and exports take, not the room culling leaves.
pub(crate) fn three_d_art_bounds(symbols: &SymbolBoxes, art: &Node) -> Option<Rect> {
    fn pieces(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
        match &n.kind {
            NodeKind::Group { children, clip: false } if !has_object_fx(n) => {
                children.iter().filter(|c| c.visible).fold(None, |acc, c| vectorcraft_geom::union_opt(acc, pieces(symbols, c)))
            }
            // The art its brushed strokes draw, not the room culling leaves for it.
            _ => {
                let brushed = vectorcraft_brush::has_brush(n)
                    .then(|| vectorcraft_brush::expand(symbols.document(), n))
                    .flatten()
                    .and_then(|art| art.visual_bounds_with(symbols));
                vectorcraft_geom::union_opt(cull_bounds(symbols, n), brushed)
            }
        }
    }
    let mut art = art.clone();
    art.appearance.effects.retain(|e| effects::is_geometry(&e.id));
    let art = effects::evaluate_container(Some(symbols.document()), &art).unwrap_or(art);
    pieces(symbols, &art)
}

/// `b`, the bounds of what `n` draws, grown by how far `n`'s own raster effects paint past it.
pub(crate) fn with_own_raster_reach(symbols: &SymbolBoxes, n: &Node, b: Rect) -> Rect {
    let o = outset(symbols, &n.appearance.effects, b);
    b.inflate(o, o)
}

/// `n`'s own visual box, type's grown to hold its glyphs ([`glyphs_box`]): what it draws when its
/// object effects leave it as it is (Revolve builds no art from type).
fn own_box(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    let b = n.visual_bounds_with(symbols)?;
    Some(match &n.kind {
        NodeKind::Text(t) => vectorcraft_geom::union_opt(Some(b), glyphs_box(t, n.appearance.outset())).unwrap_or(b),
        _ => b,
    })
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
/// art ([`art_effects_reach`]), how far brush art can paint ([`crate::brush_fx::pad`]) and how far
/// a path's own effects paint past it ([`path_effects_reach`]), on the farthest side; for a path
/// whose live-effect plug-ins paint their pieces, how far its pieces and their brush art reach past
/// it ([`painted_cull_bounds`]). 0 for any other kind of object.
fn live_content_reach(symbols: &SymbolBoxes, n: &Node) -> f64 {
    if !vectorcraft_doc::live::is_live(n) {
        return 0.0;
    }
    let mut reach = 0.0f64;
    n.walk(&mut |c| {
        let own = match painted_cull_bounds(symbols, c) {
            Some(b) => match (b, c.visual_bounds_with(symbols)) {
                (Some(b), Some(v)) => [v.x0 - b.x0, v.y0 - b.y0, b.x1 - v.x1, b.y1 - v.y1].into_iter().fold(0.0, f64::max),
                _ => 0.0,
            },
            None => art_effects_reach(symbols, c).max(path_effects_reach(symbols, c)) + crate::brush_fx::pad(symbols, c),
        };
        reach = reach.max(own);
    });
    reach
}

/// How far the effects of path or compound path `n` (a drop shadow, a glow, a geometry effect)
/// paint past its visual bounds, on its farthest side ([`visual_bounds`]): 0 when it has no
/// visible effects, and for any other kind of object.
fn path_effects_reach(symbols: &SymbolBoxes, n: &Node) -> f64 {
    if !has_fx(n) {
        return 0.0;
    }
    match (visual_bounds(symbols, n), n.visual_bounds_with(symbols)) {
        (Some(c), Some(v)) => [v.x0 - c.x0, v.y0 - c.y0, c.x1 - v.x1, c.y1 - v.y1].into_iter().fold(0.0, f64::max),
        _ => 0.0,
    }
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
    let art = r.children().into_iter().flatten().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, cull_bounds_in(symbols, c, false, false)));
    match (art, r.visual_bounds_with(symbols)) {
        (Some(c), Some(v)) => [v.x0 - c.x0, v.y0 - c.y0, c.x1 - v.x1, c.y1 - v.y1].into_iter().fold(0.0, f64::max),
        _ => 0.0,
    }
}
