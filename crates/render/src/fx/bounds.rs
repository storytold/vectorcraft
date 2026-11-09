use super::predicates::{has_fx, has_object_fx, is_container};
use super::raster::doc_raster_ppi;
use super::{
    Affine, AppearanceItem, Arc, BezPath, Document, Effect, FillRule, GeomContext, Node, NodeKind, PathEl, Rect, Renderer, Shape, SymbolBoxes,
    effects,
};

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

/// Visual bounds including geometry effects, stroke outsets and shadows/glows (symbol instances
/// measured by their art from `symbols`).
pub(crate) fn visual_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    if let Some(art) = effects::revolve_art(n).or_else(|| effects::solid_art(n)) {
        return cull_bounds(symbols, &art).map(|b| {
            let o = outset(symbols, &n.appearance.effects, b);
            b.inflate(o, o)
        });
    }
    if let Some(art) = painted_art(symbols, n) {
        return art.visual_bounds_with(symbols).map(|r| with_raster_reach(symbols, n, r));
    }
    let bp = node_bezpath(n)?;
    let ctx = GeomContext { doc: Some(symbols.document()), ..GeomContext::of(n) };
    let g = effected_path(symbols.document(), n, &bp);
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
    r.map(|r| with_raster_reach(symbols, n, r.inflate(s, s)))
}

/// `r`, the art of `n` with its strokes, grown by how far the raster effects of `n`'s fills and
/// strokes and then `n`'s own paint past it.
fn with_raster_reach(symbols: &SymbolBoxes, n: &Node, r: Rect) -> Rect {
    let r = n.appearance.items.iter().filter(|i| paints(i)).map(|i| spread(symbols, item_effects(i), r)).fold(r, |acc, b| acc.union(b));
    let o = outset(symbols, &n.appearance.effects, r);
    r.inflate(o, o)
}

/// The art of `n` with each piece a live-effect plug-in returns and the paints of that piece
/// ([`effects::has_painted_effects`]); `None` for any other object.
fn painted_art(symbols: &SymbolBoxes, n: &Node) -> Option<Node> {
    effects::has_painted_effects(n).then(|| effects::reshape(Some(symbols.document()), n, None)).flatten()
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
    Some(items.map(|i| spread(symbols, item_effects(i), item_bounds(symbols.document(), n, i).unwrap_or(b))).fold(b, |acc, r| acc.union(r)))
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
        // A group's or layer's 3D effect: the art of its members' solids.
        NodeKind::Layer { .. } | NodeKind::Group { clip: false, .. }
            if effects::has_solid(n)
                && let Some(art) = effects::solid_art(n) =>
        {
            drawn_bounds(symbols, &art)
        }
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
            if let Some(art) = painted_art(symbols, n) {
                return art.visual_bounds_with(symbols);
            }
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
    cull_bounds_in(symbols, n, true)
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
/// linear part.
fn cull_bounds_in(symbols: &SymbolBoxes, n: &Node, after_reshape: bool) -> Option<Rect> {
    let inner = |c: &Node| cull_bounds_in(symbols, c, after_reshape);
    match &n.kind {
        // The object and its crop marks.
        _ if effects::has_crop_marks(n) => effects::crop_marks_art(n).and_then(|art| inner(&art)),
        // The object Vector Halftone samples, and as far as its dots reach past it.
        _ if effects::addons::halftone::has_halftone(n) => {
            let reach = effects::addons::halftone::reach(n);
            inner(&effects::addons::halftone::split(n).0).map(|b| b.inflate(reach, reach))
        }
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
